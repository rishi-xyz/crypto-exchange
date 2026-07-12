//! The [`Order`](crate::order::Order) struct — a single limit order in the exchange.
//!
//! An order represents a intent to buy or sell a specific quantity of an asset
//! at a specific price. Orders live inside an [`OrderBook`](crate::orderbook::OrderBook)
//! and are matched on a price-time priority basis.
//!
//! # Order Lifecycle
//!
//! ```text
//! new() → Engine assigns snowflake ID → enters book → matched (partial/full) → Filled
//!                                        or
//! new() → Engine assigns snowflake ID → enters book → cancelled → removed
//! ```
//!
//! The `order_id` passed to [`Order::new`](crate::order::Order::new) is always a placeholder (`0`).
//! The [`CoreEngine`](crate::engine::CoreEngine) calls [`Order::set_order_id`](crate::order::Order::set_order_id)
//! to stamp a real snowflake ID before the order enters the book.

use std::{collections::VecDeque, sync::{Arc, Mutex}, time::{SystemTime, UNIX_EPOCH}};
use serde::{Deserialize, Serialize};
use tracing::{debug, trace, warn};

use crate::types::{OrderId, OrderStatus, OrderType, Price, Quantity, Side, UserId};

/// A single limit order in the order book.
///
/// Orders are wrapped in `Arc<Mutex<Order>>` ([`OrderPointer`]) so they can be
/// shared between the book's price-level deques and the `orders_map` lookup table.
///
/// # Invariants
///
/// - `remaining_quantity <= initial_quantity` always holds.
/// - `remaining_quantity == 0` implies `status == Filled`.
/// - `timestamp` is set once at construction and never changes.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Order {
    /// Snowflake ID assigned by the engine. Placeholder `0` until [`set_order_id`](Order::set_order_id) is called.
    order_id: OrderId,
    /// How long the order lives (GTC, FAK, etc.)
    order_type: OrderType,
    /// Buy or sell
    side: Side,
    /// Current lifecycle state
    status: OrderStatus,
    /// Limit price in quote units
    price: Price,
    /// Original quantity when the order was created
    initial_quantity: Quantity,
    /// Quantity yet to be matched. Decreases on each fill.
    remaining_quantity: Quantity,
    /// Nanosecond timestamp of when the order was created (epoch)
    timestamp: u64,
    /// UUID of the user who placed this order
    user_id: UserId,
}

impl Order {
    /// Creates a new order.
    ///
    /// The `order_id` parameter is a placeholder — the engine will overwrite it
    /// with a snowflake ID via [`set_order_id`](Order::set_order_id) before the
    /// order enters the book.
    ///
    /// # Arguments
    ///
    /// * `order_id` — Placeholder ID (typically `0`). Overwritten by the engine.
    /// * `order_type` — How long the order lives ([`GoodTillCancel`](OrderType::GoodTillCancel), [`FillAndKill`](OrderType::FillAndKill), etc.)
    /// * `side` — Buy or sell
    /// * `status` — Initial status (typically [`Empty`](OrderStatus::Empty))
    /// * `price` — Limit price in quote units
    /// * `quantity` — Number of base units to trade
    /// * `user_id` — UUID of the placing user
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let order = Order::new(
    ///     0, // placeholder — engine assigns real ID
    ///     OrderType::GoodTillCancel,
    ///     Side::Buy,
    ///     OrderStatus::Empty,
    ///     50000,  // price
    ///     10,     // quantity
    ///     user_id,
    /// );
    /// assert_eq!(order.get_remaining_quantity(), 10);
    /// ```
    pub fn new(
        order_id: OrderId,
        order_type: OrderType,
        side: Side,
        status: OrderStatus,
        price: Price,
        quantity: Quantity,
        user_id: UserId
    ) ->Self {
        let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos() as u64;
        trace!(
            order_id,
            order_type = ?order_type,
            side = ?side,
            price,
            quantity,
            user = %user_id,
            "Order created"
        );
        Self {
            order_id,
            order_type,
            side,
            status,
            price,
            initial_quantity: quantity,
            remaining_quantity: quantity,
            timestamp: ts,
            user_id,
        }
    }
    
    /// Returns the snowflake order ID.
    pub fn get_order_id(&self) ->OrderId {
        return  self.order_id
    }

    /// Returns the order type ([`GoodTillCancel`](OrderType::GoodTillCancel), [`FillAndKill`](OrderType::FillAndKill), etc.)
    pub fn get_type(&self) ->OrderType {
        return  self.order_type
    }

    /// Returns which side of the trade this order is on.
    pub fn get_side(&self) ->Side {
        return  self.side
    }

    /// Returns the current lifecycle status of the order.
    pub fn get_status(&self) ->OrderStatus {
        return  self.status
    }

    /// Returns the limit price in quote units.
    pub fn get_price(&self) ->Price {
        return  self.price
    }

    /// Returns the original quantity when the order was created.
    pub fn get_initial_quantity(&self) ->Quantity {
        return  self.initial_quantity
    }

    /// Returns the quantity yet to be matched.
    ///
    /// Decreases as fills occur. When this reaches `0`, the order is fully filled.
    pub fn get_remaining_quantity(&self) ->Quantity {
        return  self.remaining_quantity
    }

    /// Returns the quantity that has been matched so far.
    ///
    /// Equivalent to `initial_quantity - remaining_quantity`.
    pub fn get_filled_quantity(&self) ->Quantity {
        return self.initial_quantity - self.remaining_quantity
    }

    /// Returns `true` if the order has been fully filled (`remaining_quantity == 0`).
    pub fn is_filled(&self) ->bool {
        return self.get_remaining_quantity() == 0;
    }

    /// Applies a fill to this order, reducing `remaining_quantity`.
    ///
    /// Updates the order's status to [`Filled`](OrderStatus::Filled) or
    /// [`PartiallyFilled`](OrderStatus::PartiallyFilled) accordingly.
    ///
    /// # Arguments
    ///
    /// * `quantity` — Number of units to fill. Must be `<= remaining_quantity`.
    ///
    /// # Errors
    ///
    /// Returns `Err` if `quantity > remaining_quantity` (overfill attempt).
    ///
    /// # Examples
    ///
    /// ```ignore
    /// order.fills(5).unwrap();
    /// assert_eq!(order.get_remaining_quantity(), 5);
    /// assert_eq!(order.get_status(), OrderStatus::PartiallyFilled);
    /// ```
    pub fn fills(&mut self, quantity: Quantity) -> Result<(), String> {
        if quantity > self.get_remaining_quantity() {
            warn!(
                order_id = self.order_id,
                fill_qty = quantity,
                remaining = self.remaining_quantity,
                "Overfill attempt"
            );
            return Err(format!(
                "Order ({}) cannot be filled for more than its remaining quantity",
                self.get_order_id()
            ));
        }
        self.remaining_quantity -= quantity;
        if self.remaining_quantity == 0 {
            self.status = OrderStatus::Filled;
        } else {
            self.status = OrderStatus::PartiallyFilled;
        }
        debug!(
            order_id = self.order_id,
            fill_qty = quantity,
            remaining = self.remaining_quantity,
            status = ?self.status,
            "Fill applied"
        );
        Ok(())
    }

    /// Returns the nanosecond epoch timestamp of when this order was created.
    pub fn get_timestamp(&self) -> u64 {
        self.timestamp
    }

    /// Returns the UUID of the user who placed this order.
    pub fn get_user_id(&self) -> UserId {
        self.user_id
    }

    /// Overwrites the order's ID with the given snowflake ID.
    ///
    /// Called by the [`WalEngine`](crate::wal::WalEngine) after construction
    /// to assign the real engine-generated ID. This replaces the placeholder `0`
    /// that was passed to [`Order::new`](crate::order::Order::new).
    pub fn set_order_id(&mut self, id: OrderId) {
        trace!(old_id = self.order_id, new_id = id, "Order ID stamped");
        self.order_id = id;
    }
}

/// Thread-safe pointer to a single order, shared between the book's price-level
/// deques and the `orders_map` lookup table.
pub type OrderPointer = Arc<Mutex<Order>>;

/// A deque of orders at a single price level, ordered by time of arrival (FIFO).
pub type OrderPointers = VecDeque<OrderPointer>;
