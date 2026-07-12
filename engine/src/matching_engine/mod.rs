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
//! # WAL (Write-Ahead Log)
//!
//! When constructed via [`Engine::new_with_wal`], every mutation is written to a
//! WAL file **before** being applied. On restart, the WAL is replayed to reconstruct
//! state. The WAL uses synchronous, blocking I/O with `flush()` to guarantee durability.
//!
//! Public methods (`add_order`, `cancel_order`, etc.) write WAL entries and then
//! delegate to private `_inner` methods that contain the core logic. The
//! `modify_order` method writes a single `ModifyOrder` entry and calls the `_inner`
//! versions directly, avoiding redundant WAL writes.

use std::collections::HashMap;
use std::path::Path;
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
    wal::{Wal, WalOperation},
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
    /// Write-ahead log for crash recovery. None = no WAL (tests, dev).
    wal: Option<Wal>,
    /// When true, `add_order_inner` uses the order's existing ID instead of generating a new one.
    /// Set to true during WAL replay.
    replay_mode: bool,
}

impl Engine {
    /// Creates a new engine with no WAL (dev/test mode).
    pub fn new() -> Self {
        info!("Engine initialized (no WAL)");
        Engine {
            orderbooks: HashMap::new(),
            users: HashMap::new(),
            id_generator: SnowflakeGenerator::new(1, 1),
            wal: None,
            replay_mode: false,
        }
    }

    /// Creates an engine with WAL-backed crash recovery.
    ///
    /// 1. Replays existing WAL entries to reconstruct state
    /// 2. Truncates the WAL (fresh start)
    /// 3. Opens the WAL for new append-only writes
    pub fn new_with_wal(path: &Path) -> Self {
        let entries = match Wal::replay(path) {
            Ok(entries) => entries,
            Err(e) => {
                error!(error = %e, "WAL replay failed — starting fresh");
                Vec::new()
            }
        };

        let mut engine = Engine {
            orderbooks: HashMap::new(),
            users: HashMap::new(),
            id_generator: SnowflakeGenerator::new(1, 1),
            wal: None,
            replay_mode: true,
        };

        let mut replayed = 0u64;
        for entry in &entries {
            match engine.replay_entry(entry) {
                Ok(()) => replayed += 1,
                Err(e) => {
                    warn!(
                        sequence = entry.sequence,
                        error = %e,
                        "WAL entry replay failed — skipping"
                    );
                }
            }
        }
        engine.replay_mode = false;

        // Truncate old WAL, open fresh for appending
        let mut wal = Wal::open(path).expect("Failed to open WAL after replay");
        let _ = wal.truncate();
        engine.wal = Some(wal);

        info!(replayed, "Engine initialized with WAL recovery");
        engine
    }

    /// Replays a single WAL entry during startup.
    fn replay_entry(&mut self, entry: &crate::wal::WalEntry) -> Result<(), String> {
        match &entry.operation {
            WalOperation::AddTradingPair { pair } => {
                self.add_trading_pair_inner(*pair);
                Ok(())
            }
            WalOperation::AddUser { user } => {
                self.add_user_inner(user.clone());
                Ok(())
            }
            WalOperation::Deposit { user_id, asset, amount } => {
                self.deposit_inner(*user_id, *asset, *amount)
            }
            WalOperation::PlaceOrder { pair, order } => {
                let user_id = order.get_user_id();
                let order_ptr: OrderPointer = Arc::new(Mutex::new(*order));
                self.add_order_inner(user_id, pair, order_ptr)?;
                Ok(())
            }
            WalOperation::CancelOrder { pair, order_id } => {
                self.cancel_order_inner(pair, order_id);
                Ok(())
            }
            WalOperation::ModifyOrder { pair, old_order_id, new_order } => {
                self.cancel_order_inner(pair, old_order_id);
                let user_id = new_order.get_user_id();
                let order_ptr: OrderPointer = Arc::new(Mutex::new(*new_order));
                self.add_order_inner(user_id, pair, order_ptr)?;
                Ok(())
            }
        }
    }

    // =========================================================================
    // Public methods — write WAL then delegate to _inner
    // =========================================================================

    /// Registers a new trading pair.
    #[instrument(skip(self))]
    pub fn add_trading_pair(&mut self, pair: TradingPair) {
        if let Some(ref mut wal) = self.wal {
            let _ = wal.append(WalOperation::AddTradingPair { pair });
        }
        self.add_trading_pair_inner(pair);
    }

    /// Removes a trading pair and its entire orderbook.
    #[instrument(skip(self))]
    pub fn remove_trading_pair(&mut self, pair: &TradingPair) -> Option<OrderBook> {
        let removed = self.orderbooks.remove(pair);
        info!(pair = %pair, removed = removed.is_some(), "Trading pair removed");
        removed
    }

    /// Registers a new user in the engine.
    #[instrument(skip(self, user), fields(user_id = %user.get_user_id()))]
    pub fn add_user(&mut self, user: User) {
        if let Some(ref mut wal) = self.wal {
            let _ = wal.append(WalOperation::AddUser { user: user.clone() });
        }
        self.add_user_inner(user);
    }

    /// Removes a user from the engine.
    #[instrument(skip(self))]
    pub fn remove_user(&mut self, user_id: &UserId) -> Option<User> {
        let removed = self.users.remove(user_id);
        info!(user_id = %user_id, removed = removed.is_some(), "User removed");
        removed
    }

    /// Credits a user's balance for the given asset.
    #[instrument(skip(self), fields(user_id = %user_id))]
    pub fn deposit(
        &mut self,
        user_id: UserId,
        asset: Asset,
        amount: Quantity,
    ) -> Result<(), String> {
        if let Some(ref mut wal) = self.wal {
            wal.append(WalOperation::Deposit { user_id, asset, amount })?;
        }
        self.deposit_inner(user_id, asset, amount)
    }

    /// Places a new order into the engine.
    #[instrument(skip(self, order), fields(user_id = %user_id, pair = %pair))]
    pub fn add_order(
        &mut self,
        user_id: UserId,
        pair: &TradingPair,
        order: OrderPointer,
    ) -> Result<Option<AddOrderResult>, String> {
        // Generate ID and stamp it (normal mode only)
        if !self.replay_mode {
            let order_id = self.id_generator.next_id();
            {
                let mut o = order.lock().unwrap();
                o.set_order_id(order_id);
            }
            debug!(order_id, "Assigned snowflake ID");
        }

        // WAL write — snapshot the Order (Copy) before mutation
        if let Some(ref mut wal) = self.wal {
            let order_snapshot = *order.lock().unwrap();
            wal.append(WalOperation::PlaceOrder { pair: *pair, order: order_snapshot })?;
        }

        self.add_order_inner(user_id, pair, order)
    }

    /// Cancels a resting order.
    #[instrument(skip(self))]
    pub fn cancel_order(&mut self, pair: &TradingPair, order_id: &OrderId) -> bool {
        if let Some(ref mut wal) = self.wal {
            let _ = wal.append(WalOperation::CancelOrder { pair: *pair, order_id: *order_id });
        }
        self.cancel_order_inner(pair, order_id)
    }

    /// Modifies an existing order via cancel-replace.
    ///
    /// Writes a single `ModifyOrder` WAL entry, then delegates to `_inner` methods.
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

        // Generate new ID (normal mode only)
        let new_id = if self.replay_mode {
            // During replay, use the ID that will come from add_order_inner
            // For modify, we need to know the new_order's ID — but it's not set yet.
            // In replay mode, add_order_inner uses the order's existing ID.
            // Since we're about to create the order with a placeholder, we need to handle this.
            // Actually, during replay the ModifyOrder entry already has the new_order with its real ID.
            // So we should use that. But we're constructing new_order here...
            // The solution: in replay mode, the modify_order is called from replay_entry
            // which already passes the new_order from the WAL. So we never reach here in replay.
            unreachable!("modify_order should not be called in replay mode — use replay_entry instead")
        } else {
            self.id_generator.next_id()
        };

        let new_order = Arc::new(Mutex::new(Order::new(
            new_id,
            order_type,
            modify_order.get_side(),
            modify_order.get_status(),
            modify_order.get_price(),
            modify_order.get_quantity(),
            user_id,
        )));

        // WAL write — single entry for the whole modify
        if let Some(ref mut wal) = self.wal {
            let new_order_snapshot = *new_order.lock().unwrap();
            let _ = wal.append(WalOperation::ModifyOrder {
                pair: *pair,
                old_order_id: order_id,
                new_order: new_order_snapshot,
            });
        }

        debug!(order_id, "Cancelling old order for modify");
        self.cancel_order_inner(pair, &order_id);

        debug!(old_order_id = order_id, new_order_id = new_id, "Placing new order for modify");
        let result = self.add_order_inner(user_id, pair, new_order).ok()??;

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

    /// Returns a user's balances across all assets.
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

    // =========================================================================
    // Private _inner methods — core logic, no WAL writes
    //
    // Public methods (add_order, cancel_order, etc.) write WAL entries first,
    // then delegate to these _inner methods for the actual state mutation.
    // During WAL replay, these are called directly (bypassing WAL writes).
    // =========================================================================

    /// Core logic for adding a trading pair. Creates an empty orderbook if one doesn't exist.
    fn add_trading_pair_inner(&mut self, pair: TradingPair) {
        self.orderbooks.entry(pair).or_insert(OrderBook::new());
        info!(pair = %pair, "Trading pair added");
    }

    /// Core logic for registering a user. Inserts into the users map.
    fn add_user_inner(&mut self, user: User) {
        let id = user.get_user_id();
        self.users.insert(id, user);
        info!(user_id = %id, "User registered");
    }

    /// Core logic for crediting a user's balance. Returns `Err` if user not found.
    fn deposit_inner(
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

    /// Core logic for placing an order. This is the most complex method in the engine.
    ///
    /// # Flow
    ///
    /// 1. **Read order parameters** — side, price, quantity, order ID
    /// 2. **Lock balance** — buy locks quote asset (`price * qty`), sell locks base asset (`qty`)
    /// 3. **Add to orderbook** — delegates to [`OrderBook::add_order`] which attempts matching
    /// 4. **Stamp trades** — if matched, assigns snowflake trade IDs and nanosecond timestamps
    /// 5. **Settle fills** — updates buyer/seller balances via [`User::apply_fill`]:
    ///    - Buyer: debit locked quote, credit base
    ///    - Seller: debit locked base, credit quote
    /// 6. **Unlock unfilled** — if the incoming order was fully consumed (no longer in book),
    ///    its remaining locked balance is unlocked
    /// 7. **No match** — if no resting orders matched, the order rests in the book
    ///    and locked balance stays locked
    ///
    /// # Replay Mode
    ///
    /// When [`replay_mode`](Engine::replay_mode) is `true`, the order's existing
    /// snowflake ID is used as-is (not regenerated). This ensures replay produces
    /// identical state.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the user is not found, the trading pair doesn't exist,
    /// or the balance lock fails (insufficient funds).
    fn add_order_inner(
        &mut self,
        user_id: UserId,
        pair: &TradingPair,
        order: OrderPointer,
    ) -> Result<Option<AddOrderResult>, String> {
        let order_id = if self.replay_mode {
            // During replay, use the order's existing ID (already set from WAL)
            order.lock().unwrap().get_order_id()
        } else {
            // Normal mode: ID was already set by the public add_order method
            order.lock().unwrap().get_order_id()
        };

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

    /// Core logic for cancelling a resting order.
    ///
    /// Removes the order from the orderbook and unlocks the user's frozen balance.
    /// Returns `true` if the order was found and cancelled, `false` if not found.
    fn cancel_order_inner(&mut self, pair: &TradingPair, order_id: &OrderId) -> bool {
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
}
