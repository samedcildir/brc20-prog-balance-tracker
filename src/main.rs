use base64::{Engine, prelude::BASE64_STANDARD};
use dotenvy::dotenv;
use jsonrpsee::http_client::HttpClientBuilder;

use crate::{
    database::BalanceDatabase,
    tracker::{BalanceTracker, TestStatus},
};

mod database;
mod tracker;

pub struct Args {
    db_url: String,
    rpc_url: String,
    rpc_user: String,
    rpc_password: String,
    network: String,
}

fn parse_env() -> Args {
    // Placeholder for argument parsing logic
    // Parse rpc-url, rpc-user, rpc-password and network type (mainnet, signet)
    let db_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://balances.sqlite".into());
    let rpc_url = std::env::var("RPC_URL").unwrap_or_else(|_| "http://localhost:18545".into());
    let rpc_user = std::env::var("RPC_USER").unwrap_or_else(|_| "user".into());
    let rpc_password = std::env::var("RPC_PASSWORD").unwrap_or_else(|_| "password".into());
    let network = std::env::var("NETWORK").unwrap_or_else(|_| "mainnet".into());

    Args {
        rpc_url,
        rpc_user,
        rpc_password,
        network,
        db_url,
    }
}

#[tokio::main]
async fn main() {
    dotenv().ok();
    let env = parse_env();

    println!("Database URL: {}", env.db_url);
    println!("RPC URL: {}", env.rpc_url);
    println!("Network: {}", env.network);

    let first_block = match env.network.as_str() {
        "mainnet" => 912690,
        "signet" => 230000,
        _ => 0,
    };

    if std::env::args().any(|arg| arg == "--reset") {
        let db = BalanceDatabase::new(&env.db_url, first_block).await;
        db.reset().await;
        db.init().await;
        println!("Database reset complete.");
        return;
    }

    let mut tracker = BalanceTracker::new(
        BalanceDatabase::new(&env.db_url, first_block).await,
        HttpClientBuilder::new()
            .set_headers({
                let mut headers = http::HeaderMap::new();
                let auth_value = format!(
                    "Basic {}",
                    BASE64_STANDARD.encode(format!("{}:{}", env.rpc_user, env.rpc_password))
                );
                headers.insert(
                    http::header::AUTHORIZATION,
                    http::HeaderValue::from_str(&auth_value).unwrap(),
                );
                headers
            })
            .max_request_size(u32::MAX)
            .max_response_size(u32::MAX)
            .build(env.rpc_url)
            .unwrap(),
    );

    if std::env::args().any(|arg| arg == "--test") {
        loop {
            match tracker.test().await.expect("Test failed") {
                TestStatus::Passed => {
                    println!("All tests passed!");
                    break;
                }
                TestStatus::NeedsRetry => {
                    println!("Tests need retry, waiting...");

                    // Wait for a while before retrying
                    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                }
            }
        }
    }

    tracker.run().await;
}
