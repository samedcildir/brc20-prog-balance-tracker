use rust_embed::Embed;
use sqlx::{Row, PgPool};
use alloy_primitives::U256;

#[derive(Embed)]
#[folder = "sql"]
struct Sql;

pub struct BalanceDatabase {
    db: PgPool,
    first_block: i64,
}

impl BalanceDatabase {
    pub async fn new(db_url: &str, first_block: i64) -> Self {
        BalanceDatabase {
            db: PgPool::connect(db_url).await.unwrap(),
            first_block,
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
        sqlx::query(&init_query).execute(&self.db).await.unwrap();
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

    pub async fn get_balance_of_contract(&self, wallet: String, contract_address: String) -> Option<U256> {
        let row = sqlx::query(
            "SELECT amount FROM brc20_prog_current_balances WHERE wallet = ? AND contract_address = ?",
        )
        .bind(wallet)
        .bind(contract_address)
        .fetch_optional(&self.db)
        .await
        .unwrap();
        row.map(|r| r.get::<String, _>("amount").parse::<U256>().unwrap_or(U256::ZERO))
    }

    pub async fn update_balance(
        &self,
        block_height: u64,
        wallet: String,
        ticker: String,
        contract_address: String,
        amount: U256,
        is_brc20: bool,
    ) {
        let mut tx = self.db.begin().await.unwrap();
        sqlx::query("INSERT INTO brc20_prog_current_balances (wallet, ticker, amount, block_height, contract_address, is_brc20) VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT (wallet, contract_address) DO UPDATE SET amount = excluded.amount, block_height = excluded.block_height")
            .bind(wallet.clone())
            .bind(ticker.clone())
            .bind(amount.to_string())
            .bind(block_height as i64)
            .bind(contract_address.clone())
            .bind(is_brc20)
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query("INSERT INTO brc20_prog_historical_balances (block_height, wallet, ticker, amount, contract_address, is_brc20) VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT (wallet, contract_address, block_height) DO UPDATE SET amount = excluded.amount")
            .bind(block_height as i64)
            .bind(wallet)
            .bind(ticker)
            .bind(amount.to_string())
            .bind(contract_address)
            .bind(is_brc20)
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }

    pub async fn add_ticker(&self, ticker: String, ticker_hash: Option<String>, contract_address: String, is_brc20: bool) {
        sqlx::query("INSERT INTO brc20_prog_tickers (ticker, ticker_hash, contract_address, is_brc20) VALUES (?, ?, ?, ?)")
            .bind(ticker)
            .bind(ticker_hash)
            .bind(contract_address)
            .bind(is_brc20)
            .execute(&self.db)
            .await
            .unwrap();
    }

    pub async fn get_ticker_by_address(&self, contract_address: String) -> Option<(String, bool)> {
        let row = sqlx::query("SELECT ticker, is_brc20 FROM brc20_prog_tickers WHERE contract_address = ?")
            .bind(contract_address)
            .fetch_optional(&self.db)
            .await
            .unwrap();
        row.map(|r| (r.get::<String, _>("ticker"), r.get::<bool, _>("is_brc20")))
    }

    pub async fn get_last_block(&self) -> u64 {
        let row =
            sqlx::query("SELECT MAX(block_height) as max_height FROM brc20_prog_block_hashes")
                .fetch_one(&self.db)
                .await
                .unwrap();
        (row.get::<Option<i64>, _>("max_height")
            .unwrap_or(self.first_block - 1)) as u64
    }

    pub async fn get_next_block(&self) -> u64 {
        self.get_last_block().await + 1
    }

    pub async fn get_block_hash(&self, block_height: u64) -> Option<String> {
        let row =
            sqlx::query("SELECT block_hash FROM brc20_prog_block_hashes WHERE block_height = ?")
                .bind(block_height as i64)
                .fetch_optional(&self.db)
                .await
                .unwrap();
        row.map(|r| r.get::<String, _>("block_hash"))
    }

    pub async fn set_block_hash(&self, block_height: u64, block_hash: String) {
        sqlx::query("INSERT INTO brc20_prog_block_hashes (block_height, block_hash) VALUES (?, ?)")
            .bind(block_height as i64)
            .bind(block_hash)
            .execute(&self.db)
            .await
            .unwrap();
    }

    pub async fn validate_block_hash(&self, block_height: u64, block_hash: String) -> bool {
        if block_height < self.first_block as u64 {
            return true;
        }
        let stored_hash = self.get_block_hash(block_height).await;
        stored_hash.map_or(false, |h| h == block_hash)
    }

    pub async fn clear_residue(&self) {
        // Reorg deletes all data after the last processed block
        // So it works as a cleanup mechanism
        self.reorg(self.get_last_block().await).await;
    }

    pub async fn random_wallet_ticker_pairs(&self, count: i32) -> Vec<(String, String, U256)> {
        let rows = sqlx::query(
            "SELECT wallet, ticker, amount FROM brc20_prog_current_balances WHERE id IN (SELECT id FROM brc20_prog_current_balances ORDER BY RANDOM() LIMIT ?)",
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

    pub async fn reorg(&self, from_block_height: u64) {
        let mut tx = self.db.begin().await.unwrap();
        let from_block_height = from_block_height as i64;

        sqlx::query("DELETE FROM brc20_prog_block_hashes WHERE block_height > ?")
            .bind(from_block_height)
            .execute(&mut *tx)
            .await
            .unwrap();

        sqlx::query("DELETE FROM brc20_prog_historical_balances WHERE block_height > ?")
            .bind(from_block_height)
            .execute(&mut *tx)
            .await
            .unwrap();

        let deleted_rows = sqlx::query(
            "DELETE from brc20_prog_current_balances WHERE block_height > ? RETURNING wallet, contract_address",
        )
        .bind(from_block_height)
        .fetch_all(&mut *tx)
        .await
        .unwrap();

        for row in deleted_rows {
            let wallet: String = row.get("wallet");
            let contract_address: String = row.get("contract_address");
            // Restore the balance for the deleted row
            if let Some(balance_row) = sqlx::query("SELECT block_height, amount, ticker, is_brc20 FROM brc20_prog_historical_balances WHERE wallet = ? AND contract_address = ? ORDER BY block_height DESC LIMIT 1")
                .bind(wallet.clone())
                .bind(contract_address.clone())
                .fetch_optional(&mut *tx)
                .await
                .unwrap() {
                    let block_height: i64 = balance_row.get("block_height");
                    let amount: String = balance_row.get("amount");
                    let ticker: String = balance_row.get("ticker");
                    let is_brc20: bool = balance_row.get("is_brc20");
                    // Restore the balance for the deleted row
                    sqlx::query("INSERT INTO brc20_prog_current_balances (wallet, ticker, amount, block_height, contract_address, is_brc20) VALUES (?, ?, ?, ?, ?, ?)")
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

        sqlx::query("DELETE FROM brc20_prog_block_hashes WHERE block_height > ?")
            .bind(from_block_height)
            .execute(&mut *tx)
            .await
            .unwrap();

        tx.commit().await.unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_database() {
        std::fs::create_dir_all("tmp").unwrap();
        let test_file = format!("sqlite://tmp/{}.db", uuid::Uuid::new_v4());
        let db = BalanceDatabase::new(&test_file, 0).await;

        db.init().await;

        db.update_balance(1, "wallet1".to_string(), "BRC20".to_string(), "0x1234123412341234123412341234123412341234".to_string(), U256::from(100u64), true)
            .await;
        let balance = db
            .get_balance_of_contract("wallet1".to_string(), "0x1234123412341234123412341234123412341234".to_string())
            .await;
        assert_eq!(balance, Some(U256::from(100u64)));

        db.set_block_hash(1, "hash1".to_string()).await;
        let block_hash = db.get_block_hash(1).await;
        assert_eq!(block_hash, Some("hash1".to_string()));

        db.reorg(1).await;
        let balance_after_reorg = db
            .get_balance_of_contract("wallet1".to_string(), "0x1234123412341234123412341234123412341234".to_string())
            .await;
        assert_eq!(balance_after_reorg, None);
        let block_hash_after_reorg = db.get_block_hash(1).await;
        assert_eq!(block_hash_after_reorg, None);

        std::fs::remove_file(test_file.trim_start_matches("sqlite://")).unwrap();
    }
}
