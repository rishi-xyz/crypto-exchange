//! Trade and trade-info types — represent a matched fill between two orders.
//!
//! When an incoming order matches a resting order in the book, a [`Trade`] is created.
//! Each [`Trade`] contains two [`TradeInfo`] snapshots: one for the buy side and one
//! for the sell side. The engine stamps the trade with a snowflake ID and timestamp
//! after the orderbook produces it.

use std::{collections::VecDeque, sync::Arc};

use serde::{Deserialize, Serialize};
use tracing::trace;

use crate::types::{OrderId, Price, Quantity, TradeId, UserId};

/// A snapshot of one side of a matched trade.
///
/// Captures the order ID, price, quantity, and user of either the bid or ask
/// side at the moment of the fill. Stored as an immutable `Arc` inside [`Trade`].
///
/// # Examples
///
/// ```ignore
/// let info = TradeInfo::new(order_id, 50000, 10, user_id);
/// assert_eq!(info.get_price(), 50000);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeInfo {
    /// ID of the order that was filled on this side
    order_id: OrderId,
    /// Price at which the fill occurred (the resting order's price)
    price: Price,
    /// Quantity matched in this fill
    quantity: Quantity,
    /// User who owns this order
    user_id: UserId,
}

impl TradeInfo {
    /// Creates a new trade info snapshot.
    ///
    /// # Arguments
    ///
    /// * `order_id` — ID of the filled order
    /// * `price` — Execution price (from the resting order)
    /// * `quantity` — Number of base units matched
    /// * `user_id` — UUID of the order's owner
    pub fn new(
        order_id: OrderId,
        price: Price,
        quantity: Quantity,
        user_id: UserId
    ) -> Self {
        TradeInfo { order_id, price, quantity, user_id }
    }

    /// Returns the ID of the filled order.
    pub fn get_order_id(&self) -> OrderId { self.order_id }

    /// Returns the execution price of this fill.
    pub fn get_price(&self) -> Price { self.price }

    /// Returns the number of base units matched.
    pub fn get_quantity(&self) -> Quantity { self.quantity }

    /// Returns the UUID of the order's owner.
    pub fn get_user_id(&self) -> UserId { self.user_id }
}

/// Thread-safe, immutable pointer to a [`TradeInfo`] snapshot.
type TradeInfoPointer = Arc<TradeInfo>;

/// A single trade (fill) between two opposing orders.
///
/// Created by the [`OrderBook`](crate::orderbook::OrderBook) during matching.
/// Initially constructed with placeholder `trade_id = 0` and `timestamp = 0`;
/// the [`Engine`](crate::matching_engine::Engine) stamps real values via
/// [`set_trade_id`](Trade::set_trade_id) and [`set_timestamp`](Trade::set_timestamp).
///
/// # Fields
///
/// - `bid_trade` — snapshot of the buy order that was filled
/// - `ask_trade` — snapshot of the sell order that was filled
///
/// The trade always records the **resting order's price** as the execution price,
/// regardless of which side was the aggressor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trade {
    /// Snowflake trade ID. Placeholder `0` until stamped by the engine.
    trade_id: TradeId,
    /// Nanosecond epoch timestamp. Placeholder `0` until stamped by the engine.
    timestamp: u64,
    /// Buy side of the trade
    bid_trade: TradeInfoPointer,
    /// Sell side of the trade
    ask_trade: TradeInfoPointer,
}

impl Trade {
    /// Creates a new trade with the given bid and ask info.
    ///
    /// In practice, `trade_id` and `timestamp` are placeholders (`0`) because
    /// the orderbook does not have access to the snowflake generator.
    /// The engine stamps real values after receiving the trade.
    ///
    /// # Arguments
    ///
    /// * `trade_id` — Placeholder (typically `0`). Overwritten by the engine.
    /// * `timestamp` — Placeholder (typically `0`). Overwritten by the engine.
    /// * `bid_trade` — Trade info for the buy side
    /// * `ask_trade` — Trade info for the sell side
    pub fn new(trade_id: TradeId, timestamp: u64, bid_trade: TradeInfoPointer, ask_trade: TradeInfoPointer) 
    ->Self {
        trace!(trade_id, "Trade created");
        Trade { trade_id, timestamp, bid_trade, ask_trade }
    }

    /// Returns the snowflake trade ID.
    pub fn get_trade_id(&self) -> TradeId {
        self.trade_id
    }

    /// Returns the nanosecond epoch timestamp of when this trade occurred.
    pub fn get_timestamp(&self) -> u64 {
        self.timestamp
    }

    /// Overwrites the trade ID. Called by the engine to stamp the real snowflake ID.
    pub fn set_trade_id(&mut self, id: TradeId) {
        trace!(old_id = self.trade_id, new_id = id, "Trade ID stamped");
        self.trade_id = id;
    }

    /// Overwrites the timestamp. Called by the engine to stamp the real fill time.
    pub fn set_timestamp(&mut self, ts: u64) {
        self.timestamp = ts;
    }

    /// Returns the buy-side trade info snapshot.
    pub fn get_bid_trade_info(&self)->TradeInfoPointer {
        return self.bid_trade.clone();
    }

    /// Returns the sell-side trade info snapshot.
    pub fn get_ask_trade_info(&self)->TradeInfoPointer {
        return self.ask_trade.clone();
    }
}

/// A collection of trades produced by a single matching pass.
pub type Trades = VecDeque<Trade>; 
