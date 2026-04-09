use async_trait::async_trait;
use redis::aio::MultiplexedConnection;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// A cached routing decision stored by session ID.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CachedRoute {
    pub model_name: String,
    pub route_name: Option<String>,
    /// Milliseconds since the UNIX epoch when this entry was created.
    /// Used only by the memory backend for TTL checks and eviction ordering;
    /// Redis uses native key expiry and ignores this field.
    pub cached_at_ms: u64,
}

/// Abstracts the session-affinity cache so it can be backed by in-memory
/// storage (default, single-replica) or Redis (multi-replica).
#[async_trait]
pub trait SessionCache: Send + Sync {
    /// Return a cached route for `session_id`, or `None` if absent/expired.
    async fn get(&self, session_id: &str) -> Option<CachedRoute>;

    /// Store a routing decision for `session_id`.
    async fn put(&self, session_id: &str, route: CachedRoute);

    /// Remove a session entry explicitly.
    async fn remove(&self, session_id: &str);

    /// Evict all expired entries (no-op for backends with native TTL such as Redis).
    async fn cleanup_expired(&self);
}

// ---------------------------------------------------------------------------
// In-memory backend
// ---------------------------------------------------------------------------

/// In-process session cache backed by a `RwLock<HashMap>`.
///
/// This is the default backend and replicates the previous behaviour of
/// `RouterService`. All state is local to the process, so it is only suitable
/// for single-replica deployments.
pub struct MemorySessionCache {
    inner: RwLock<HashMap<String, CachedRoute>>,
    ttl: Duration,
    max_entries: usize,
}

impl MemorySessionCache {
    pub fn new(ttl: Duration, max_entries: usize) -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
            ttl,
            max_entries,
        }
    }
}

/// Returns milliseconds since the UNIX epoch (for TTL bookkeeping).
fn unix_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as u64
}

#[async_trait]
impl SessionCache for MemorySessionCache {
    async fn get(&self, session_id: &str) -> Option<CachedRoute> {
        let cache = self.inner.read().await;
        if let Some(entry) = cache.get(session_id) {
            let age_ms = unix_now_ms().saturating_sub(entry.cached_at_ms);
            if Duration::from_millis(age_ms) < self.ttl {
                return Some(entry.clone());
            }
        }
        None
    }

    async fn put(&self, session_id: &str, route: CachedRoute) {
        let mut cache = self.inner.write().await;
        if cache.len() >= self.max_entries && !cache.contains_key(session_id) {
            // Evict the oldest entry by `cached_at_ms`.
            if let Some(oldest_key) = cache
                .iter()
                .min_by_key(|(_, v)| v.cached_at_ms)
                .map(|(k, _)| k.clone())
            {
                cache.remove(&oldest_key);
            }
        }
        cache.insert(session_id.to_string(), route);
    }

    async fn remove(&self, session_id: &str) {
        self.inner.write().await.remove(session_id);
    }

    async fn cleanup_expired(&self) {
        let mut cache = self.inner.write().await;
        let before = cache.len();
        let ttl_ms = self.ttl.as_millis() as u64;
        let now = unix_now_ms();
        cache.retain(|_, entry| now.saturating_sub(entry.cached_at_ms) < ttl_ms);
        let removed = before - cache.len();
        if removed > 0 {
            info!(
                removed = removed,
                remaining = cache.len(),
                "cleaned up expired session cache entries"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Redis backend
// ---------------------------------------------------------------------------

/// Shared-state session cache backed by Redis.
///
/// Uses `SET … EX` for automatic TTL-based expiry so that expired sessions are
/// cleaned up by Redis itself — no background cleanup task is needed.
pub struct RedisSessionCache {
    conn: Arc<RwLock<MultiplexedConnection>>,
    ttl_secs: u64,
}

impl RedisSessionCache {
    /// Connect to Redis at `url` and return a new cache instance.
    pub async fn new(
        url: &str,
        ttl_secs: u64,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let client = redis::Client::open(url)?;
        let conn = client.get_multiplexed_tokio_connection().await?;
        Ok(Self {
            conn: Arc::new(RwLock::new(conn)),
            ttl_secs,
        })
    }
}

#[async_trait]
impl SessionCache for RedisSessionCache {
    async fn get(&self, session_id: &str) -> Option<CachedRoute> {
        let mut conn = self.conn.write().await;
        let raw: Option<String> = conn.get(session_id).await.ok()?;
        let entry: CachedRoute = serde_json::from_str(&raw?).ok()?;
        Some(entry)
    }

    async fn put(&self, session_id: &str, route: CachedRoute) {
        let json = match serde_json::to_string(&route) {
            Ok(j) => j,
            Err(e) => {
                warn!(session_id = %session_id, error = %e, "failed to serialize CachedRoute for Redis");
                return;
            }
        };
        let mut conn = self.conn.write().await;
        if let Err(e) = conn
            .set_ex::<_, _, ()>(session_id, json, self.ttl_secs)
            .await
        {
            warn!(session_id = %session_id, error = %e, "failed to write session cache entry to Redis");
        }
    }

    async fn remove(&self, session_id: &str) {
        let mut conn = self.conn.write().await;
        if let Err(e) = conn.del::<_, ()>(session_id).await {
            debug!(session_id = %session_id, error = %e, "failed to delete session cache entry from Redis");
        }
    }

    /// Redis handles expiry natively — this is a no-op.
    async fn cleanup_expired(&self) {}
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_route(model: &str) -> CachedRoute {
        CachedRoute {
            model_name: model.to_string(),
            route_name: None,
            cached_at_ms: unix_now_ms(),
        }
    }

    #[tokio::test]
    async fn memory_cache_miss_returns_none() {
        let cache = MemorySessionCache::new(Duration::from_secs(600), 100);
        assert!(cache.get("unknown").await.is_none());
    }

    #[tokio::test]
    async fn memory_cache_hit_returns_entry() {
        let cache = MemorySessionCache::new(Duration::from_secs(600), 100);
        cache.put("s1", make_route("gpt-4o")).await;
        let hit = cache.get("s1").await.unwrap();
        assert_eq!(hit.model_name, "gpt-4o");
    }

    #[tokio::test]
    async fn memory_cache_expired_returns_none() {
        let cache = MemorySessionCache::new(Duration::ZERO, 100);
        cache.put("s1", make_route("gpt-4o")).await;
        assert!(cache.get("s1").await.is_none());
    }

    #[tokio::test]
    async fn memory_cache_cleanup_removes_expired() {
        let cache = MemorySessionCache::new(Duration::ZERO, 100);
        cache.put("s1", make_route("gpt-4o")).await;
        cache.put("s2", make_route("claude")).await;
        cache.cleanup_expired().await;
        assert!(cache.inner.read().await.is_empty());
    }

    #[tokio::test]
    async fn memory_cache_evicts_oldest_when_full() {
        let cache = MemorySessionCache::new(Duration::from_secs(600), 2);
        cache
            .put(
                "s1",
                CachedRoute {
                    model_name: "model-a".to_string(),
                    route_name: None,
                    cached_at_ms: unix_now_ms(),
                },
            )
            .await;
        tokio::time::sleep(Duration::from_millis(10)).await;
        cache
            .put(
                "s2",
                CachedRoute {
                    model_name: "model-b".to_string(),
                    route_name: None,
                    cached_at_ms: unix_now_ms(),
                },
            )
            .await;
        cache
            .put(
                "s3",
                CachedRoute {
                    model_name: "model-c".to_string(),
                    route_name: None,
                    cached_at_ms: unix_now_ms(),
                },
            )
            .await;
        let inner = cache.inner.read().await;
        assert_eq!(inner.len(), 2);
        assert!(!inner.contains_key("s1"), "s1 should have been evicted");
        assert!(inner.contains_key("s2"));
        assert!(inner.contains_key("s3"));
    }

    #[tokio::test]
    async fn memory_cache_remove_deletes_entry() {
        let cache = MemorySessionCache::new(Duration::from_secs(600), 100);
        cache.put("s1", make_route("gpt-4o")).await;
        cache.remove("s1").await;
        assert!(cache.get("s1").await.is_none());
    }

    #[tokio::test]
    async fn cached_route_serializes_round_trip() {
        let original = CachedRoute {
            model_name: "claude-3".to_string(),
            route_name: Some("code".to_string()),
            cached_at_ms: 1_700_000_000_000,
        };
        let json = serde_json::to_string(&original).unwrap();
        let decoded: CachedRoute = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.model_name, original.model_name);
        assert_eq!(decoded.route_name, original.route_name);
        assert_eq!(decoded.cached_at_ms, original.cached_at_ms);
    }
}
