use std::{error::Error, str::FromStr};

use alloy_primitives::{Address, Bytes, FixedBytes, U256};
use alloy_sol_macro::sol;
use alloy_sol_types::{SolCall, SolEvent};
use brc20_prog::{
    Brc20ProgApiClient,
    types::{EthCall, GetLogsFilter, RawBytes},
};
use jsonrpsee::http_client::HttpClient;

use crate::database::BalanceDatabase;

sol! {
    /**
     * @dev Emitted when a ticker is deposited the first time
     */
    event BRC20Created(bytes indexed ticker, address indexed contract_address);

    /**
     * @dev Emitted when `value` tokens are moved from one account (`from`) to
     * another (`to`).
     *
     * Note that `value` may be zero.
     */
    event Transfer(address indexed from, address indexed to, uint256 value);

    /**
     * @dev Returns the balance of a specific account.
     */
    function balanceOf(bytes calldata ticker, address account) public view virtual returns (uint256) {
        return _brc20s[ticker].balanceOf(account);
    }

    /**
     * @dev Returns the name of the token.
     */
    function name() public view virtual returns (string memory);
}

static CONTROLLER_ADDR: &str = "0xc54dd4581af2dbf18e4d90840226756e9d2b3cdb";

pub enum TestStatus {
    Passed,
    NeedsRetry,
}

pub struct BalanceTracker {
    database: BalanceDatabase,
    client: HttpClient,
}

pub struct TickerInfo {
    pub ticker: String,
}

impl BalanceTracker {
    pub fn new(database: BalanceDatabase, client: HttpClient) -> Self {
        BalanceTracker { database, client }
    }

    pub async fn get_ticker_info_by_address(
        &self,
        address: String,
    ) -> Option<TickerInfo> {
        let call = EthCall {
            from: Some(Address::ZERO.into()),
            to: Some(Address::from_str(&address).unwrap().into()),
            data: Some(RawBytes::new(format!(
                "0x{}",
                hex::encode(nameCall::new(()).abi_encode())
            ))),
        };
        let name_data = self.client.eth_call(call, None).await.ok()?;
        let ticker_name = nameCall::abi_decode_returns(
            hex::decode(name_data.trim_start_matches("0x"))
                .ok()?
                .as_slice(),
        )
        .ok()?;

        Some(TickerInfo {
            ticker: ticker_name,
        })
    }

    pub async fn run(&self) {
        self.database.init().await;
        self.database.clear_residue().await;
        loop {
            match self.check_reorg().await {
                Ok(_) => {}
                Err(err) => {
                    eprintln!("Error checking for reorg: {}", err);
                    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                    continue;
                }
            };

            let next_block: u64 = self.database.get_next_block().await.into();
            println!("Processing block {}", next_block);

            let Ok(prog_block) = self
                .client
                .eth_get_block_by_number(next_block.to_string(), Some(false))
                .await
            else {
                println!("Failed to fetch latest block, retrying...");
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                continue;
            };

            if next_block > prog_block.number.into() {
                println!("Waiting for new blocks...");
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }

            let Ok(mut logs) = self
                .client
                .eth_get_logs(GetLogsFilter {
                    from_block: Some(format!("0x{:x}", next_block)),
                    to_block: Some(format!("0x{:x}", next_block)),
                    address: None,
                    topics: None,
                })
                .await
            else {
                println!("Failed to fetch logs, retrying...");
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                continue;
            };

            logs.sort_by(|a, b| {
                a.transaction_index
                    .cmp(&b.transaction_index)
                    .then(a.log_index.cmp(&b.log_index))
            });

            for log in logs {
                let address_string = log.address.address.to_string().to_lowercase();
                if address_string == CONTROLLER_ADDR {
                    if log.topics[0].bytes == BRC20Created::SIGNATURE_HASH {
                        // Handle BRC20Created event, add ticker to database
                        let call = EthCall {
                            from: Some(Address::ZERO.into()),
                            to: Some(address_from_topic(log.topics[2].bytes).into()),
                            data: Some(RawBytes::new(format!(
                                "0x{}",
                                hex::encode(nameCall::new(()).abi_encode())
                            ))),
                        };
                        let ticker_name = nameCall::abi_decode_returns(
                            hex::decode(
                                self.client
                                    .eth_call(call, None)
                                    .await
                                    .expect("Failed to call name function")
                                    .trim_start_matches("0x"),
                            )
                            .expect("Failed to decode hex")
                            .as_slice(),
                        )
                        .unwrap();

                        println!(
                            "New ticker created: {} at address {}",
                            ticker_name,
                            address_from_topic(log.topics[2].bytes)
                        );

                        self.database
                            .add_ticker(
                                ticker_name,
                                Some(log.topics[1].bytes.to_string()),
                                address_from_topic(log.topics[2].bytes)
                                    .to_string()
                                    .to_lowercase(),
                                true,
                            )
                            .await;
                        continue;
                    }
                } else {
                    if log.topics[0].bytes == Transfer::SIGNATURE_HASH {
                        let (ticker_name, is_brc20) = match self.database.get_ticker_by_address(address_string.clone()).await {
                            Some((ticker_name, is_brc20)) => (ticker_name, is_brc20),
                            None => match self.get_ticker_info_by_address(address_string.clone()).await {
                                Some(ticker_info) => {
                                    self.database
                                        .add_ticker(ticker_info.ticker.clone(), None, address_string.clone(), false)
                                        .await;
                                    (ticker_info.ticker, false)
                                }
                                None => {
                                    println!("Unknown contract address: {}, skipping log", address_string);
                                    continue;
                                }
                            },
                        };

                        let from_address = address_from_topic(log.topics[1].bytes)
                            .to_string()
                            .to_lowercase();
                        let to_address = address_from_topic(log.topics[2].bytes)
                            .to_string()
                            .to_lowercase();
                        let amount = if log.topics.len() > 3 {
                            // this is for ERC-721 and ERC-1155 transfers, skip it
                            continue;
                        } else {
                            amount_from_data(log.data.bytes)
                        };

                        if amount == 0 {
                            continue;
                        }

                        if from_address == "0x0000000000000000000000000000000000000000" {
                            // Handle transfer from zero address (minting)
                            println!("Mint of {} ${}, is brc20: {} to {}", amount, ticker_name, is_brc20, to_address);
                            let balance = self
                                .database
                                .get_balance_of_contract(to_address.clone(), address_string.clone())
                                .await
                                .unwrap_or(U256::ZERO);
                            self.database
                                .update_balance(
                                    next_block,
                                    to_address,
                                    ticker_name,
                                    address_string,
                                    balance.checked_add(amount).expect("Overflow"),
                                    is_brc20,
                                )
                                .await;
                        } else if to_address == "0x0000000000000000000000000000000000000000" {
                            // Handle transfer to zero address (burning)
                            println!("Burn of {} ${} from {}", amount, ticker_name, from_address);
                            let balance = self
                                .database
                                .get_balance_of_contract(from_address.clone(), address_string.clone())
                                .await
                                .unwrap_or(U256::ZERO);
                            self.database
                                .update_balance(
                                    next_block,
                                    from_address,
                                    ticker_name,
                                    address_string,
                                    balance.checked_sub(amount).expect("Insufficient balance"),
                                    is_brc20,
                                )
                                .await;
                        } else {
                            println!(
                                "Transfer of {} ${} from {} to {}",
                                amount, ticker_name, from_address, to_address
                            );

                            println!("Transaction hash: {:?}", log.transaction_hash);

                            let from_balance = self
                                .database
                                .get_balance_of_contract(from_address.clone(), address_string.clone())
                                .await
                                .unwrap_or(U256::ZERO);

                            let to_balance = self
                                .database
                                .get_balance_of_contract(to_address.clone(), address_string.clone())
                                .await
                                .unwrap_or(U256::ZERO);

                            println!("From balance: {:?}", from_balance);
                            println!("To balance: {:?}", to_balance);

                            self.database
                                .update_balance(
                                    next_block,
                                    from_address,
                                    ticker_name.clone(),
                                    address_string.clone(),
                                    from_balance
                                        .checked_sub(amount)
                                        .expect("Insufficient balance"),
                                    is_brc20,
                                )
                                .await;

                            self.database
                                .update_balance(
                                    next_block,
                                    to_address,
                                    ticker_name.clone(),
                                    address_string.clone(),
                                    to_balance.checked_add(amount).expect("Overflow"),
                                    is_brc20,
                                )
                                .await;
                        }
                    }
                }
            }

            self.database
                .set_block_hash(next_block.try_into().unwrap(), prog_block.hash.bytes.to_string())
                .await;
        }
    }

    /// Returns the last confirmed block
    pub async fn check_reorg(&self) -> Result<(), Box<dyn Error>> {
        let last_block = self.database.get_last_block().await;
        for i in 0..10 {
            let block_number = last_block - i;

            let prog_block = self
                .client
                .eth_get_block_by_number(block_number.to_string(), Some(false))
                .await?;

            if self
                .database
                .validate_block_hash(block_number, prog_block.hash.bytes.to_string())
                .await
            {
                if i != 0 {
                    println!("Reorg detected!! Rolling back to block {}", block_number);
                    self.database.reorg(block_number).await;
                    println!("Rollback complete");
                }
                return Ok(());
            }
        }
        panic!("Reorg too deep, cannot recover");
    }

    pub async fn test(&self) -> Result<TestStatus, Box<dyn Error>> {
        let current_block = self.client.eth_block_number().await?;
        let mut count = 1;
        let total = 1000;
        let pairs = self.database.random_wallet_ticker_pairs(total).await;
        let controller_address: Address = CONTROLLER_ADDR.parse().unwrap();
        for (wallet, ticker, amount) in pairs {
            if count % (total / 10) == 0 {
                println!("Testing {}/{}", count, total);
            }
            let ticker_bytes = ticker.clone().into_bytes();
            let call = EthCall {
                from: Some(Address::ZERO.into()),
                to: Some(controller_address.into()),
                data: Some(RawBytes::new(format!(
                    "0x{}",
                    hex::encode(
                        balanceOfCall::new((Bytes::from(ticker_bytes), wallet.parse().unwrap()))
                            .abi_encode()
                    )
                ))),
            };
            let balance = self.client.eth_call(call, None).await?;
            let module_balance = amount_from_data(
                hex::decode(balance.trim_start_matches("0x"))
                    .unwrap()
                    .into(),
            );
            if module_balance != amount {
                println!(
                    "Mismatch for wallet {} ticker {}: db {} on-chain {}",
                    wallet, ticker, amount, module_balance
                );
                let mut next_block = self.client.eth_block_number().await?;
                let mut indexed_block = self.database.get_last_block().await;
                if next_block == current_block {
                    return Err("Balance mismatch".into());
                }
                println!("Received new block during the test, waiting for database to catch up...");
                while u32::from_str_radix(&next_block.trim_start_matches("0x"), 16).unwrap()
                    != indexed_block
                {
                    println!(
                        "Waiting for database to catch up... current {}, indexed {}",
                        next_block, indexed_block
                    );
                    next_block = self.client.eth_block_number().await?;
                    indexed_block = self.database.get_last_block().await;
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
                return Ok(TestStatus::NeedsRetry);
            }
            count += 1;
        }
        Ok(TestStatus::Passed)
    }
}

fn address_from_topic(bytes: FixedBytes<32>) -> Address {
    Address::from_slice(&bytes.as_slice()[12..32])
}

fn amount_from_data(bytes: Bytes) -> U256 {
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes.to_vec()[0..32]);
    let amount = U256::from_be_bytes(arr);
    amount
}
