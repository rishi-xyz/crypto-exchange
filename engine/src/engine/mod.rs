//! The matching engine — trait, core implementation, and env-based factory.
//!
//! # Architecture
//!
//! ```text
//! ExchangeEngine (trait)
//!     ├── CoreEngine   — pure business logic, no WAL
//!     └── WalEngine    — wraps CoreEngine, adds WAL + ID generation
//!
//! engine_from_env() → Box<dyn ExchangeEngine>
//!   WAL_ENABLED=true  → WalEngine
//!   otherwise         → CoreEngine
//! ```
//!
//! [`CoreEngine`](crate::engine::CoreEngine) owns orderbooks, users, and the snowflake ID generator.
//! It performs the actual mutations — balance locking, order matching, fill settlement.
//! It does **not** write to the WAL or generate order IDs; those are concerns of
//! the [`WalEngine`](crate::wal::WalEngine) wrapper.
//!
//! # Concurrency Model
//!
//! The engine is **not** `Sync`. It is designed to be wrapped in a
//! `tokio::sync::RwLock` at the gRPC boundary.

use std::collections::HashMap;

use tracing::{debug, error, info, instrument, warn};

use crate::{
    level_info::OrderBookLevelInfo,
    order::{Order, OrderPointer},
    order_modify::OrderModify,
    orderbook::OrderBook,
    snowflake::SnowflakeGenerator,
    trade::Trades,
    trading_pair::TradingPair,
    types::{Asset, OrderId, OrderType, Quantity, Side, UserId},
    users::User,
};

use std::sync::{Arc, Mutex};

// =========================================================================
// Result type
// =========================================================================

/// The result of successfully placing an order that may have produced trades.
pub struct AddOrderResult {
    pub order_id: OrderId,
    pub trades: Option<Trades>,
}

// =========================================================================
// ExchangeEngine trait
// =========================================================================

/// The public API for the matching engine.
///
/// Both [`CoreEngine`] and [`WalEngine`](crate::wal::WalEngine) implement this trait.
/// Callers use the trait to stay decoupled from the specific implementation.
pub trait ExchangeEngine {
    /// Registers a new trading pair. Creates an empty orderbook if one doesn't exist.
    fn add_trading_pair(&mut self, pair: TradingPair);

    /// Removes a trading pair and its entire orderbook.
    fn remove_trading_pair(&mut self, pair: &TradingPair) -> Option<OrderBook>;

    /// Registers a new user in the engine.
    fn add_user(&mut self, user: User);

    /// Removes a user from the engine.
    fn remove_user(&mut self, user_id: &UserId) -> Option<User>;

    /// Credits a user's balance for the given asset.
    fn deposit(&mut self, user_id: UserId, asset: Asset, amount: Quantity) -> Result<(), String>;

    /// Places a new order into the engine.
    ///
    /// The order must already have a valid ID set — callers are responsible
    /// for ID generation (typically via [`WalEngine`](crate::wal::WalEngine)).
    fn add_order(
        &mut self,
        user_id: UserId,
        pair: &TradingPair,
        order: OrderPointer,
    ) -> Result<Option<AddOrderResult>, String>;

    /// Cancels a resting order. Returns `true` if found and cancelled.
    fn cancel_order(&mut self, pair: &TradingPair, order_id: &OrderId) -> bool;

    /// Modifies an existing order via cancel-replace.
    ///
    /// Writes a single WAL entry (if applicable), cancels the old order,
    /// and places a new one with a fresh ID.
    fn modify_order(
        &mut self,
        pair: &TradingPair,
        modify_order: OrderModify,
    ) -> Option<AddOrderResult>;

    /// Returns a depth snapshot of the order book for the given pair.
    fn get_order_info(&self, pair: &TradingPair) -> Option<OrderBookLevelInfo>;

    /// Returns the total number of resting orders for the given pair.
    fn size(&self, pair: &TradingPair) -> Option<usize>;

    /// Returns a user's balances across all assets as `HashMap<Asset, (available, locked)>`.
    fn get_user_balance(
        &self,
        user_id: &UserId,
    ) -> Option<HashMap<Asset, (Quantity, Quantity)>>;
}

// =========================================================================
// CoreEngine — pure business logic
// =========================================================================

/// The core matching engine — coordinates orderbooks, users, and ID generation.
///
/// Contains pure business logic with no WAL awareness. While it includes an
/// internal ID generator, order ID *assignment* is the responsibility of
/// [`WalEngine`](crate::wal::WalEngine) which wraps this struct.
#[derive(Debug)]
pub struct CoreEngine {
    orderbooks: HashMap<TradingPair, OrderBook>,
    users: HashMap<UserId, User>,
    id_generator: SnowflakeGenerator,
}

impl CoreEngine {
    /// Creates a new engine with no WAL (dev/test mode).
    pub fn new() -> Self {
        info!("CoreEngine initialized");
        CoreEngine {
            orderbooks: HashMap::new(),
            users: HashMap::new(),
            id_generator: SnowflakeGenerator::new(1, 1),
        }
    }

    /// Returns the next snowflake ID. Called by [`WalEngine`](crate::wal::WalEngine)
    /// before delegating to CoreEngine methods.
    pub fn next_id(&mut self) -> OrderId {
        self.id_generator.next_id()
    }

    /// Returns the order type for the given order ID, if it exists in the book.
    /// Called by [`WalEngine`](crate::wal::WalEngine) when constructing ModifyOrder entries.
    pub fn get_order_type(&self, pair: &TradingPair, order_id: &OrderId) -> Option<OrderType> {
        self.orderbooks
            .get(pair)
            .and_then(|book| book.get_order_type(order_id))
    }
}

impl Default for CoreEngine {
    fn default() -> Self {
        Self::new()
    }
}

// =========================================================================
// ExchangeEngine impl for CoreEngine
// =========================================================================

impl ExchangeEngine for CoreEngine {
    /// Registers a new trading pair.
    #[instrument(skip(self))]
    fn add_trading_pair(&mut self, pair: TradingPair) {
        self.orderbooks.entry(pair).or_insert(OrderBook::new());
        info!(pair = %pair, "Trading pair added");
    }

    /// Removes a trading pair and its entire orderbook.
    #[instrument(skip(self))]
    fn remove_trading_pair(&mut self, pair: &TradingPair) -> Option<OrderBook> {
        let removed = self.orderbooks.remove(pair);
        info!(pair = %pair, removed = removed.is_some(), "Trading pair removed");
        removed
    }

    /// Registers a new user in the engine.
    #[instrument(skip(self, user), fields(user_id = %user.get_user_id()))]
    fn add_user(&mut self, user: User) {
        let id = user.get_user_id();
        self.users.insert(id, user);
        info!(user_id = %id, "User registered");
    }

    /// Removes a user from the engine.
    #[instrument(skip(self))]
    fn remove_user(&mut self, user_id: &UserId) -> Option<User> {
        let removed = self.users.remove(user_id);
        info!(user_id = %user_id, removed = removed.is_some(), "User removed");
        removed
    }

    /// Credits a user's balance for the given asset.
    #[instrument(skip(self), fields(user_id = %user_id))]
    fn deposit(
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

    /// Places a new order into the engine.
    ///
    /// Expects the order to already have a valid ID set (caller is responsible
    /// for ID generation — typically [`WalEngine`](crate::wal::WalEngine)).
    #[instrument(skip(self, order), fields(user_id = %user_id, pair = %pair))]
    fn add_order(
        &mut self,
        user_id: UserId,
        pair: &TradingPair,
        order: OrderPointer,
    ) -> Result<Option<AddOrderResult>, String> {
        let order_id = order.lock().unwrap().get_order_id();

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
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
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

                Ok(Some(AddOrderResult {
                    order_id,
                    trades: Some(trades),
                }))
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
    #[instrument(skip(self))]
    fn cancel_order(&mut self, pair: &TradingPair, order_id: &OrderId) -> bool {
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
    /// Constructs a new order from the modify request parameters, cancels the
    /// old order, and places the new one.
    #[instrument(skip(self, modify_order), fields(order_id = modify_order.get_order_id()))]
    fn modify_order(
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

        let new_order = Arc::new(Mutex::new(Order::new(
            modify_order.get_order_id(),
            order_type,
            modify_order.get_side(),
            modify_order.get_status(),
            modify_order.get_price(),
            modify_order.get_quantity(),
            user_id,
        )));

        debug!(order_id, "Cancelling old order for modify");
        self.cancel_order(pair, &order_id);

        let new_id = new_order.lock().unwrap().get_order_id();
        debug!(old_order_id = order_id, new_order_id = new_id, "Placing new order for modify");
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
    fn get_order_info(&self, pair: &TradingPair) -> Option<OrderBookLevelInfo> {
        self.orderbooks
            .get(pair)
            .map(|book: &OrderBook| book.get_order_info())
    }

    /// Returns the total number of resting orders for the given pair.
    fn size(&self, pair: &TradingPair) -> Option<usize> {
        self.orderbooks.get(pair).map(|book| book.size())
    }

    /// Returns a user's balances across all assets.
    fn get_user_balance(
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
}

// =========================================================================
// Factory — env-based construction
// =========================================================================

/// Creates an engine based on environment variables.
///
/// - `WAL_ENABLED=true` → [`WalEngine`](crate::wal::WalEngine) with crash recovery
/// - Otherwise → [`CoreEngine`] (no WAL, suitable for dev/test)
///
/// # Environment Variables
///
/// | Variable | Default | Description |
/// |----------|---------|-------------|
/// | `WAL_ENABLED` | `false` | Set to `"true"` to enable WAL |
/// | `WAL_PATH` | `"engine.wal"` | Path to the WAL file |
pub fn engine_from_env() -> Box<dyn ExchangeEngine> {
    match std::env::var("WAL_ENABLED") {
        Ok(ref v) if v == "true" => {
            let path = std::env::var("WAL_PATH").unwrap_or_else(|_| "engine.wal".into());
            info!(path = %path, "Creating WalEngine from env");
            Box::new(crate::wal::WalEngine::new(std::path::Path::new(&path)))
        }
        _ => {
            info!("Creating CoreEngine from env (WAL disabled)");
            Box::new(CoreEngine::new())
        }
    }
}
