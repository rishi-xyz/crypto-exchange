//! Core type aliases and enumerations used across the matching engine.
//!
//! This module defines the vocabulary types that every other module depends on.
//! Type aliases are used instead of newtypes to keep the engine internals simple
//! and avoid wrapping/unwrapping at every boundary.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use std::fmt;

/// Price of an order in the smallest quote-unit (e.g. cents for USDC).
///
/// Using `i32` allows negative prices to be rejected at the type level
/// while keeping arithmetic simple. Max representable price: ~2.1 billion.
pub type Price  = i32;

/// Quantity of an asset being traded, in the smallest base-unit.
///
/// Using `u32` — quantities are always non-negative. Max: ~4.2 billion units.
pub type Quantity = u32;

/// Unique identifier for an order, assigned by the engine via snowflake generation.
///
/// Callers pass `0` as a placeholder in [`Order::new`](crate::order::Order::new);
/// the [`CoreEngine`](crate::engine::CoreEngine) overwrites it with a real snowflake ID
/// before the order enters the book.
pub type OrderId = u64;

/// Unique identifier for a trade (fill), assigned by the engine via snowflake generation.
///
/// Trade IDs follow the same snowflake format as order IDs, ensuring global uniqueness
/// and time-sortability across distributed engine instances.
pub type TradeId = u64;

/// Universally unique identifier for a user account.
///
/// Uses UUID v4 (random). Generated client-side or by the Go API layer;
/// the engine does not create user IDs.
pub type UserId = Uuid;

/// Determines how long an order lives and how it interacts with the book.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum OrderType {
    /// Remains in the book until explicitly cancelled or fully filled.
    /// The default order type for limit orders.
    GoodTillCancel,
    /// Remains in the book until end of the trading day, then auto-cancelled.
    /// (V1: treated identically to GTC — day-end expiry is not yet implemented.)
    GoodForDay,
    /// Immediate-or-cancel: matches as much as possible at the limit price,
    /// then any unfilled remainder is discarded. Nothing rests in the book.
    FillAndKill,
    /// Fill-or-kill: must be filled entirely in one match, or the entire
    /// order is cancelled. (V1: not yet implemented — behaves like FAK.)
    FillOrKill,
}

/// Which side of the trade the order is on.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Side {
    /// Buyer — wants to purchase the base asset with quote currency.
    Buy,
    /// Seller — wants to sell the base asset for quote currency.
    Sell,
}

/// Tracks the lifecycle state of an order.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum OrderStatus {
    /// Order was explicitly cancelled by the user or the engine (e.g. FAK expiry).
    Cancelled,
    /// Some quantity has been matched, but `remaining_quantity > 0`.
    PartiallyFilled,
    /// All quantity has been matched (`remaining_quantity == 0`).
    Filled,
    /// Initial state before any matching. Used as a default when constructing
    /// orders via [`OrderModify::to_order_pointer`](crate::order_modify::OrderModify::to_order_pointer).
    Empty,
}

/// Supported crypto assets in the exchange.
///
/// V1 supports a fixed set. A production system would use a string or
/// database-backed asset registry, but an enum keeps things simple for
/// the matching engine prototype.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Eq, Hash)]
pub enum Asset {
    /// Ethereum
    ETH,
    /// Solana
    SOL,
    /// Bitcoin
    BTC,
    /// USD Coin (stablecoin)
    USDC,
    /// Tether (stablecoin)
    USDT,
}

impl fmt::Display for Asset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Asset::ETH => write!(f, "ETH"),
            Asset::SOL => write!(f, "SOL"),
            Asset::BTC => write!(f, "BTC"),
            Asset::USDC => write!(f, "USDC"),
            Asset::USDT => write!(f, "USDT"),
        }
    }
}
