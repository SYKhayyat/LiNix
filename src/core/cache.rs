use crate::core::Result;
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// A generic thread-safe cache with TTL support
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
    /// Create a new cache with specified TTL
    pub fn new(ttl: Duration) -> Self {
        Self {
            store: Arc::new(RwLock::new(HashMap::new())),
            ttl,
        }
    }

    /// Get a value from the cache
    pub async fn get(&self, key: &K) -> Option<V> {
        let store = self.store.read().await;

        if let Some(entry) = store.get(key) {
            if entry.expires_at > Instant::now() {
                return Some(entry.value.clone());
            }
        }

        None
    }

    /// Insert a value into the cache
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

    /// Remove a value from the cache
    pub async fn remove(&self, key: &K) {
        let mut store = self.store.write().await;
        store.remove(key);
    }

    /// Clear all entries from the cache
    pub async fn clear(&self) {
        let mut store = self.store.write().await;
        store.clear();
    }

    /// Remove expired entries
    pub async fn cleanup(&self) {
        let mut store = self.store.write().await;
        let now = Instant::now();

        store.retain(|_, entry| entry.expires_at > now);
    }

    /// Get the number of entries in the cache (including expired)
    pub async fn len(&self) -> usize {
        let store = self.store.read().await;
        store.len()
    }

    /// Check if the cache is empty
    pub async fn is_empty(&self) -> bool {
        let store = self.store.read().await;
        store.is_empty()
    }

    /// Get or insert a value using a provided function
    pub async fn get_or_insert_with<F, Fut>(&self, key: K, f: F) -> Result<V>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<V>>,
    {
        if let Some(value) = self.get(&key).await {
            return Ok(value);
        }

        let value = f().await?;
        self.set(key, value.clone()).await;

        Ok(value)
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

/// Cache for package-related data
pub struct PackageCache {
    /// Cache for installed packages by backend
    installed: SmartCache<String, Vec<String>>,

    /// Cache for search results
    search: SmartCache<String, Vec<crate::core::Package>>,

    /// Cache for package info
    info: SmartCache<String, crate::core::Package>,
}

impl PackageCache {
    /// Create a new package cache with default TTLs
    pub fn new() -> Self {
        Self {
            installed: SmartCache::new(Duration::from_secs(300)),
            search: SmartCache::new(Duration::from_secs(600)),
            info: SmartCache::new(Duration::from_secs(300)),
        }
    }

    /// Create a new package cache with custom TTLs
    pub fn with_ttls(installed_ttl: Duration, search_ttl: Duration, info_ttl: Duration) -> Self {
        Self {
            installed: SmartCache::new(installed_ttl),
            search: SmartCache::new(search_ttl),
            info: SmartCache::new(info_ttl),
        }
    }

    /// Get installed packages for a backend
    pub async fn get_installed(&self, backend: &str) -> Option<Vec<String>> {
        self.installed.get(&backend.to_string()).await
    }

    /// Set installed packages for a backend
    pub async fn set_installed(&self, backend: String, packages: Vec<String>) {
        self.installed.set(backend, packages).await;
    }

    /// Get search results
    pub async fn get_search(&self, query: &str) -> Option<Vec<crate::core::Package>> {
        self.search.get(&query.to_string()).await
    }

    /// Set search results
    pub async fn set_search(&self, query: String, results: Vec<crate::core::Package>) {
        self.search.set(query, results).await;
    }

    /// Get package info
    pub async fn get_info(&self, package: &str) -> Option<crate::core::Package> {
        self.info.get(&package.to_string()).await
    }

    /// Set package info
    pub async fn set_info(&self, package: String, info: crate::core::Package) {
        self.info.set(package, info).await;
    }

    /// Clear all caches
    pub async fn clear_all(&self) {
        self.installed.clear().await;
        self.search.clear().await;
        self.info.clear().await;
    }

    /// Cleanup expired entries in all caches
    pub async fn cleanup_all(&self) {
        self.installed.cleanup().await;
        self.search.cleanup().await;
        self.info.cleanup().await;
    }
}

impl Default for PackageCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_cache_basic_operations() {
        let cache: SmartCache<String, String> = SmartCache::new(Duration::from_secs(60));

        cache.set("key1".to_string(), "value1".to_string()).await;

        let value = cache.get(&"key1".to_string()).await;
        assert_eq!(value, Some("value1".to_string()));

        cache.remove(&"key1".to_string()).await;
        let value = cache.get(&"key1".to_string()).await;
        assert_eq!(value, None);
    }

    #[tokio::test]
    async fn test_cache_expiration() {
        let cache: SmartCache<String, String> = SmartCache::new(Duration::from_millis(100));

        cache.set("key1".to_string(), "value1".to_string()).await;

        assert!(cache.get(&"key1".to_string()).await.is_some());

        tokio::time::sleep(Duration::from_millis(150)).await;

        assert!(cache.get(&"key1".to_string()).await.is_none());
    }

    #[tokio::test]
    async fn test_package_cache() {
        let cache = PackageCache::new();

        let packages = vec!["pkg1".to_string(), "pkg2".to_string()];
        cache.set_installed("apt".to_string(), packages.clone()).await;

        let retrieved = cache.get_installed("apt").await;
        assert_eq!(retrieved, Some(packages));
    }
}
