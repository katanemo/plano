use std::time::Duration;

use async_trait::async_trait;
use redis::aio::MultiplexedConnection;
use redis::AsyncCommands;

use super::{SessionBinding, SessionCache};

const KEY_PREFIX: &str = "plano:affinity:";

pub struct RedisSessionCache {
    conn: MultiplexedConnection,
}

impl RedisSessionCache {
    pub async fn new(url: &str) -> Result<Self, redis::RedisError> {
        let client = redis::Client::open(url)?;
        let conn = client.get_multiplexed_async_connection().await?;
        Ok(Self { conn })
    }

    fn make_key(key: &str) -> String {
        format!("{KEY_PREFIX}{key}")
    }
}

#[async_trait]
impl SessionCache for RedisSessionCache {
    async fn get(&self, key: &str) -> Option<SessionBinding> {
        let mut conn = self.conn.clone();
        let value: Option<String> = conn.get(Self::make_key(key)).await.ok()?;
        value.and_then(|v| serde_json::from_str(&v).ok())
    }

    async fn put(&self, key: &str, binding: SessionBinding, ttl: Duration) {
        let mut conn = self.conn.clone();
        // The Redis TTL is only a GC bound; warmth is decided by the router from
        // `binding.last_used`, not by expiry here.
        let ttl_secs = ttl.as_secs().max(1);
        let Ok(json) = serde_json::to_string(&binding) else {
            return;
        };
        let _: Result<(), _> = conn.set_ex(Self::make_key(key), json, ttl_secs).await;
    }

    async fn remove(&self, key: &str) {
        let mut conn = self.conn.clone();
        let _: Result<(), _> = conn.del(Self::make_key(key)).await;
    }
}
