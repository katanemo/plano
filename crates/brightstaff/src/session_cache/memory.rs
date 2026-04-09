use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use tokio::sync::RwLock;
use tracing::info;

use super::{CachedRoute, SessionCache};

pub struct MemorySessionCache {
    store: Arc<RwLock<HashMap<String, (CachedRoute, Instant)>>>,
    ttl: Duration,
    max_entries: usize,
}

impl MemorySessionCache {
    pub fn new(ttl: Duration, max_entries: usize) -> Self {
        Self {
            store: Arc::new(RwLock::new(HashMap::new())),
            ttl,
            max_entries,
        }
    }
}

#[async_trait]
impl SessionCache for MemorySessionCache {
    async fn get(&self, session_id: &str) -> Option<CachedRoute> {
        let store = self.store.read().await;
        if let Some((route, inserted_at)) = store.get(session_id) {
            if inserted_at.elapsed() < self.ttl {
                return Some(route.clone());
            }
        }
        None
    }

    async fn put(&self, session_id: &str, route: CachedRoute, _ttl: Duration) {
        let mut store = self.store.write().await;
        if store.len() >= self.max_entries && !store.contains_key(session_id) {
            if let Some(oldest_key) = store
                .iter()
                .min_by_key(|(_, (_, inserted_at))| *inserted_at)
                .map(|(k, _)| k.clone())
            {
                store.remove(&oldest_key);
            }
        }
        store.insert(session_id.to_string(), (route, Instant::now()));
    }

    async fn remove(&self, session_id: &str) {
        self.store.write().await.remove(session_id);
    }

    async fn cleanup_expired(&self) {
        let ttl = self.ttl;
        let mut store = self.store.write().await;
        let before = store.len();
        store.retain(|_, (_, inserted_at)| inserted_at.elapsed() < ttl);
        let removed = before - store.len();
        if removed > 0 {
            info!(
                removed = removed,
                remaining = store.len(),
                "cleaned up expired session cache entries"
            );
        }
    }
}
