use rust_embed::Embed;
use sqlx::{Row, PgPool};
use alloy_primitives::U256;

#[derive(Embed)]
#[folder = "sql"]
struct Sql;

pub struct BalanceDatabase {
    db: PgPool,
    first_block: i32,
    ticker_cache: std::collections::HashMap<String, (String, bool)>,
}

impl BalanceDatabase {
    pub async fn new(db_url: &str, first_block: i32) -> Self {
        BalanceDatabase {
            db: PgPool::connect(db_url).await.unwrap(),
            first_block,
            ticker_cache: std::collections::HashMap::new(),
        }
    }

    pub async fn init(&self) {
        let init_query = String::from_utf8(
            Sql::get("init.sql")
                .expect("Failed to read init.sql")
                .data
                .to_vec(),
        )
        .expect("Failed to read init.sql");
        sqlx::raw_sql(&init_query).execute(&self.db).await.unwrap();
    }

    pub async fn reset(&self) {
        let reset_query = String::from_utf8(
            Sql::get("reset.sql")
                .expect("Failed to read reset.sql")
                .data
                .to_vec(),
        )
        .expect("Failed to read reset.sql");
        println!("Executing reset query:\n{}", reset_query);
        sqlx::query(&reset_query).execute(&self.db).await.unwrap();
    }

    pub async fn add_balance(
        &self,
        block_height: u64,
        wallet: String,
        ticker: String,
        contract_address: String,
        amount: U256,
        is_brc20: bool,
    ) {
        let mut tx = self.db.begin().await.unwrap();
        let r = sqlx::query("INSERT INTO brc20_prog_current_balances (wallet, ticker, amount, block_height, contract_address, is_brc20) VALUES ($1, $2, $3::numeric, $4, $5, $6) ON CONFLICT (wallet, contract_address) DO UPDATE SET amount = amount + excluded.amount, block_height = excluded.block_height RETURNING amount::text")
            .bind(wallet.clone())
            .bind(ticker.clone())
            .bind(amount.to_string())
            .bind(block_height as i64)
            .bind(contract_address.clone())
            .bind(is_brc20)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        let new_amount: String = r.get("amount");
        sqlx::query("INSERT INTO brc20_prog_historical_balances (block_height, wallet, ticker, amount, contract_address, is_brc20) VALUES ($1, $2, $3, $4::numeric, $5, $6) ON CONFLICT (wallet, contract_address, block_height) DO UPDATE SET amount = excluded.amount")
            .bind(block_height as i64)
            .bind(wallet)
            .bind(ticker)
            .bind(new_amount)
            .bind(contract_address)
            .bind(is_brc20)
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }

    pub async fn remove_balance(
        &self,
        block_height: u64,
        wallet: String,
        ticker: String,
        contract_address: String,
        amount: U256,
        is_brc20: bool,
    ) {
        let mut tx = self.db.begin().await.unwrap();
        let r = sqlx::query("INSERT INTO brc20_prog_current_balances (wallet, ticker, amount, block_height, contract_address, is_brc20) VALUES ($1, $2, -1 * $3::numeric, $4, $5, $6) ON CONFLICT (wallet, contract_address) DO UPDATE SET amount = amount + excluded.amount, block_height = excluded.block_height RETURNING amount::text")
            .bind(wallet.clone())
            .bind(ticker.clone())
            .bind(amount.to_string())
            .bind(block_height as i64)
            .bind(contract_address.clone())
            .bind(is_brc20)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        let new_amount: String = r.get("amount");
        if new_amount.starts_with("-") {
            panic!("Insufficient balance for wallet {} on contract {}", wallet, contract_address);
        }
        sqlx::query("INSERT INTO brc20_prog_historical_balances (block_height, wallet, ticker, amount, contract_address, is_brc20) VALUES ($1, $2, $3, $4::numeric, $5, $6) ON CONFLICT (wallet, contract_address, block_height) DO UPDATE SET amount = excluded.amount")
            .bind(block_height as i64)
            .bind(wallet)
            .bind(ticker)
            .bind(new_amount)
            .bind(contract_address)
            .bind(is_brc20)
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }

    pub async fn add_ticker(&mut self, ticker: String, ticker_hash: Option<String>, contract_address: String, is_brc20: bool) {
        sqlx::query("INSERT INTO brc20_prog_tickers (ticker, ticker_hash, contract_address, is_brc20) VALUES ($1, $2, $3, $4) ON CONFLICT (contract_address) DO UPDATE SET ticker = excluded.ticker, ticker_hash = excluded.ticker_hash, is_brc20 = excluded.is_brc20")
            .bind(ticker.clone())
            .bind(ticker_hash)
            .bind(contract_address.clone())
            .bind(is_brc20)
            .execute(&self.db)
            .await
            .unwrap();
        
        // Add to cache
        self.ticker_cache.insert(contract_address, (ticker, is_brc20));
    }

    pub async fn get_ticker_by_address(&mut self, contract_address: String) -> Option<(String, bool)> {
        if let Some(cached) = self.ticker_cache.get(&contract_address) {
            return Some(cached.clone());
        }
        let row = sqlx::query("SELECT ticker, is_brc20 FROM brc20_prog_tickers WHERE contract_address = $1")
            .bind(contract_address.clone())
            .fetch_optional(&self.db)
            .await
            .unwrap();
        if let Some(r) = &row {
            let result = (r.get::<String, _>("ticker"), r.get::<bool, _>("is_brc20"));
            self.ticker_cache.insert(contract_address, result.clone());
            return Some(result);
        }
        None
    }

    pub async fn get_last_block(&self) -> u32 {
        let row =
            sqlx::query("SELECT MAX(block_height) as max_height FROM brc20_prog_block_hashes")
                .fetch_one(&self.db)
                .await
                .unwrap();
        (row.get::<Option<i32>, _>("max_height")
            .unwrap_or(self.first_block - 1)) as u32
    }

    pub async fn get_next_block(&self) -> u32 {
        self.get_last_block().await + 1
    }

    pub async fn get_block_hash(&self, block_height: u32) -> Option<String> {
        let row =
            sqlx::query("SELECT block_hash FROM brc20_prog_block_hashes WHERE block_height = $1")
                .bind(block_height as i32)
                .fetch_optional(&self.db)
                .await
                .unwrap();
        row.map(|r| r.get::<String, _>("block_hash"))
    }

    pub async fn set_block_hash(&self, block_height: u32, block_hash: String) {
        sqlx::query("INSERT INTO brc20_prog_block_hashes (block_height, block_hash) VALUES ($1, $2)")
            .bind(block_height as i32)
            .bind(block_hash)
            .execute(&self.db)
            .await
            .unwrap();
    }

    pub async fn validate_block_hash(&self, block_height: u32, block_hash: String) -> bool {
        if block_height < self.first_block as u32 {
            return true;
        }
        let stored_hash = self.get_block_hash(block_height).await;
        stored_hash.map_or(false, |h| h == block_hash)
    }

    pub async fn clear_residue(&mut self) {
        // Reorg deletes all data after the last processed block
        // So it works as a cleanup mechanism
        self.reorg(self.get_last_block().await).await;
    }

    pub async fn random_wallet_ticker_pairs(&self, count: i32) -> Vec<(String, String, U256)> {
        let rows = sqlx::query(
            "SELECT wallet, ticker, amount::text FROM brc20_prog_current_balances WHERE id IN (SELECT id FROM brc20_prog_current_balances ORDER BY RANDOM() LIMIT $1)",
        )
        .bind(count)
        .fetch_all(&self.db)
        .await
        .unwrap();
        rows.into_iter()
            .map(|r| {
                (
                    r.get("wallet"),
                    r.get("ticker"),
                    r.get::<String, _>("amount")
                        .parse::<U256>()
                        .expect("Failed to parse amount"),
                )
            })
            .collect()
    }

    pub async fn reorg(&mut self, from_block_height: u32) {
        // invalidate ticker cache
        self.ticker_cache.clear();

        let mut tx = self.db.begin().await.unwrap();
        let from_block_height = from_block_height as i32;

        sqlx::query("DELETE FROM brc20_prog_block_hashes WHERE block_height > $1")
            .bind(from_block_height)
            .execute(&mut *tx)
            .await
            .unwrap();

        sqlx::query("DELETE FROM brc20_prog_historical_balances WHERE block_height > $1")
            .bind(from_block_height)
            .execute(&mut *tx)
            .await
            .unwrap();

        let deleted_rows = sqlx::query(
            "DELETE from brc20_prog_current_balances WHERE block_height > $1 RETURNING wallet, contract_address",
        )
        .bind(from_block_height)
        .fetch_all(&mut *tx)
        .await
        .unwrap();

        for row in deleted_rows {
            let wallet: String = row.get("wallet");
            let contract_address: String = row.get("contract_address");
            // Restore the balance for the deleted row
            if let Some(balance_row) = sqlx::query("SELECT block_height, amount::text, ticker, is_brc20 FROM brc20_prog_historical_balances WHERE wallet = $1 AND contract_address = $2 ORDER BY block_height DESC LIMIT 1")
                .bind(wallet.clone())
                .bind(contract_address.clone())
                .fetch_optional(&mut *tx)
                .await
                .unwrap() {
                    let block_height: i32 = balance_row.get("block_height");
                    let amount: String = balance_row.get("amount");
                    let ticker: String = balance_row.get("ticker");
                    let is_brc20: bool = balance_row.get("is_brc20");
                    // Restore the balance for the deleted row
                    sqlx::query("INSERT INTO brc20_prog_current_balances (wallet, ticker, amount, block_height, contract_address, is_brc20) VALUES ($1, $2, $3::numeric, $4, $5, $6)")
                        .bind(wallet)
                        .bind(ticker)
                        .bind(amount)
                        .bind(block_height)
                        .bind(contract_address)
                        .bind(is_brc20)
                        .execute(&mut *tx)
                        .await
                        .unwrap();
                }
        }

        sqlx::query("DELETE FROM brc20_prog_block_hashes WHERE block_height > $1")
            .bind(from_block_height)
            .execute(&mut *tx)
            .await
            .unwrap();

        tx.commit().await.unwrap();
    }
}
