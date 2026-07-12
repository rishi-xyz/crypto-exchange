//! # Crypto Exchange Matching Engine
//!
//! A price-time priority order matching engine for a centralized cryptocurrency exchange.
//!
//! This crate provides the core matching logic — it does **not** handle networking,
//! persistence, or API layers. Those live in the Go API gateway and are connected
//! via gRPC (see the `discussions/architecture-v1.md` for the full picture).
//!
//! ## Architecture
//!
//! ```text
//! Clients → Go API Layer → gRPC → This Engine
//!                ↓                     ↓
//!        Redis Streams ← ← ← Redis Pub/Sub (fills)
//! ```
//!
//! ## Module Overview
//!
//! | Module | Purpose |
//! |--------|---------|
//! | [`types`] | Core type aliases (`Price`, `Quantity`, `OrderId`, etc.) and enums (`Side`, `OrderType`, `OrderStatus`, `Asset`) |
//! | [`order`] | The [`Order`](order::Order) struct — represents a single resting or incoming order |
//! | [`trade`] | [`Trade`](trade::Trade) and [`TradeInfo`](trade::TradeInfo) — represent a matched fill between two orders |
//! | [`orderbook`] | [`OrderBook`](orderbook::OrderBook) — the BTreeMap-based price-time priority book for one trading pair |
//! | [`engine`] | [`ExchangeEngine`](engine::ExchangeEngine) trait, [`CoreEngine`](engine::CoreEngine) impl, and [`engine_from_env`](engine::engine_from_env) factory |
//! | [`users`] | [`User`](users::User) — balance management with lock/unlock/fill semantics |
//! | [`level_info`] | Snapshot types for orderbook depth queries |
//! | [`order_modify`] | [`OrderModify`](order_modify::OrderModify) — request type for cancel-replace operations |
//! | [`trading_pair`] | [`TradingPair`](trading_pair::TradingPair) — base/quote asset pair identifier |
//! | [`snowflake`] | [`SnowflakeGenerator`](snowflake::SnowflakeGenerator) — 64-bit time-sortable unique ID generator |
//! | [`wal`] | [`WalEngine`](wal::WalEngine) — WAL-backed engine wrapper for crash recovery |

pub mod types;
pub mod level_info;
pub mod trade;
pub mod order;
pub mod order_modify;
pub mod orderbook;
pub mod trading_pair;
pub mod engine;
pub mod users;
pub mod snowflake;
pub mod logging;
pub mod wal;
