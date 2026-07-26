use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

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
    pub fn new(ttl: Duration) -> Self {
        Self {
            store: Arc::new(RwLock::new(HashMap::new())),
            ttl,
        }
    }

    pub async fn get(&self, key: &K) -> Option<V> {
        let store = self.store.read().await;

        if let Some(entry) = store.get(key) {
            if entry.expires_at > Instant::now() {
                return Some(entry.value.clone());
            }
        }

        None
    }

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

    pub async fn remove(&self, key: &K) {
        let mut store = self.store.write().await;
        store.remove(key);
    }

    pub async fn clear(&self) {
        let mut store = self.store.write().await;
        store.clear();
    }

    pub async fn cleanup(&self) {
        let mut store = self.store.write().await;
        let now = Instant::now();

        store.retain(|_, entry| entry.expires_at > now);
    }

    /// Counts expired-but-uncleaned entries too, so a non-zero len does not mean a `get`
    /// will hit.
    pub async fn len(&self) -> usize {
        let store = self.store.read().await;
        store.len()
    }

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

pub struct PackageCache {
    installed: SmartCache<String, Vec<crate::core::Package>>,
    search: SmartCache<String, Vec<crate::core::Package>>,
    info: SmartCache<String, crate::core::Package>,
}

impl PackageCache {
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
