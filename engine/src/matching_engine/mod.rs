//! The top-level matching engine — owns orderbooks, users, and the ID generator.
//!
//! [`Engine`] is the main facade for the exchange. It coordinates:
//!
//! - **Order placement** — ID generation, balance locking, book insertion, matching, fill settlement
//! - **Order cancellation** — book removal, balance unlocking
//! - **Order modification** — cancel-replace with new snowflake ID
//! - **User management** — creation, deposits, balance queries
//! - **Trading pair management** — adding/removing pairs
//!
//! # Concurrency Model
//!
//! `Engine` is **not** `Sync`. It is designed to be wrapped in a
//! `tokio::sync::RwLock<Engine>` at the gRPC boundary:
//!
//! - Read-only ops (`get_order_info`, `get_user_balance`, `size`) → `engine.read().await`
//! - Mutating ops (`add_order`, `cancel_order`, `modify_order`) → `engine.write().await`
//!
//! This is simple, correct, and sufficient for V1 single-instance deployment.
//!
//! # ID Generation
//!
//! The engine owns a [`SnowflakeGenerator`] and stamps IDs on:
//! - **Orders** — via `set_order_id()` after construction
//! - **Trades** — via `set_trade_id()` and `set_timestamp()` after the book produces them
//!
//! Callers never supply real IDs — the `order_id` passed to `Order::new()` is always `0`.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use tracing::{debug, error, info, instrument, warn};

use crate::{
    level_info::OrderBookLevelInfo,
    order::{Order, OrderPointer},
    order_modify::OrderModify,
    orderbook::OrderBook,
    snowflake::SnowflakeGenerator,
    trade::Trades,
    trading_pair::TradingPair,
    types::{Asset, OrderId, Quantity, Side, UserId},
    users::User,
};

use std::sync::{Arc, Mutex};

/// The result of successfully placing an order that may have produced trades.
///
/// Returned by [`Engine::add_order`], [`Engine::modify_order`], and
/// [`Engine::cancel_order`](Engine::cancel_order) (for modify). Contains
/// the engine-assigned order ID and any trades that occurred.
pub struct AddOrderResult {
    /// The snowflake order ID assigned by the engine
    pub order_id: OrderId,
    /// Trades produced by matching, or `None` if the order didn't match
    pub trades: Option<Trades>,
}

/// The central matching engine — coordinates orderbooks, users, and ID generation.
///
/// # Examples
///
/// ```ignore
/// let mut engine = Engine::new();
/// let pair = TradingPair::new(Asset::ETH, Asset::USDC);
/// engine.add_trading_pair(pair);
/// engine.add_user(User::new(Some(user_id)));
/// engine.deposit(user_id, Asset::USDC, 100_000).unwrap();
///
/// let order = Arc::new(Mutex::new(Order::new(
///     0, OrderType::GoodTillCancel, Side::Buy, OrderStatus::Empty,
///     50000, 10, user_id,
/// )));
/// let result = engine.add_order(user_id, &pair, order).unwrap();
/// ```
#[derive(Debug)]
pub struct Engine {
    /// One orderbook per trading pair (e.g. ETH-USDC, SOL-USDC)
    orderbooks: HashMap<TradingPair, OrderBook>,
    /// User accounts keyed by UUID
    users: HashMap<UserId, User>,
    /// Snowflake ID generator for orders and trades
    id_generator: SnowflakeGenerator,
}

impl Engine {
    /// Creates a new engine with no pairs, no users, and a fresh ID generator.
    ///
    /// The ID generator is initialized with `machine_id=1, datacenter_id=1`.
    /// For multi-instance deployments, you'd pass different `machine_id`/`datacenter_id`
    /// values (not yet implemented).
    pub fn new() -> Self {
        info!("Engine initialized");
        Engine {
            orderbooks: HashMap::new(),
            users: HashMap::new(),
            id_generator: SnowflakeGenerator::new(1, 1),
        }
    }

    /// Registers a new trading pair.
    ///
    /// Creates an empty [`OrderBook`] for the pair. Orders can only be placed
    /// after the pair has been registered.
    #[instrument(skip(self))]
    pub fn add_trading_pair(&mut self, pair: TradingPair) {
        self.orderbooks.entry(pair).or_insert(OrderBook::new());
        info!(pair = %pair, "Trading pair added");
    }

    /// Removes a trading pair and its entire orderbook.
    ///
    /// All resting orders in the book are lost. In production, you'd want
    /// to cancel all orders and unlock their balances first.
    #[instrument(skip(self))]
    pub fn remove_trading_pair(&mut self, pair: &TradingPair) -> Option<OrderBook> {
        let removed = self.orderbooks.remove(pair);
        info!(pair = %pair, removed = removed.is_some(), "Trading pair removed");
        removed
    }

    /// Registers a new user in the engine.
    ///
    /// The user must already have a UUID (from [`User::new`]).
    #[instrument(skip(self, user), fields(user_id = %user.get_user_id()))]
    pub fn add_user(&mut self, user: User) {
        let id = user.get_user_id();
        self.users.insert(id, user);
        info!(user_id = %id, "User registered");
    }

    /// Removes a user from the engine.
    #[instrument(skip(self))]
    pub fn remove_user(&mut self, user_id: &UserId) -> Option<User> {
        let removed = self.users.remove(user_id);
        info!(user_id = %user_id, removed = removed.is_some(), "User removed");
        removed
    }

    /// Credits a user's balance for the given asset.
    ///
    /// Called by the Go API layer after a blockchain deposit is confirmed.
    ///
    /// # Errors
    ///
    /// Returns `Err("User not found")` if the user doesn't exist.
    #[instrument(skip(self), fields(user_id = %user_id))]
    pub fn deposit(
        &mut self,
        user_id: UserId,
        asset: Asset,
        amount: Quantity,
    ) -> Result<(), String> {
        let user = self.users.get_mut(&user_id);
        match user {
            Some(u) => {
                u.add_balance(asset, amount);
                info!(user_id = %user_id, asset = ?asset, amount, "Deposit credited");
                Ok(())
            }
            None => {
                error!(user_id = %user_id, "Deposit failed — user not found");
                Err("User not found".into())
            }
        }
    }

    /// Returns a user's balances across all assets.
    ///
    /// Only includes assets with non-zero balance or locked amount.
    /// Returns `HashMap<Asset, (available, locked)>`.
    pub fn get_user_balance(
        &self,
        user_id: &UserId,
    ) -> Option<HashMap<Asset, (Quantity, Quantity)>> {
        let user = self.users.get(user_id)?;
        let mut result = HashMap::new();
        for asset in [
            Asset::ETH,
            Asset::SOL,
            Asset::BTC,
            Asset::USDC,
            Asset::USDT,
        ] {
            let bal = user.get_balance(&asset);
            let locked = user.get_locked(&asset);
            if bal > 0 || locked > 0 {
                result.insert(asset, (bal, locked));
            }
        }
        Some(result)
    }

    /// Places a new order into the engine.
    ///
    /// This is the main order placement flow:
    ///
    /// 1. **Generate ID** — assigns a snowflake ID via [`set_order_id`](Order::set_order_id)
    /// 2. **Lock balance** — locks the required funds (quote for buys, base for sells)
    /// 3. **Add to book** — inserts into the [`OrderBook`] and attempts matching
    /// 4. **Settle fills** — stamps trade IDs/timestamps, updates user balances
    /// 5. **Cleanup** — unlocks remaining balance for fully-filled or FAK orders
    ///
    /// # Returns
    ///
    /// - `Ok(Some(AddOrderResult))` — order was placed (may include trades)
    /// - `Ok(None)` — order was rejected (duplicate ID or unmatched FAK)
    /// - `Err(msg)` — user not found, pair not found, insufficient balance, or overflow
    #[instrument(skip(self, order), fields(user_id = %user_id, pair = %pair))]
    pub fn add_order(
        &mut self,
        user_id: UserId,
        pair: &TradingPair,
        order: OrderPointer,
    ) -> Result<Option<AddOrderResult>, String> {
        let order_id = self.id_generator.next_id();
        {
            let mut o = order.lock().unwrap();
            o.set_order_id(order_id);
        }
        debug!(order_id, "Assigned snowflake ID");

        let (side, price, quantity) = {
            let o = order.lock().unwrap();
            (
                o.get_side(),
                o.get_price(),
                o.get_initial_quantity(),
            )
        };

        let (lock_asset, lock_amount) = match side {
            Side::Buy => (
                pair.quote,
                (price as u32)
                    .checked_mul(quantity)
                    .ok_or("Lock amount overflow")?,
            ),
            Side::Sell => (pair.base, quantity),
        };

        {
            let user = self.users.get_mut(&user_id).ok_or("User not found")?;
            match user.lock(order_id, lock_asset, lock_amount) {
                Ok(()) => {
                    debug!(order_id, lock_asset = ?lock_asset, lock_amount, "Balance locked");
                }
                Err(e) => {
                    warn!(order_id, error = %e, "Balance lock failed");
                    return Err(e);
                }
            }
        }

        let book = self
            .orderbooks
            .get_mut(pair)
            .ok_or("Trading pair not found")?;

        match book.add_order(order) {
            Some(mut trades) => {
                let incoming_side = side;

                for trade in &mut trades {
                    trade.set_trade_id(self.id_generator.next_id());
                    trade.set_timestamp(
                        SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
                            .as_nanos() as u64,
                    );
                }

                for trade in &trades {
                    let bid = trade.get_bid_trade_info();
                    let ask = trade.get_ask_trade_info();
                    let qty = bid.get_quantity();

                    let quote_amount = match incoming_side {
                        Side::Buy => (ask.get_price() as u32) * qty,
                        Side::Sell => (bid.get_price() as u32) * qty,
                    };

                    if let Some(buyer) = self.users.get_mut(&bid.get_user_id()) {
                        let _ = buyer.apply_fill(
                            bid.get_order_id(),
                            pair.quote,
                            quote_amount,
                            pair.base,
                            qty,
                        );
                    }
                    if let Some(seller) = self.users.get_mut(&ask.get_user_id()) {
                        let _ = seller.apply_fill(
                            ask.get_order_id(),
                            pair.base,
                            qty,
                            pair.quote,
                            quote_amount,
                        );
                    }
                }

                if !self
                    .orderbooks
                    .get(pair)
                    .map_or(false, |b| b.has_order(&order_id))
                {
                    if let Some(user) = self.users.get_mut(&user_id) {
                        let _ = user.unlock_order(&order_id);
                    }
                }

                info!(
                    order_id,
                    trades_count = trades.len(),
                    "Order matched"
                );

                for trade in &trades {
                    let bid = trade.get_bid_trade_info();
                    let ask = trade.get_ask_trade_info();
                    debug!(
                        trade_id = trade.get_trade_id(),
                        bid_order = bid.get_order_id(),
                        ask_order = ask.get_order_id(),
                        price = bid.get_price(),
                        quantity = bid.get_quantity(),
                        "Trade settled"
                    );
                }

                Ok(Some(AddOrderResult { order_id, trades: Some(trades) }))
            }
            None => {
                if let Some(user) = self.users.get_mut(&user_id) {
                    let _ = user.unlock_order(&order_id);
                }
                debug!(order_id, "Order rested in book (no match)");
                Ok(None)
            }
        }
    }

    /// Cancels a resting order.
    ///
    /// Removes the order from the book and unlocks its balance.
    ///
    /// # Returns
    ///
    /// `true` if the order was found and cancelled, `false` otherwise.
    #[instrument(skip(self))]
    pub fn cancel_order(&mut self, pair: &TradingPair, order_id: &OrderId) -> bool {
        let order = match self
            .orderbooks
            .get_mut(pair)
            .and_then(|book| book.cancel_order(order_id))
        {
            Some(order) => order,
            None => {
                debug!(order_id = %order_id, pair = %pair, "Cancel failed — order not found");
                return false;
            }
        };

        let uid = order.get_user_id();
        if let Some(user) = self.users.get_mut(&uid) {
            let result = user.unlock_order(order_id).is_ok();
            info!(order_id = %order_id, pair = %pair, user_id = %uid, cancelled = result, "Order cancelled");
            result
        } else {
            true
        }
    }

    /// Modifies an existing order via cancel-replace.
    ///
    /// The old order is cancelled and a new one is created with a fresh snowflake ID.
    /// The new order goes through the full placement flow (balance lock, matching, etc.).
    #[instrument(skip(self, modify_order), fields(order_id = modify_order.get_order_id()))]
    pub fn modify_order(
        &mut self,
        pair: &TradingPair,
        modify_order: OrderModify,
    ) -> Option<AddOrderResult> {
        let order_id = modify_order.get_order_id();
        let user_id = modify_order.get_user_id();

        let order_type = self
            .orderbooks
            .get(pair)?
            .get_order_type(&order_id)?;

        debug!(order_id, "Cancelling old order for modify");
        self.cancel_order(pair, &order_id);

        let new_id = self.id_generator.next_id();
        let new_order = Arc::new(Mutex::new(Order::new(
            new_id,
            order_type,
            modify_order.get_side(),
            modify_order.get_status(),
            modify_order.get_price(),
            modify_order.get_quantity(),
            user_id,
        )));
        debug!(old_order_id = order_id, new_order_id = new_id, "Placement new order for modify");

        let result = self.add_order(user_id, pair, new_order).ok()??;
        info!(
            old_order_id = order_id,
            new_order_id = result.order_id,
            trades = result.trades.as_ref().map_or(0, |t| t.len()),
            "Order modified"
        );
        Some(result)
    }

    /// Returns a depth snapshot of the order book for the given pair.
    pub fn get_order_info(&self, pair: &TradingPair) -> Option<OrderBookLevelInfo> {
        self.orderbooks
            .get(pair)
            .map(|book: &OrderBook| book.get_order_info())
    }

    /// Returns the total number of resting orders for the given pair.
    pub fn size(&self, pair: &TradingPair) -> Option<usize> {
        self.orderbooks.get(pair).map(|book| book.size())
    }
}
