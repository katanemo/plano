use std::time::Duration;

use async_trait::async_trait;
use redis::aio::MultiplexedConnection;
use redis::AsyncCommands;

use super::{CachedRoute, SessionCache};

pub struct RedisSessionCache {
    conn: MultiplexedConnection,
}

impl RedisSessionCache {
    pub async fn new(url: &str) -> Result<Self, redis::RedisError> {
        let client = redis::Client::open(url)?;
        let conn = client.get_multiplexed_async_connection().await?;
        Ok(Self { conn })
    }
}

#[async_trait]
impl SessionCache for RedisSessionCache {
    async fn get(&self, session_id: &str) -> Option<CachedRoute> {
        let mut conn = self.conn.clone();
        let value: Option<String> = conn.get(session_id).await.ok()?;
        value.and_then(|v| serde_json::from_str(&v).ok())
    }

    async fn put(&self, session_id: &str, route: CachedRoute, ttl: Duration) {
        let mut conn = self.conn.clone();
        let Ok(json) = serde_json::to_string(&route) else {
            return;
        };
        let ttl_secs = ttl.as_secs().max(1);
        let _: Result<(), _> = conn.set_ex(session_id, json, ttl_secs).await;
    }

    async fn remove(&self, session_id: &str) {
        let mut conn = self.conn.clone();
        let _: Result<(), _> = conn.del(session_id).await;
    }

    async fn cleanup_expired(&self) {
        // Redis handles TTL expiry natively via EX — nothing to do here.
    }
}
