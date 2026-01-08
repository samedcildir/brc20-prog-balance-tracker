--- Historical balances ---

CREATE TABLE IF NOT EXISTS brc20_prog_historical_balances (
    id serial8 PRIMARY KEY, 
    block_height int4 NOT NULL, 
    wallet TEXT NOT NULL, 
    contract_address TEXT NOT NULL, 
    ticker TEXT NOT NULL, 
    amount numeric(80) NOT NULL, 
    is_brc20 BOOLEAN NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_brc20_prog_historical_balances_block_height ON brc20_prog_historical_balances (block_height);
CREATE INDEX IF NOT EXISTS idx_brc20_prog_historical_balances_ticker ON brc20_prog_historical_balances (ticker, is_brc20);
CREATE INDEX IF NOT EXISTS idx_brc20_prog_historical_balances_contract_address ON brc20_prog_historical_balances (contract_address);
CREATE INDEX IF NOT EXISTS idx_brc20_prog_historical_balances_wallet ON brc20_prog_historical_balances (wallet);
CREATE UNIQUE INDEX IF NOT EXISTS idx_brc20_prog_historical_balances_wallet_contract_address_block_height ON brc20_prog_historical_balances (wallet, contract_address, block_height);

--- Current balances ---

CREATE TABLE IF NOT EXISTS brc20_prog_current_balances (
    id serial8 PRIMARY KEY, 
    wallet TEXT NOT NULL, 
    contract_address TEXT NOT NULL, 
    ticker TEXT NOT NULL, 
    amount numeric(80) NOT NULL, 
    is_brc20 BOOLEAN NOT NULL, 
    block_height int4 NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_brc20_prog_current_balances_ticker ON brc20_prog_current_balances (ticker, is_brc20);
CREATE INDEX IF NOT EXISTS idx_brc20_prog_current_balances_contract_address ON brc20_prog_current_balances (contract_address);
CREATE INDEX IF NOT EXISTS idx_brc20_prog_current_balances_wallet ON brc20_prog_current_balances (wallet);
CREATE INDEX IF NOT EXISTS idx_brc20_prog_current_balances_block_height ON brc20_prog_current_balances (block_height);
CREATE UNIQUE INDEX IF NOT EXISTS idx_brc20_prog_current_balances_wallet_contract_address ON brc20_prog_current_balances (wallet, contract_address);

--- Block hashes ---

CREATE TABLE IF NOT EXISTS brc20_prog_block_hashes (id serial8 PRIMARY KEY, block_height int4 NOT NULL, block_hash TEXT NOT NULL);

CREATE INDEX IF NOT EXISTS idx_brc20_prog_block_hashes_block_height ON brc20_prog_block_hashes (block_height);

--- brc20_prog_tickers ---

CREATE TABLE IF NOT EXISTS brc20_prog_tickers (
    id serial8 PRIMARY KEY, 
    ticker TEXT NOT NULL, 
    ticker_hash TEXT NULL, 
    contract_address TEXT NOT NULL, 
    is_brc20 BOOLEAN NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_brc20_prog_tickers_ticker ON brc20_prog_tickers (ticker);
CREATE INDEX IF NOT EXISTS idx_brc20_prog_tickers_ticker_hash ON brc20_prog_tickers (ticker_hash);
CREATE UNIQUE INDEX IF NOT EXISTS idx_brc20_prog_tickers_contract_address ON brc20_prog_tickers (contract_address);
