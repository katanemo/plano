use async_trait::async_trait;
use std::time::Duration;

pub mod memory;
pub mod redis;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct CachedRoute {
    pub model_name: String,
    pub route_name: Option<String>,
}

#[async_trait]
pub trait SessionCache: Send + Sync {
    /// Look up a cached routing decision by session ID.
    async fn get(&self, session_id: &str) -> Option<CachedRoute>;

    /// Store a routing decision in the session cache with the given TTL.
    async fn put(&self, session_id: &str, route: CachedRoute, ttl: Duration);

    /// Remove a cached routing decision by session ID.
    async fn remove(&self, session_id: &str);

    /// Remove all expired entries. No-op for backends that handle expiry natively (e.g. Redis).
    async fn cleanup_expired(&self);
}
