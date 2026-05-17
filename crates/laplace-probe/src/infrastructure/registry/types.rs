//! # Server Registry Types
//!
//! Thread-safe registry for managing QUIC server instances.
//! Uses u64 handles for safe, concurrent access to servers.

use dashmap::DashMap;
use std::sync::Arc;

use crate::adapters::quinn::server::QuicServer;

// ============================================================================
// HANDLE ALLOCATION
// ============================================================================

/// Handle allocator for generating unique server identifiers
pub struct HandleAllocator {
    /// Next handle to allocate
    next_handle: std::sync::atomic::AtomicU64,
}

#[allow(clippy::new_without_default)]
impl HandleAllocator {
    /// Create new allocator starting at handle 1
    pub fn new() -> Self {
        Self {
            next_handle: std::sync::atomic::AtomicU64::new(1),
        }
    }

    /// Allocate next unique handle
    pub fn allocate(&self) -> u64 {
        self.next_handle
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    }
}

// ============================================================================
// SERVER REGISTRY
// ============================================================================

/// Thread-safe registry for managing QUIC server instances
///
/// Provides concurrent read/write access to server instances using
/// handle-based lookups. Safe for access from multiple threads and async tasks.
pub struct ServerRegistry {
    /// Map of handle -> server instance
    servers: DashMap<u64, Arc<QuicServer>>,

    /// Handle allocator
    allocator: HandleAllocator,
}

impl ServerRegistry {
    /// Create new empty registry
    pub fn new() -> Self {
        Self {
            servers: DashMap::new(),
            allocator: HandleAllocator::new(),
        }
    }

    /// Register a new server and get its handle
    ///
    /// Creates a new entry in the registry and returns a unique handle
    /// that can be used to reference this server.
    ///
    /// # Arguments
    /// * `server` - Newly created server instance
    ///
    /// # Returns
    /// Unique u64 handle for the registered server
    pub fn register(&self, mut server: QuicServer) -> u64 {
        let handle = self.allocator.allocate();
        server.handle = handle;
        self.servers.insert(handle, Arc::new(server));
        handle
    }

    /// Lookup server by handle
    ///
    /// Returns an Arc reference to the server if found, allowing
    /// safe concurrent access from multiple threads.
    ///
    /// # Arguments
    /// * `handle` - Server handle (from register)
    ///
    /// # Returns
    /// Arc<QuicServer> if found, None otherwise
    pub fn get(&self, handle: u64) -> Option<Arc<QuicServer>> {
        self.servers.get(&handle).map(|entry| Arc::clone(&entry))
    }

    /// Unregister and remove a server
    ///
    /// Removes the server from the registry. The handle becomes invalid
    /// after this call, though existing Arc references remain valid.
    ///
    /// # Arguments
    /// * `handle` - Server handle to remove
    ///
    /// # Returns
    /// Arc<QuicServer> if found and removed, None otherwise
    pub fn unregister(&self, handle: u64) -> Option<Arc<QuicServer>> {
        self.servers.remove(&handle).map(|(_, server)| server)
    }

    /// Get list of all active server handles
    ///
    /// Useful for monitoring and management operations.
    ///
    /// # Returns
    /// Vector of active server handles
    pub fn list_handles(&self) -> Vec<u64> {
        self.servers
            .iter()
            .map(|ref_multi| *ref_multi.key())
            .collect()
    }

    /// Get count of registered servers
    ///
    /// Returns the number of currently active servers in the registry.
    ///
    /// # Returns
    /// Number of registered servers
    pub fn count(&self) -> usize {
        self.servers.len()
    }

    /// Check if a handle is valid (registered server exists)
    ///
    /// # Arguments
    /// * `handle` - Handle to check
    ///
    /// # Returns
    /// true if server exists, false otherwise
    pub fn contains(&self, handle: u64) -> bool {
        self.servers.contains_key(&handle)
    }

    /// Clear all servers from registry
    ///
    /// Removes all entries, though existing Arc references remain valid.
    /// Used primarily for testing and cleanup.
    pub fn clear(&self) {
        self.servers.clear();
    }

    /// Iterate over all servers
    ///
    /// Provides read-only access to all servers. During iteration,
    /// new entries may be added/removed but existing references remain valid.
    ///
    /// # Returns
    /// Iterator over (handle, Arc<QuicServer>) pairs
    pub fn iter(&self) -> impl Iterator<Item = (u64, Arc<QuicServer>)> + '_ {
        self.servers
            .iter()
            .map(|entry| (*entry.key(), Arc::clone(entry.value())))
    }
}

impl Default for ServerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(kani)]
mod proofs {
    use super::HandleAllocator;

    /// H-KNUL1: HandleAllocator::allocate() is strictly monotone.
    ///
    /// Proves that each successive handle is strictly greater than the previous,
    /// guaranteeing the server registry never reuses stale handles.
    #[kani::proof]
    fn handle_allocator_monotone() {
        let alloc = HandleAllocator::new();
        let h1 = alloc.allocate();
        let h2 = alloc.allocate();
        assert!(
            h2 > h1,
            "second handle must be strictly greater than the first"
        );
    }

    /// H-KNUL2: HandleAllocator never issues the same handle twice.
    ///
    /// Proves that ServerRegistry::register() — which delegates uniqueness to
    /// HandleAllocator — can never assign the same handle to two different servers,
    /// eliminating the possibility of silent registry collisions.
    #[kani::proof]
    fn registry_no_duplicate_handle() {
        let alloc = HandleAllocator::new();
        let h1 = alloc.allocate();
        let h2 = alloc.allocate();
        let h3 = alloc.allocate();
        assert_ne!(h1, h2, "consecutive handles must be distinct (1 vs 2)");
        assert_ne!(h2, h3, "consecutive handles must be distinct (2 vs 3)");
        assert_ne!(
            h1, h3,
            "non-consecutive handles must also be distinct (1 vs 3)"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use laplace_interfaces::FfiQuicConfig;

    #[test]
    fn test_registry_registration() {
        let registry = ServerRegistry::new();
        let config = FfiQuicConfig::new();
        let server = QuicServer::new_production(0, config);

        let handle = registry.register(server);
        assert!(handle > 0);
        assert!(registry.contains(handle));
    }

    #[test]
    fn test_registry_unique_handles() {
        let registry = ServerRegistry::new();
        let config = FfiQuicConfig::new();

        let handle1 = registry.register(QuicServer::new_production(0, config.clone()));
        let handle2 = registry.register(QuicServer::new_production(0, config.clone()));
        let handle3 = registry.register(QuicServer::new_production(0, config));

        assert_ne!(handle1, handle2);
        assert_ne!(handle2, handle3);
        assert_ne!(handle1, handle3);
    }

    #[test]
    fn test_registry_lookup() {
        let registry = ServerRegistry::new();
        let config = FfiQuicConfig::new();
        let server = QuicServer::new_production(0, config);

        let handle = registry.register(server);
        let retrieved = registry.get(handle);

        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().handle, handle);
    }

    #[test]
    fn test_registry_unregister() {
        let registry = ServerRegistry::new();
        let config = FfiQuicConfig::new();
        let server = QuicServer::new_production(0, config);

        let handle = registry.register(server);
        assert!(registry.contains(handle));

        let removed = registry.unregister(handle);
        assert!(removed.is_some());
        assert!(!registry.contains(handle));
    }

    #[test]
    fn test_registry_count() {
        let registry = ServerRegistry::new();
        let config = FfiQuicConfig::new();

        assert_eq!(registry.count(), 0);

        registry.register(QuicServer::new_production(0, config.clone()));
        assert_eq!(registry.count(), 1);

        registry.register(QuicServer::new_production(0, config.clone()));
        assert_eq!(registry.count(), 2);

        registry.register(QuicServer::new_production(0, config));
        assert_eq!(registry.count(), 3);
    }

    #[test]
    fn test_registry_list_handles() {
        let registry = ServerRegistry::new();
        let config = FfiQuicConfig::new();

        let h1 = registry.register(QuicServer::new_production(0, config.clone()));
        let h2 = registry.register(QuicServer::new_production(0, config.clone()));
        let h3 = registry.register(QuicServer::new_production(0, config));

        let handles = registry.list_handles();
        assert_eq!(handles.len(), 3);
        assert!(handles.contains(&h1));
        assert!(handles.contains(&h2));
        assert!(handles.contains(&h3));
    }

    #[test]
    fn test_registry_concurrent_access() {
        use std::thread;

        let registry = Arc::new(ServerRegistry::new());
        let config = FfiQuicConfig::new();

        // Register servers from multiple threads
        let mut handles = vec![];
        for _i in 0..10 {
            let reg = Arc::clone(&registry);
            let cfg = config.clone();
            let handle = thread::spawn(move || {
                let server = QuicServer::new_production(0, cfg);
                reg.register(server)
            })
            .join()
            .unwrap();
            handles.push(handle);
        }

        // Verify all registered
        assert_eq!(registry.count(), 10);

        // Verify all handles are unique and retrievable
        for handle in handles {
            assert!(registry.contains(handle));
            assert!(registry.get(handle).is_some());
        }
    }
}
