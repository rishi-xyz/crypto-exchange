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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WalOperation {
    Deposit {
        user_id: UserId,
        asset: Asset,
        amount: Quantity,
    },
    PlaceOrder {
        pair: TradingPair,
        order: Order,
    },
    CancelOrder {
        pair: TradingPair,
        order_id: OrderId,
    },
    ModifyOrder {
        pair: TradingPair,
        old_order_id: OrderId,
        new_order: Order,
    },
    AddTradingPair {
        pair: TradingPair,
    },
    AddUser {
        user: User,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalEntry {
    pub sequence: u64,
    pub operation: WalOperation,
    pub timestamp: u64,
}

#[derive(Debug)]
pub struct Wal {
    file: BufWriter<File>,
    path: std::path::PathBuf,
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
