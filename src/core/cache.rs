use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// A generic thread-safe cache with TTL (Time-To-Live) support.
/// Utilizes RwLock for high-concurrency read access.
pub struct SmartCache<K, V>
where
    K: Eq + Hash + Clone,
    V: Clone,
{
    store: Arc<RwLock<HashMap<K, CacheEntry<V>>>>,
    ttl: Duration,
}

struct CacheEntry<V> {
    value: V,
    expires_at: Instant,
}

impl<K, V> SmartCache<K, V>
where
    K: Eq + Hash + Clone,
    V: Clone,
{
    /// Create a new cache with specified TTL.
    pub fn new(ttl: Duration) -> Self {
        Self {
            store: Arc::new(RwLock::new(HashMap::new())),
            ttl,
        }
    }

    /// Get a value from the cache if it hasn't expired.
    pub async fn get(&self, key: &K) -> Option<V> {
        let store = self.store.read().await;

        if let Some(entry) = store.get(key) {
            if entry.expires_at > Instant::now() {
                return Some(entry.value.clone());
            }
        }

        None
    }

    /// Insert a value into the cache.
    pub async fn set(&self, key: K, value: V) {
        let mut store = self.store.write().await;

        store.insert(
            key,
            CacheEntry {
                value,
                expires_at: Instant::now() + self.ttl,
            },
        );
    }

    /// Remove a value from the cache.
    pub async fn remove(&self, key: &K) {
        let mut store = self.store.write().await;
        store.remove(key);
    }

    /// Clear all entries from the cache.
    pub async fn clear(&self) {
        let mut store = self.store.write().await;
        store.clear();
    }

    /// Explicitly remove expired entries from the cache.
    pub async fn cleanup(&self) {
        let mut store = self.store.write().await;
        let now = Instant::now();

        store.retain(|_, entry| entry.expires_at > now);
    }

    /// Get the number of entries in the cache.
    pub async fn len(&self) -> usize {
        let store = self.store.read().await;
        store.len()
    }

    /// Returns true if the cache holds no entries.
    pub async fn is_empty(&self) -> bool {
        self.len().await == 0
    }

}

impl<K, V> Clone for SmartCache<K, V>
where
    K: Eq + Hash + Clone,
    V: Clone,
{
    fn clone(&self) -> Self {
        Self {
            store: Arc::clone(&self.store),
            ttl: self.ttl,
        }
    }
}

/// Specialized cache for package-related data.
/// Used by the App context to speed up repeat lookups during resolution and search.
pub struct PackageCache {
    /// Cache for installed packages by backend.
    installed: SmartCache<String, Vec<crate::core::Package>>,
    /// Cache for cross-backend search results.
    search: SmartCache<String, Vec<crate::core::Package>>,
    /// Cache for package metadata.
    info: SmartCache<String, crate::core::Package>,
}

impl PackageCache {
    /// Initializes the package cache with default TTLs (5-10 minutes).
    pub fn new() -> Self {
        Self {
            installed: SmartCache::new(Duration::from_secs(300)),
            search: SmartCache::new(Duration::from_secs(600)),
            info: SmartCache::new(Duration::from_secs(300)),
        }
    }

    pub async fn get_installed(&self, backend: &str) -> Option<Vec<crate::core::Package>> {
        self.installed.get(&backend.to_string()).await
    }

    pub async fn set_installed(&self, backend: String, packages: Vec<crate::core::Package>) {
        self.installed.set(backend, packages).await;
    }

    pub async fn get_search(&self, query: &str) -> Option<Vec<crate::core::Package>> {
        self.search.get(&query.to_string()).await
    }

    pub async fn set_search(&self, query: String, results: Vec<crate::core::Package>) {
        self.search.set(query, results).await;
    }

    pub async fn get_info(&self, package: &str) -> Option<crate::core::Package> {
        self.info.get(&package.to_string()).await
    }

    pub async fn set_info(&self, package: String, info: crate::core::Package) {
        self.info.set(package, info).await;
    }

    pub async fn clear_all(&self) {
        self.installed.clear().await;
        self.search.clear().await;
        self.info.clear().await;
    }
}

impl Default for PackageCache {
    fn default() -> Self {
        Self::new()
    }
}
