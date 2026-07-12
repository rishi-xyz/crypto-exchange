//! Snowflake ID generator — produces 64-bit, time-sortable, globally unique IDs.
//!
//! Used for order IDs and trade IDs. Snowflake IDs encode timestamp, datacenter,
//! machine, and sequence information into a single `u64`, making them:
//!
//! - **Time-sortable** — IDs generated later are always larger
//! - **Globally unique** — no collisions across datacenters or machines
//! - **Compact** — single `u64`, no UUID overhead
//!
//! # Bit Layout (64-bit)
///
/// ```text
/// 0 | 41-bit timestamp_ms | 5-bit datacenter_id | 5-bit machine_id | 12-bit sequence
/// ```
///
/// | Field | Bits | Range | Purpose |
/// |-------|------|-------|---------|
/// | timestamp_ms | 41 | ~69 years from epoch | Milliseconds since custom epoch |
/// | datacenter_id | 5 | 0–31 | Prevents cross-datacenter collisions |
/// | machine_id | 5 | 0–31 | Prevents same-datacenter collisions |
/// | sequence | 12 | 0–4095 | Per-millisecond counter for burst traffic |
///
/// # Clock Regression
///
/// If the system clock moves backwards, the generator **panics** rather than
/// producing duplicate IDs. In production, you'd handle this with a configurable
/// tolerance or NTP-safe fallback. For V1, a panic is acceptable.
///
/// # Sequence Overflow
///
/// If more than 4096 IDs are generated within a single millisecond, the generator
/// spins until the next millisecond. This handles burst traffic without collisions.

use std::time::{SystemTime, UNIX_EPOCH};

use tracing::debug;

/// Custom epoch in milliseconds (Nov 18, 2023 00:00:00 UTC).
/// Timestamps are stored as milliseconds elapsed since this epoch.
const EPOCH: u64 = 1_700_000_000_000;

/// Number of bits allocated to the datacenter ID field.
pub const DATACENTER_BITS: u64 = 5;

/// Number of bits allocated to the machine ID field.
pub const MACHINE_BITS: u64 = 5;

/// Number of bits allocated to the sequence counter.
pub const SEQUENCE_BITS: u64 = 12;

/// Maximum valid datacenter ID (31).
pub const MAX_DATACENTER_ID: u64 = (1 << DATACENTER_BITS) - 1;

/// Maximum valid machine ID (31).
pub const MAX_MACHINE_ID: u64 = (1 << MACHINE_BITS) - 1;

/// Maximum sequence value before rollover (4095).
pub const MAX_SEQUENCE: u64 = (1 << SEQUENCE_BITS) - 1;

/// Bit shift for the machine ID field (same as `SEQUENCE_BITS`).
pub const MACHINE_SHIFT: u64 = SEQUENCE_BITS;

/// Bit shift for the datacenter ID field.
pub const DATACENTER_SHIFT: u64 = SEQUENCE_BITS + MACHINE_BITS;

/// Bit shift for the timestamp field.
pub const TIMESTAMP_SHIFT: u64 = SEQUENCE_BITS + MACHINE_BITS + DATACENTER_BITS;

/// Generates unique 64-bit snowflake IDs.
///
/// Each generator instance is identified by a `(machine_id, datacenter_id)` pair.
/// In a single-instance deployment (V1), both are `1`. In a multi-instance setup,
/// each instance gets a unique pair to prevent ID collisions.
///
/// # Panics
///
/// [`next_id`](SnowflakeGenerator::next_id) panics if the system clock moves backwards.
///
/// # Examples
///
/// ```ignore
/// let mut gen = SnowflakeGenerator::new(1, 1);
/// let id1 = gen.next_id();
/// let id2 = gen.next_id();
/// assert!(id2 > id1);
/// ```
#[derive(Debug)]
pub struct SnowflakeGenerator {
    /// Which machine/pod this generator belongs to (0–31)
    machine_id: u64,
    /// Which datacenter this generator belongs to (0–31)
    datacenter_id: u64,
    /// Per-millisecond counter (0–4095), resets on new millisecond
    sequence: u64,
    /// Last timestamp used, for clock regression detection and sequence management
    last_timestamp: u64,
}

impl SnowflakeGenerator {
    /// Creates a new snowflake generator.
    ///
    /// # Arguments
    ///
    /// * `machine_id` — Unique machine identifier (0–31). Panics if out of range.
    /// * `datacenter_id` — Unique datacenter identifier (0–31). Panics if out of range.
    ///
    /// # Panics
    ///
    /// Panics if `machine_id > 31` or `datacenter_id > 31`.
    pub fn new(machine_id: u64, datacenter_id: u64) -> Self {
        assert!(
            machine_id <= MAX_MACHINE_ID,
            "machine_id must be 0-{}",
            MAX_MACHINE_ID
        );
        assert!(
            datacenter_id <= MAX_DATACENTER_ID,
            "datacenter_id must be 0-{}",
            MAX_DATACENTER_ID
        );
        debug!(machine_id, datacenter_id, "Snowflake generator initialized");
        SnowflakeGenerator {
            machine_id,
            datacenter_id,
            sequence: 0,
            last_timestamp: 0,
        }
    }

    /// Generates the next unique snowflake ID.
    ///
    /// Handles two edge cases:
    /// - **Same millisecond** — increments the sequence counter. If the counter
    ///   overflows ( exceeds 4095), spins until the next millisecond.
    /// - **Clock regression** — panics immediately. A production system might
    ///   use a tolerance window or NTP-safe fallback.
    ///
    /// # Panics
    ///
    /// Panics if the system clock has moved backwards since the last call.
    ///
    /// # Returns
    ///
    /// A 64-bit snowflake ID encoding `(timestamp, datacenter, machine, sequence)`.
    pub fn next_id(&mut self) -> u64 {
        let mut timestamp = Self::current_millis();

        if timestamp < self.last_timestamp {
            panic!(
                "Clock moved backwards. Refusing to generate id for {} milliseconds",
                self.last_timestamp - timestamp
            );
        }

        if timestamp == self.last_timestamp {
            self.sequence = (self.sequence + 1) & MAX_SEQUENCE;
            if self.sequence == 0 {
                // Sequence exhausted in this millisecond, spin until next ms
                while timestamp <= self.last_timestamp {
                    timestamp = Self::current_millis();
                }
            }
        } else {
            self.sequence = 0;
        }

        self.last_timestamp = timestamp;

        ((timestamp - EPOCH) << TIMESTAMP_SHIFT)
            | (self.datacenter_id << DATACENTER_SHIFT)
            | (self.machine_id << MACHINE_SHIFT)
            | self.sequence
    }

    /// Returns the current time in milliseconds since the Unix epoch.
    fn current_millis() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("SystemTime is before UNIX epoch")
            .as_millis() as u64
    }
}
