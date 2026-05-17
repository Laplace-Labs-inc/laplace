//! Memory Model Type Definitions — re-exported from `laplace-interfaces`
//!
//! All canonical types live in `laplace_interfaces::domain::memory::types`.
//! This file re-exports them so that code within `laplace-core` can continue
//! to use the short path `crate::domain::memory::{Address, …}`.

pub use laplace_interfaces::domain::memory::types::{
    Address, ConsistencyModel, CoreId, MemoryConfig, MemoryOp, StoreEntry, Value,
};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_store_entry_creation() {
        let entry = StoreEntry::new(Address::new(42), Value::new(100));
        assert_eq!(entry.addr.0, 42);
        assert_eq!(entry.val.0, 100);
    }

    #[test]
    fn test_store_entry_equality() {
        let entry1 = StoreEntry::new(Address::new(10), Value::new(20));
        let entry2 = StoreEntry::new(Address::new(10), Value::new(20));
        let entry3 = StoreEntry::new(Address::new(10), Value::new(30));

        assert_eq!(entry1, entry2);
        assert_ne!(entry1, entry3);
    }

    #[test]
    fn test_memory_op_display() {
        let op = MemoryOp::Read {
            core: CoreId::new(0),
            addr: Address::new(100),
        };

        assert_eq!(format!("{}", op), "Read(core=Core(0), addr=0x64)");

        let write = MemoryOp::Write {
            core: CoreId::new(1),
            addr: Address::new(200),
            val: Value::new(42),
        };
        assert_eq!(write.to_string(), "Write(core=Core(1), addr=0xc8, val=42)");

        let fence = MemoryOp::Fence {
            core: CoreId::new(0),
        };
        assert_eq!(fence.to_string(), "Fence(core=Core(0))");
    }

    #[test]
    fn test_memory_config_default() {
        let config = MemoryConfig::default();
        assert_eq!(config.num_cores, 2);
        assert_eq!(config.max_buffer_size, 2);
        assert_eq!(config.consistency_model, ConsistencyModel::Relaxed);
        assert_eq!(config.initial_size, 1024);
    }

    #[test]
    fn test_memory_config_custom() {
        let config = MemoryConfig {
            num_cores: 4,
            max_buffer_size: 8,
            consistency_model: ConsistencyModel::SequentiallyConsistent,
            initial_size: 4096,
        };

        assert_eq!(config.num_cores, 4);
        assert_eq!(config.max_buffer_size, 8);
        assert_eq!(
            config.consistency_model,
            ConsistencyModel::SequentiallyConsistent
        );
        assert_eq!(config.initial_size, 4096);
    }
}
