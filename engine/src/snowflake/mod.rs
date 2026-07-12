use std::time::{SystemTime, UNIX_EPOCH};

const EPOCH: u64 = 1_700_000_000_000; // Custom epoch: Nov 18 2023 in ms
pub const DATACENTER_BITS: u64 = 5;
pub const MACHINE_BITS: u64 = 5;
pub const SEQUENCE_BITS: u64 = 12;

pub const MAX_DATACENTER_ID: u64 = (1 << DATACENTER_BITS) - 1;
pub const MAX_MACHINE_ID: u64 = (1 << MACHINE_BITS) - 1;
pub const MAX_SEQUENCE: u64 = (1 << SEQUENCE_BITS) - 1;

pub const MACHINE_SHIFT: u64 = SEQUENCE_BITS;
pub const DATACENTER_SHIFT: u64 = SEQUENCE_BITS + MACHINE_BITS;
pub const TIMESTAMP_SHIFT: u64 = SEQUENCE_BITS + MACHINE_BITS + DATACENTER_BITS;

#[derive(Debug)]
pub struct SnowflakeGenerator {
    machine_id: u64,
    datacenter_id: u64,
    sequence: u64,
    last_timestamp: u64,
}

impl SnowflakeGenerator {
    pub fn new(machine_id: u64, datacenter_id: u64) -> Self {
        assert!(machine_id <= MAX_MACHINE_ID, "machine_id must be 0-{}", MAX_MACHINE_ID);
        assert!(datacenter_id <= MAX_DATACENTER_ID, "datacenter_id must be 0-{}", MAX_DATACENTER_ID);
        SnowflakeGenerator {
            machine_id,
            datacenter_id,
            sequence: 0,
            last_timestamp: 0,
        }
    }

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

    fn current_millis() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("SystemTime is before UNIX epoch")
            .as_millis() as u64
    }
}
