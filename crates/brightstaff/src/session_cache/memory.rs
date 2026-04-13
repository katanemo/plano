use std::{
    num::NonZeroUsize,
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use lru::LruCache;
use tokio::sync::Mutex;
use tracing::info;

use super::{CachedRoute, SessionCache};

type CacheStore = Mutex<LruCache<String, (CachedRoute, Instant, Duration)>>;

pub struct MemorySessionCache {
    store: Arc<CacheStore>,
}

impl MemorySessionCache {
    pub fn new(max_entries: usize) -> Self {
        let capacity = NonZeroUsize::new(max_entries)
            .unwrap_or_else(|| NonZeroUsize::new(10_000).expect("10_000 is non-zero"));
        let store = Arc::new(Mutex::new(LruCache::new(capacity)));

        // Spawn a background task to evict TTL-expired entries every 5 minutes.
        let store_clone = Arc::clone(&store);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(300));
            loop {
                interval.tick().await;
                Self::evict_expired(&store_clone).await;
            }
        });

        Self { store }
    }

    async fn evict_expired(store: &CacheStore) {
        let mut cache = store.lock().await;
        let expired: Vec<String> = cache
            .iter()
            .filter(|(_, (_, inserted_at, ttl))| inserted_at.elapsed() >= *ttl)
            .map(|(k, _)| k.clone())
            .collect();
        let removed = expired.len();
        for key in &expired {
            cache.pop(key.as_str());
        }
        if removed > 0 {
            info!(
                removed = removed,
                remaining = cache.len(),
                "cleaned up expired session cache entries"
            );
        }
    }
}

#[async_trait]
impl SessionCache for MemorySessionCache {
    async fn get(&self, key: &str) -> Option<CachedRoute> {
        let mut cache = self.store.lock().await;
        if let Some((route, inserted_at, ttl)) = cache.get(key) {
            if inserted_at.elapsed() < *ttl {
                return Some(route.clone());
            }
        }
        None
    }

    async fn put(&self, key: &str, route: CachedRoute, ttl: Duration) {
        self.store
            .lock()
            .await
            .put(key.to_string(), (route, Instant::now(), ttl));
    }

    async fn remove(&self, key: &str) {
        self.store.lock().await.pop(key);
    }
}
