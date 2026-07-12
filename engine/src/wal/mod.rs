//! Write-ahead log for crash recovery.
//!
//! Every engine mutation is written to a local file as a newline-delimited JSON line
//! ([JSONL](https://jsonlines.org/)) **before** the mutation is applied. On startup,
//! the WAL is replayed to reconstruct engine state, then truncated.
//!
//! # Format
//!
//! Each line is a serialized [`WalEntry`]:
//!
//! ```text
//! {"sequence":1,"operation":{"AddTradingPair":{"pair":{"base":"ETH","quote":"USDC"}}},"timestamp":1700000000000000000}
//! {"sequence":2,"operation":{"AddUser":{"user":{...}}},"timestamp":1700000000000001000}
//! ```
//!
//! # Lifecycle
//!
//! ```text
//! Engine::new_with_wal(path)
//!   ├─ Wal::replay(path)        → read all entries
//!   ├─ replay each entry        → reconstruct state via _inner methods
//!   ├─ Wal::open(path)          → open fresh WAL
//!   ├─ wal.truncate()           → clear old entries
//!   └─ engine.wal = Some(wal)   → ready for new writes
//!
//! engine.add_order(...)
//!   ├─ wal.append(PlaceOrder)   → write-ahead
//!   └─ add_order_inner(...)     → mutate state
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
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::{
    order::Order,
    trading_pair::TradingPair,
    types::{Asset, OrderId, Quantity, UserId},
    users::User,
};

/// All mutating operations that can be written to the WAL.
///
/// Each variant corresponds to exactly one public method on [`Engine`](crate::matching_engine::Engine).
/// The operation is written to disk **before** the mutation is applied, ensuring
/// durability even if the process crashes mid-mutation.
///
/// # Replay
///
/// During startup, [`Wal::replay`] reads these entries and the engine re-executes
/// each one via the corresponding `_inner` method.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WalOperation {
    /// Credits a user's balance. Corresponds to [`Engine::deposit`](crate::matching_engine::Engine::deposit).
    Deposit {
        user_id: UserId,
        asset: Asset,
        amount: Quantity,
    },
    /// Places a new order into the matching engine. Corresponds to [`Engine::add_order`](crate::matching_engine::Engine::add_order).
    ///
    /// Includes the [`TradingPair`] because during replay the order hasn't been
    /// inserted into the book yet, so the pair can't be inferred from book state.
    PlaceOrder {
        pair: TradingPair,
        order: Order,
    },
    /// Cancels a resting order. Corresponds to [`Engine::cancel_order`](crate::matching_engine::Engine::cancel_order).
    CancelOrder {
        pair: TradingPair,
        order_id: OrderId,
    },
    /// Modifies an order via cancel-replace. Corresponds to [`Engine::modify_order`](crate::matching_engine::Engine::modify_order).
    ///
    /// Written as a **single** entry (not separate cancel + place) to avoid
    /// replay ordering issues. During replay, the engine cancels the old order
    /// and places the new one.
    ModifyOrder {
        pair: TradingPair,
        old_order_id: OrderId,
        new_order: Order,
    },
    /// Adds a new trading pair to the engine. Corresponds to [`Engine::add_trading_pair`](crate::matching_engine::Engine::add_trading_pair).
    AddTradingPair {
        pair: TradingPair,
    },
    /// Registers a new user. Corresponds to [`Engine::add_user`](crate::matching_engine::Engine::add_user).
    AddUser {
        user: User,
    },
}

/// A single entry in the write-ahead log.
///
/// Serialized as one JSON line per entry. Entries are ordered by [`sequence`]
/// (monotonically increasing) and timestamped at write time.
///
/// [`sequence`]: WalEntry::sequence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalEntry {
    /// Monotonically increasing sequence number. Used for ordering and crash detection.
    pub sequence: u64,
    /// The mutation to replay.
    pub operation: WalOperation,
    /// Nanosecond epoch timestamp of when the entry was written.
    pub timestamp: u64,
}

/// Append-only write-ahead log backed by a JSONL file.
///
/// Writes are buffered via [`BufWriter`] and explicitly `flush()`ed after each
/// entry to guarantee durability. The WAL tracks a [`sequence`] counter that
/// resumes from the last entry on open.
///
/// [`sequence`]: Wal::sequence
///
/// # Usage
///
/// ```text
/// let mut wal = Wal::open(Path::new("engine.wal"))?;
/// wal.append(WalOperation::Deposit { ... })?;
/// // Mutation applied after WAL write succeeds
/// ```
#[derive(Debug)]
pub struct Wal {
    /// Buffered file writer — entries are flushed after each append.
    file: BufWriter<File>,
    /// Path to the WAL file on disk, used for truncation and re-opening.
    path: std::path::PathBuf,
    /// Current sequence counter — incremented on each append, resumed from file on open.
    sequence: u64,
}

impl Wal {
    /// Opens or creates a WAL file for appending.
    ///
    /// Scans existing entries to resume the sequence counter.
    pub fn open(path: &Path) -> Result<Self, String> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| format!("Failed to open WAL file: {}", e))?;

        // Scan existing entries to find the max sequence
        let max_seq = Self::scan_sequence(path)?;

        info!(path = %path.display(), max_sequence = max_seq, "WAL opened");

        Ok(Wal {
            file: BufWriter::new(file),
            path: path.to_path_buf(),
            sequence: max_seq,
        })
    }

    /// Reads all entries from the WAL file and returns them in order.
    ///
    /// Skips incomplete/corrupt lines (from crash during write).
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
