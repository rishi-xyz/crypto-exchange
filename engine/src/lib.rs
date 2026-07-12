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
//! | [`matching_engine`] | [`Engine`](matching_engine::Engine) — the top-level facade that owns orderbooks, users, and the snowflake ID generator |
//! | [`users`] | [`User`](users::User) — balance management with lock/unlock/fill semantics |
//! | [`level_info`] | Snapshot types for orderbook depth queries |
//! | [`order_modify`] | [`OrderModify`](order_modify::OrderModify) — request type for cancel-replace operations |
//! | [`trading_pair`] | [`TradingPair`](trading_pair::TradingPair) — base/quote asset pair identifier |
//! | [`snowflake`] | [`SnowflakeGenerator`](snowflake::SnowflakeGenerator) — 64-bit time-sortable unique ID generator |
//!
//! ## Order Lifecycle
//!
//! 1. Caller creates an `Order` with a placeholder ID (`0`).
//! 2. [`matching_engine::Engine::add_order`](crate::matching_engine::Engine::add_order) assigns a snowflake ID, locks the user's balance, and sends the order to the [`OrderBook`](crate::orderbook::OrderBook).
//! 3. The orderbook attempts to match against resting orders (price-time priority).
//! 4. Matches produce [`Trade`](crate::trade::Trade)s. The engine stamps real snowflake trade IDs and timestamps.
//! 5. Balances are updated: the aggressor's locked funds are converted, resting orders' locked funds are settled.
//! 6. Unfilled FAK orders are cancelled after the matching pass.
//!
//! ## Key Design Decisions
//!
//! - **Engine assigns IDs** — callers never supply real order/trade IDs.
//! - **Modify = new ID** — cancel-replace retires the old order and creates a new one with a fresh snowflake ID.
//! - **Single-threaded engine** — `Engine` is `!Sync`. Concurrency is handled externally via `tokio::sync::RwLock` in the gRPC layer.
//! - **Snowflake IDs** — 64-bit, time-sortable, globally unique across distributed instances.

pub mod types;
pub mod level_info;
pub mod trade;
pub mod order;
pub mod order_modify;
pub mod orderbook;
pub mod trading_pair;
pub mod matching_engine;
pub mod users;
pub mod snowflake;
pub mod logging;
pub mod wal;