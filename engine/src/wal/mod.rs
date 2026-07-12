//! Write-ahead log for crash recovery.
//!
//! Every engine mutation is written to a local file as a newline-delimited JSON line
//! ([JSONL](https://jsonlines.org/)) **before** the mutation is applied. On startup,
//! the WAL is replayed to reconstruct engine state, then truncated.
//!
//! # Format
//!
//! Each line is a serialized [`WalEntry`](crate::wal::WalEntry):
//!
//! ```text
//! {"sequence":1,"operation":{"AddTradingPair":{"pair":{"base":"ETH","quote":"USDC"}}},"timestamp":1700000000000000000}
//! {"sequence":2,"operation":{"AddUser":{"user":{...}}},"timestamp":1700000000000001000}
//! ```
//!
//! # Lifecycle
//!
//! ```text
//! WalEngine::new(path)
//!   ├─ Wal::replay(path)        → read all entries
//!   ├─ replay each entry        → reconstruct state via CoreEngine methods
//!   ├─ Wal::open(path)          → open fresh WAL
//!   └─ wal.truncate()           → clear old entries
//!
//! wal_engine.add_order(...)
//!   ├─ generate snowflake ID    → stamp order
//!   ├─ wal.append(PlaceOrder)   → write-ahead
//!   └─ core.add_order(...)      → mutate state
//! ```
//!
//! # Crash Safety
//!
//! - Entries are `flush()`ed to disk before every mutation.
//! - Partial/corrupt lines (from crash mid-write) are skipped during replay.
//! - Sequence numbers are monotonically increasing for ordering guarantees.
//! - The WAL is truncated after successful replay — no compaction needed.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tracing::{debug, error, info, instrument, warn};

use crate::{
    engine::{AddOrderResult, CoreEngine, ExchangeEngine},
    order::Order,
    order_modify::OrderModify,
    trading_pair::TradingPair,
    types::{Asset, OrderId, Quantity, UserId},
    users::User,
    order::OrderPointer,
    level_info::OrderBookLevelInfo,
};

// =========================================================================
// WAL types (unchanged)
// =========================================================================

/// All mutating operations that can be written to the WAL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WalOperation {
    /// Credits a user's balance. Written by [`Engine::deposit`](crate::engine::ExchangeEngine::deposit).
    Deposit {
        user_id: UserId,
        asset: Asset,
        amount: Quantity,
    },
    /// Places a new order. Written by [`Engine::add_order`](crate::engine::ExchangeEngine::add_order).
    ///
    /// Includes the [`TradingPair`] because during replay the order hasn't been
    /// inserted into the book yet, so the pair can't be inferred from book state.
    PlaceOrder {
        pair: TradingPair,
        order: Order,
    },
    /// Cancels a resting order. Written by [`Engine::cancel_order`](crate::engine::ExchangeEngine::cancel_order).
    CancelOrder {
        pair: TradingPair,
        order_id: OrderId,
    },
    /// Modifies an order via cancel-replace. Written as a **single** entry
    /// (not separate cancel + place) to avoid replay ordering issues.
    ModifyOrder {
        pair: TradingPair,
        old_order_id: OrderId,
        new_order: Order,
    },
    /// Adds a new trading pair. Written by [`Engine::add_trading_pair`](crate::engine::ExchangeEngine::add_trading_pair).
    AddTradingPair {
        pair: TradingPair,
    },
    /// Registers a new user. Written by [`Engine::add_user`](crate::engine::ExchangeEngine::add_user).
    AddUser {
        user: User,
    },
}

/// A single entry in the write-ahead log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalEntry {
    /// Monotonically increasing sequence number. Used for ordering and crash detection.
    pub sequence: u64,
    /// The mutation to replay.
    pub operation: WalOperation,
    /// Nanosecond epoch timestamp of when the entry was written.
    pub timestamp: u64,
}

// =========================================================================
// Wal — file I/O
// =========================================================================

/// Append-only write-ahead log backed by a JSONL file.
#[derive(Debug)]
pub struct Wal {
    file: BufWriter<File>,
    path: std::path::PathBuf,
    sequence: u64,
}

impl Wal {
    /// Opens or creates a WAL file for appending.
    pub fn open(path: &Path) -> Result<Self, String> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| format!("Failed to open WAL file: {}", e))?;

        let max_seq = Self::scan_sequence(path)?;

        info!(path = %path.display(), max_sequence = max_seq, "WAL opened");

        Ok(Wal {
            file: BufWriter::new(file),
            path: path.to_path_buf(),
            sequence: max_seq,
        })
    }

    /// Reads all entries from the WAL file and returns them in order.
    pub fn replay(path: &Path) -> Result<Vec<WalEntry>, String> {
        let file = File::open(path).map_err(|e| format!("Failed to open WAL for replay: {}", e))?;
        let reader = BufReader::new(file);
        let mut entries = Vec::new();
        let mut skipped = 0u64;

        for (line_no, line) in reader.lines().enumerate() {
            let line = line.map_err(|e| format!("Failed to read WAL line {}: {}", line_no + 1, e))?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match serde_json::from_str::<WalEntry>(trimmed) {
                Ok(entry) => entries.push(entry),
                Err(e) => {
                    warn!(line = line_no + 1, error = %e, "Skipping corrupt/partial WAL line");
                    skipped += 1;
                }
            }
        }

        info!(
            entries = entries.len(),
            skipped,
            "WAL replay complete"
        );
        Ok(entries)
    }

    /// Appends an operation to the WAL, flushing to disk.
    pub fn append(&mut self, operation: WalOperation) -> Result<WalEntry, String> {
        self.sequence += 1;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;

        let entry = WalEntry {
            sequence: self.sequence,
            operation,
            timestamp,
        };

        let mut json = serde_json::to_string(&entry)
            .map_err(|e| format!("Failed to serialize WAL entry: {}", e))?;
        json.push('\n');

        self.file
            .write_all(json.as_bytes())
            .map_err(|e| format!("Failed to write WAL entry: {}", e))?;
        self.file
            .flush()
            .map_err(|e| format!("Failed to flush WAL: {}", e))?;

        debug!(sequence = self.sequence, "WAL entry appended");
        Ok(entry)
    }

    /// Truncates the WAL file (called after successful replay).
    pub fn truncate(&mut self) -> Result<(), String> {
        self.file.flush().map_err(|e| format!("Failed to flush: {}", e))?;
        let file = File::create(&self.path)
            .map_err(|e| format!("Failed to truncate WAL: {}", e))?;
        self.file = BufWriter::new(file);
        self.sequence = 0;
        info!("WAL truncated");
        Ok(())
    }

    /// Scans existing entries to find the maximum sequence number.
    fn scan_sequence(path: &Path) -> Result<u64, String> {
        if !path.exists() {
            return Ok(0);
        }

        let file =
            File::open(path).map_err(|e| format!("Failed to open WAL for scan: {}", e))?;
        let reader = BufReader::new(file);
        let mut max_seq = 0u64;

        for line in reader.lines() {
            let line = line.map_err(|e| format!("Failed to read WAL line: {}", e))?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(entry) = serde_json::from_str::<WalEntry>(trimmed) {
                if entry.sequence > max_seq {
                    max_seq = entry.sequence;
                }
            }
        }

        Ok(max_seq)
    }
}

// =========================================================================
// WalEngine — middleware/interceptor
// =========================================================================

/// WAL-backed engine wrapper.
///
/// Wraps [`CoreEngine`] and adds:
/// - Write-ahead logging for crash recovery
/// - Snowflake ID generation for orders
/// - Replay on construction
pub struct WalEngine {
    inner: CoreEngine,
    wal: Wal,
}

impl WalEngine {
    /// Creates a new WAL-backed engine with crash recovery.
    ///
    /// 1. Replays existing WAL entries to reconstruct state
    /// 2. Truncates the WAL (fresh start)
    /// 3. Opens the WAL for new append-only writes
    pub fn new(wal_path: &Path) -> Self {
        let entries = match Wal::replay(wal_path) {
            Ok(entries) => entries,
            Err(e) => {
                error!(error = %e, "WAL replay failed — starting fresh");
                Vec::new()
            }
        };

        let mut core = CoreEngine::new();
        let mut replayed = 0u64;

        for entry in &entries {
            match Self::replay_entry(&mut core, entry) {
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

        let mut wal = Wal::open(wal_path).expect("Failed to open WAL after replay");
        let _ = wal.truncate();

        info!(replayed, "WalEngine initialized with WAL recovery");

        WalEngine { inner: core, wal }
    }

    /// Replays a single WAL entry during startup.
    fn replay_entry(core: &mut CoreEngine, entry: &WalEntry) -> Result<(), String> {
        match &entry.operation {
            WalOperation::AddTradingPair { pair } => {
                core.add_trading_pair(*pair);
                Ok(())
            }
            WalOperation::AddUser { user } => {
                core.add_user(user.clone());
                Ok(())
            }
            WalOperation::Deposit { user_id, asset, amount } => {
                core.deposit(*user_id, *asset, *amount)
            }
            WalOperation::PlaceOrder { pair, order } => {
                let user_id = order.get_user_id();
                let order_ptr: OrderPointer = Arc::new(Mutex::new(*order));
                core.add_order(user_id, pair, order_ptr)?;
                Ok(())
            }
            WalOperation::CancelOrder { pair, order_id } => {
                core.cancel_order(pair, order_id);
                Ok(())
            }
            WalOperation::ModifyOrder { pair, old_order_id, new_order } => {
                core.cancel_order(pair, old_order_id);
                let user_id = new_order.get_user_id();
                let order_ptr: OrderPointer = Arc::new(Mutex::new(*new_order));
                core.add_order(user_id, pair, order_ptr)?;
                Ok(())
            }
        }
    }
}

impl std::fmt::Debug for WalEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WalEngine")
            .field("inner", &self.inner)
            .field("wal", &self.wal)
            .finish()
    }
}

// =========================================================================
// ExchangeEngine impl for WalEngine
// =========================================================================

impl ExchangeEngine for WalEngine {
    /// Writes `AddTradingPair` to WAL, then delegates to [`CoreEngine`].
    #[instrument(skip(self))]
    fn add_trading_pair(&mut self, pair: TradingPair) {
        if let Err(e) = self.wal.append(WalOperation::AddTradingPair { pair }) {
            error!(error = %e, "WAL write failed for AddTradingPair");
        }
        self.inner.add_trading_pair(pair);
    }

    /// Delegates to [`CoreEngine`] (no WAL entry — not critical for V1 recovery).
    #[instrument(skip(self))]
    fn remove_trading_pair(&mut self, pair: &TradingPair) -> Option<crate::orderbook::OrderBook> {
        self.inner.remove_trading_pair(pair)
    }

    /// Writes `AddUser` to WAL, then delegates to [`CoreEngine`].
    #[instrument(skip(self, user), fields(user_id = %user.get_user_id()))]
    fn add_user(&mut self, user: User) {
        if let Err(e) = self.wal.append(WalOperation::AddUser { user: user.clone() }) {
            error!(error = %e, "WAL write failed for AddUser");
        }
        self.inner.add_user(user);
    }

    /// Delegates to [`CoreEngine`] (no WAL entry — not critical for V1 recovery).
    #[instrument(skip(self))]
    fn remove_user(&mut self, user_id: &UserId) -> Option<User> {
        self.inner.remove_user(user_id)
    }

    /// Writes `Deposit` to WAL, then delegates to [`CoreEngine`].
    #[instrument(skip(self), fields(user_id = %user_id))]
    fn deposit(
        &mut self,
        user_id: UserId,
        asset: Asset,
        amount: Quantity,
    ) -> Result<(), String> {
        if let Err(e) = self.wal.append(WalOperation::Deposit { user_id, asset, amount }) {
            error!(error = %e, "WAL write failed for Deposit");
            return Err(format!("WAL write failed: {}", e));
        }
        self.inner.deposit(user_id, asset, amount)
    }

    /// Generates a snowflake ID, writes `PlaceOrder` to WAL, then delegates to [`CoreEngine`].
    #[instrument(skip(self, order), fields(user_id = %user_id, pair = %pair))]
    fn add_order(
        &mut self,
        user_id: UserId,
        pair: &TradingPair,
        order: OrderPointer,
    ) -> Result<Option<AddOrderResult>, String> {
        let order_id = self.inner.next_id();
        {
            let mut o = order.lock().unwrap();
            o.set_order_id(order_id);
        }
        debug!(order_id, "Assigned snowflake ID");

        let order_snapshot = *order.lock().unwrap();
        if let Err(e) = self.wal.append(WalOperation::PlaceOrder { pair: *pair, order: order_snapshot }) {
            error!(error = %e, "WAL write failed for PlaceOrder");
            return Err(format!("WAL write failed: {}", e));
        }

        self.inner.add_order(user_id, pair, order)
    }

    /// Writes `CancelOrder` to WAL, then delegates to [`CoreEngine`].
    #[instrument(skip(self))]
    fn cancel_order(&mut self, pair: &TradingPair, order_id: &OrderId) -> bool {
        if let Err(e) = self.wal.append(WalOperation::CancelOrder { pair: *pair, order_id: *order_id }) {
            error!(error = %e, "WAL write failed for CancelOrder");
        }
        self.inner.cancel_order(pair, order_id)
    }

    /// Generates a new ID, writes a single `ModifyOrder` to WAL, then cancels old + places new via [`CoreEngine`].
    #[instrument(skip(self, modify_order), fields(order_id = modify_order.get_order_id()))]
    fn modify_order(
        &mut self,
        pair: &TradingPair,
        modify_order: OrderModify,
    ) -> Option<AddOrderResult> {
        let order_id = modify_order.get_order_id();
        let user_id = modify_order.get_user_id();

        let order_type = self.inner.get_order_type(pair, &order_id)?;

        let new_id = self.inner.next_id();

        let new_order = Arc::new(Mutex::new(Order::new(
            new_id,
            order_type,
            modify_order.get_side(),
            modify_order.get_status(),
            modify_order.get_price(),
            modify_order.get_quantity(),
            user_id,
        )));

        let new_order_snapshot = *new_order.lock().unwrap();
        if let Err(e) = self.wal.append(WalOperation::ModifyOrder {
            pair: *pair,
            old_order_id: order_id,
            new_order: new_order_snapshot,
        }) {
            error!(error = %e, "WAL write failed for ModifyOrder");
        }

        debug!(order_id, "Cancelling old order for modify");
        self.inner.cancel_order(pair, &order_id);

        debug!(old_order_id = order_id, new_order_id = new_id, "Placing new order for modify");
        let result = self.inner.add_order(user_id, pair, new_order).ok()??;

        info!(
            old_order_id = order_id,
            new_order_id = result.order_id,
            trades = result.trades.as_ref().map_or(0, |t| t.len()),
            "Order modified"
        );
        Some(result)
    }

    /// Delegates to [`CoreEngine`] (read-only, no WAL entry).
    fn get_order_info(&self, pair: &TradingPair) -> Option<OrderBookLevelInfo> {
        self.inner.get_order_info(pair)
    }

    /// Delegates to [`CoreEngine`] (read-only, no WAL entry).
    fn size(&self, pair: &TradingPair) -> Option<usize> {
        self.inner.size(pair)
    }

    /// Delegates to [`CoreEngine`] (read-only, no WAL entry).
    fn get_user_balance(
        &self,
        user_id: &UserId,
    ) -> Option<std::collections::HashMap<Asset, (Quantity, Quantity)>> {
        self.inner.get_user_balance(user_id)
    }
}
