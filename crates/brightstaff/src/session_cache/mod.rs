use std::sync::Arc;

use async_trait::async_trait;
use common::configuration::Configuration;
use std::time::Duration;
use tracing::{debug, info};

pub mod memory;
pub mod redis;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct CachedRoute {
    pub model_name: String,
    pub route_name: Option<String>,
}

#[async_trait]
pub trait SessionCache: Send + Sync {
    /// Look up a cached routing decision by key.
    async fn get(&self, key: &str) -> Option<CachedRoute>;

    /// Store a routing decision in the session cache with the given TTL.
    async fn put(&self, key: &str, route: CachedRoute, ttl: Duration);

    /// Remove a cached routing decision by key.
    async fn remove(&self, key: &str);
}

/// Initialize the session cache backend from config.
/// Defaults to the in-memory backend when no `session_cache` block is configured.
pub async fn init_session_cache(
    config: &Configuration,
) -> Result<Arc<dyn SessionCache>, Box<dyn std::error::Error + Send + Sync>> {
    use common::configuration::SessionCacheType;

    let session_max_entries = config.routing.as_ref().and_then(|r| r.session_max_entries);

    const DEFAULT_SESSION_MAX_ENTRIES: usize = 10_000;
    const MAX_SESSION_MAX_ENTRIES: usize = 10_000;

    let max_entries = session_max_entries
        .unwrap_or(DEFAULT_SESSION_MAX_ENTRIES)
        .min(MAX_SESSION_MAX_ENTRIES);

    let cache_config = config
        .routing
        .as_ref()
        .and_then(|r| r.session_cache.as_ref());

    let cache_type = cache_config
        .map(|c| &c.cache_type)
        .unwrap_or(&SessionCacheType::Memory);

    match cache_type {
        SessionCacheType::Memory => {
            info!(storage_type = "memory", "initialized session cache");
            Ok(Arc::new(memory::MemorySessionCache::new(max_entries)))
        }
        SessionCacheType::Redis => {
            let url = cache_config
                .and_then(|c| c.url.as_ref())
                .ok_or("session_cache.url is required when type is redis")?;
            debug!(storage_type = "redis", url = %url, "initializing session cache");
            let cache = redis::RedisSessionCache::new(url)
                .await
                .map_err(|e| format!("failed to connect to Redis session cache: {e}"))?;
            Ok(Arc::new(cache))
        }
    }
}
