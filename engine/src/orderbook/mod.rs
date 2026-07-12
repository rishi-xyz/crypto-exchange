//! Price-time priority order book for a single trading pair.
//!
//! The [`OrderBook`] maintains two sides — bids (buy orders) and asks (sell orders) —
//! each organized as a `BTreeMap<Price, VecDeque<Order>>`. This gives us:
//!
//! - **Price priority** — `BTreeMap` keeps levels sorted; best bid is the max key,
//!   best ask is the min key.
//! - **Time priority** — `VecDeque` at each level is FIFO; oldest order matches first.
//!
//! # Matching Algorithm
//!
//! When an incoming order arrives via [`add_order`](OrderBook::add_order):
//!
//! 1. Check if the order can match (price is at or past the best opposite price).
//! 2. Walk the opposite side from the best price inward.
//! 3. At each level, match orders front-to-back (time priority).
//! 4. Fill quantity = `min(incoming_remaining, resting_remaining)`.
//! 5. Continue until the incoming order is fully filled or no more resting orders match.
//! 6. After matching, any unfilled [`FillAndKill`](crate::types::OrderType::FillAndKill)
//!    orders at the front of the book are cancelled.
//!
//! # Thread Safety
//!
//! Individual orders are `Arc<Mutex<Order>>`. The book itself is **not** `Sync` —
//! it is owned by the [`Engine`](crate::matching_engine::Engine) which is wrapped
//! in a `tokio::sync::RwLock` at the gRPC boundary.

use std::{
    cmp::min,
    collections::{BTreeMap, HashMap, VecDeque},
    sync::{Arc, Mutex, MutexGuard},
};

use tracing::{debug, instrument, trace, warn};

use crate::{
    level_info::{LevelInfo, OrderBookLevelInfo},
    order::{Order, OrderPointer, OrderPointers},
    order_modify::OrderModify,
    trade::{Trade, TradeInfo, Trades},
    types::{OrderId, OrderType, Price, Quantity, Side, UserId},
};

/// A price-time priority order book for a single trading pair.
///
/// Contains both bid (buy) and ask (sell) sides, plus a flat lookup table
/// (`orders_map: HashMap<OrderId, Order>`) for O(1) order existence checks.
///
/// # Data Structures
///
/// - `bids_map` — `BTreeMap<Price, VecDeque<Order>>` sorted ascending. Best bid is the **last** key.
/// - `asks_map` — `BTreeMap<Price, VecDeque<Order>>` sorted ascending. Best ask is the **first** key.
/// - `orders_map` — `HashMap<OrderId, Order>` for O(1) lookups by order ID (private).
#[derive(Debug)]
pub struct OrderBook {
    /// Buy orders keyed by price, sorted ascending (best bid = last key)
    bids_map: BTreeMap<Price, OrderPointers>,
    /// Sell orders keyed by price, sorted ascending (best ask = first key)
    asks_map: BTreeMap<Price, OrderPointers>,
    /// Flat lookup of all orders by ID (for existence checks and type lookups)
    orders_map: HashMap<OrderId, Order>,
}

impl OrderBook {
    /// Creates an empty order book.
    pub fn new() -> Self {
        let bids_map: BTreeMap<Price, OrderPointers> = BTreeMap::new();
        let asks_map: BTreeMap<Price, OrderPointers> = BTreeMap::new();
        let orders_map: HashMap<OrderId, Order> = HashMap::new();
        trace!("Empty orderbook created");
        OrderBook {
            bids_map,
            asks_map,
            orders_map,
        }
    }

    /// Checks whether an incoming order can potentially match at the given price.
    ///
    /// - **Buy**: can match if `price >= best_ask`
    /// - **Sell**: can match if `price <= best_bid`
    ///
    /// Returns `false` if the opposite side is empty.
    fn can_match(&self, side: Side, price: Price) -> bool {
        match side {
            Side::Buy => {
                if let Some((&best_ask, _)) = self.asks_map.first_key_value() {
                    price >= best_ask
                } else {
                    false
                }
            }
            Side::Sell => {
                if let Some((&best_bid, _)) = self.bids_map.last_key_value() {
                    price <= best_bid
                } else {
                    false
                }
            }
        }
    }

    /// Matches resting orders against the incoming aggressor order.
    ///
    /// Walks the opposite side from the best price inward, filling at each level.
    /// Returns all trades produced. After matching, any unfilled FAK orders at
    /// the front of the book are cancelled.
    fn match_order(&mut self, _aggressor_user_id: UserId, aggressor_side: Side) -> Trades {
        let mut trades: Trades = Trades::new();
        trades.reserve(self.orders_map.len() / 2);
        let mut self_trade_blocked = false;

        debug!(
            bids_levels = self.bids_map.len(),
            asks_levels = self.asks_map.len(),
            aggressor_side = ?aggressor_side,
            "Starting matching pass"
        );

        loop {
            if self.bids_map.is_empty() || self.asks_map.is_empty() {
                break;
            }
            let bid_price: Price = *self.bids_map.last_key_value().unwrap().0;
            let ask_price: Price = *self.asks_map.first_key_value().unwrap().0;
            if bid_price < ask_price {
                break;
            }
            let bids: &mut VecDeque<Arc<Mutex<Order>>> =
                self.bids_map.get_mut(&bid_price).unwrap();
            let asks: &mut VecDeque<Arc<Mutex<Order>>> =
                self.asks_map.get_mut(&ask_price).unwrap();
            let initial_asks_len = asks.len();
            let initial_bids_len = bids.len();
            let mut skipped = 0;

            debug!(bid_price, ask_price, bids_at_level = bids.len(), asks_at_level = asks.len(), "Matching at price level");

            while bids.len() != 0 && asks.len() != 0 {
                let mut bid: MutexGuard<'_, Order> = bids.front().unwrap().lock().unwrap();
                let mut ask: MutexGuard<'_, Order> = asks.front().unwrap().lock().unwrap();
                if bid.get_user_id() == ask.get_user_id() {
                    let skipped_order_id = bid.get_order_id();
                    let skipped_side = bid.get_side();
                    drop(bid);
                    drop(ask);
                    match aggressor_side {
                        Side::Buy => {
                            let resting_order = asks.pop_front().unwrap();
                            asks.push_back(resting_order);
                            skipped += 1;
                            debug!(
                                order_id = skipped_order_id,
                                side = ?skipped_side,
                                "Self-trade skipped (resting ask re-queued)"
                            );
                            if skipped >= initial_asks_len {
                                self_trade_blocked = true;
                                break;
                            }
                        }
                        Side::Sell => {
                            let resting_order = bids.pop_front().unwrap();
                            bids.push_back(resting_order);
                            skipped += 1;
                            debug!(
                                order_id = skipped_order_id,
                                side = ?skipped_side,
                                "Self-trade skipped (resting bid re-queued)"
                            );
                            if skipped >= initial_bids_len {
                                self_trade_blocked = true;
                                break;
                            }
                        }
                    }
                    continue;
                }
                let quantity: Quantity =
                    min(bid.get_remaining_quantity(), ask.get_remaining_quantity());
                let _ = bid.fills(quantity);
                let _ = ask.fills(quantity);

                let bid_filled: bool = bid.is_filled();
                let ask_filled: bool = ask.is_filled();
                let bid_id: OrderId = bid.get_order_id();
                let ask_id: OrderId = ask.get_order_id();
                let bid_price: Price = bid.get_price();
                let ask_price: Price = ask.get_price();
                let bid_user_id: UserId = bid.get_user_id();
                let ask_user_id: UserId = ask.get_user_id();

                drop(bid);
                drop(ask);

                if bid_filled {
                    bids.pop_front();
                    self.orders_map.remove(&bid_id);
                }
                if ask_filled {
                    asks.pop_front();
                    self.orders_map.remove(&ask_id);
                }

                debug!(
                    bid_id, ask_id, price = bid_price, quantity,
                    "Trade matched"
                );

                trades.push_back(Trade::new(
                    0,
                    0,
                    Arc::new(TradeInfo::new(bid_id, bid_price, quantity, bid_user_id)),
                    Arc::new(TradeInfo::new(ask_id, ask_price, quantity, ask_user_id)),
                ));
            }
            if bids.is_empty() {
                self.bids_map.remove(&bid_price);
            }
            if asks.is_empty() {
                self.asks_map.remove(&ask_price);
            }
            if self_trade_blocked {
                break;
            }
        }

        // Cleanup unfilled FAK orders at best bid
        if !self.bids_map.is_empty() {
            let bid_price: Price = *self.bids_map.last_key_value().unwrap().0;
            let (is_fak, order_id) = {
                let bids = self.bids_map.get_mut(&bid_price).unwrap();
                let order = bids.front().unwrap().lock().unwrap();
                (
                    order.get_type() == OrderType::FillAndKill,
                    order.get_order_id(),
                )
            };
            if is_fak {
                debug!(order_id, side = ?Side::Buy, "FAK order expired — cancelling unfilled bid");
                let _ = self.cancel_order(&order_id);
            }
        }
        // Cleanup unfilled FAK orders at best ask
        if !self.asks_map.is_empty() {
            let ask_price: Price = *self.asks_map.first_key_value().unwrap().0;
            let (is_fak, order_id) = {
                let asks = self.asks_map.get_mut(&ask_price).unwrap();
                let order = asks.front().unwrap().lock().unwrap();
                (
                    order.get_type() == OrderType::FillAndKill,
                    order.get_order_id(),
                )
            };
            if is_fak {
                debug!(order_id, side = ?Side::Sell, "FAK order expired — cancelling unfilled ask");
                let _ = self.cancel_order(&order_id);
            }
        }

        debug!(trades_created = trades.len(), "Matching pass complete");
        trades
    }

    /// Cancels a resting order by ID.
    ///
    /// Removes the order from both the price-level deque and the `orders_map`.
    /// Returns the cancelled order, or `None` if not found.
    #[instrument(skip(self))]
    pub fn cancel_order(&mut self, order_id: &OrderId) -> Option<Order> {
        let order = self.orders_map.remove(order_id)?;
        let price: Price = order.get_price();
        let side: Side = order.get_side();

        let level = match side {
            Side::Buy => self.bids_map.get_mut(&price),
            Side::Sell => self.asks_map.get_mut(&price),
        };
        if let Some(orders) = level {
            orders.retain(|arc| arc.lock().unwrap().get_order_id() != *order_id);
            if orders.is_empty() {
                match side {
                    Side::Buy => {
                        self.bids_map.remove(&price);
                    }
                    Side::Sell => {
                        self.asks_map.remove(&price);
                    }
                };
            }
        }
        debug!(order_id = %order_id, price, side = ?side, "Order removed from book");
        Some(order)
    }

    /// Adds an order to the book and attempts to match it.
    ///
    /// This is the main entry point for order placement. The flow:
    ///
    /// 1. Reject duplicate order IDs
    /// 2. Reject FAK orders that can't match immediately
    /// 3. Insert the order into the `orders_map` and the appropriate price level
    /// 4. Attempt matching via the private `match_order` method
    #[instrument(skip(self, order), fields(order_id = order.lock().unwrap().get_order_id()))]
    pub fn add_order(&mut self, order: OrderPointer) -> Option<Trades> {
        let (order_id, order_type, order_side, order_price, order_user_id) = {
            let order = order.lock().unwrap();
            (
                order.get_order_id(),
                order.get_type(),
                order.get_side(),
                order.get_price(),
                order.get_user_id(),
            )
        };

        if self.orders_map.contains_key(&order_id) {
            warn!(order_id, "Duplicate order ID — rejected");
            return None;
        }
        if order_type == OrderType::FillAndKill && !self.can_match(order_side, order_price) {
            debug!(order_id, side = ?order_side, price = order_price, "FAK order can't match — rejected");
            return None;
        }

        self.orders_map
            .insert(order_id, order.lock().unwrap().clone());
        let level = match order_side {
            Side::Buy => self.bids_map.entry(order_price).or_default(),
            Side::Sell => self.asks_map.entry(order_price).or_default(),
        };
        level.push_back(order);

        debug!(
            order_id,
            side = ?order_side,
            price = order_price,
            user = %order_user_id,
            "Order inserted into book"
        );

        Some(self.match_order(order_user_id, order_side))
    }

    /// Modifies an existing order by cancel-replace.
    ///
    /// Cancels the old order and creates a new one with the parameters from
    /// `order_modify`. The new order then goes through the full matching flow.
    #[instrument(skip(self, order_modify), fields(order_id = order_modify.get_order_id()))]
    pub fn modify_orders(&mut self, order_modify: OrderModify) -> Option<Trades> {
        let order_id = order_modify.get_order_id();
        if !self.orders_map.contains_key(&order_id) {
            return None;
        }
        let order_type = {
            let existing = self.orders_map.get(&order_id).unwrap();
            existing.get_type()
        };
        let new_order = Arc::new(Mutex::new(Order::new(
            order_id,
            order_type,
            order_modify.get_side(),
            order_modify.get_status(),
            order_modify.get_price(),
            order_modify.get_quantity(),
            order_modify.get_user_id(),
        )));
        let _ = self.cancel_order(&order_id);
        self.add_order(new_order)
    }

    /// Returns the total number of resting orders in the book.
    pub fn size(&self) -> usize {
        return self.orders_map.len();
    }

    /// Returns `true` if an order with the given ID exists in the book.
    pub fn has_order(&self, order_id: &OrderId) -> bool {
        self.orders_map.contains_key(order_id)
    }

    /// Returns the order type for the given order ID, if it exists.
    pub fn get_order_type(&self, order_id: &OrderId) -> Option<OrderType> {
        self.orders_map.get(order_id).map(|o| o.get_type())
    }

    /// Returns a depth snapshot of the order book.
    pub fn get_order_info(&self) -> OrderBookLevelInfo {
        let mut bids_info = VecDeque::new();
        let mut ask_info = VecDeque::new();
        for (price, orders) in self.bids_map.iter() {
            let total_qty: Quantity = orders
                .iter()
                .map(|arc| arc.lock().unwrap().get_remaining_quantity())
                .sum();
            bids_info.push_front(LevelInfo::new(*price, total_qty));
        }
        for (price, orders) in self.asks_map.iter() {
            let total_qty: Quantity = orders
                .iter()
                .map(|arc| arc.lock().unwrap().get_remaining_quantity())
                .sum();
            ask_info.push_back(LevelInfo::new(*price, total_qty));
        }
        OrderBookLevelInfo::new(Arc::new(bids_info), Arc::new(ask_info))
    }
}
