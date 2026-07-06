use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use redis::aio::MultiplexedConnection;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};

use super::{CacheLookup, CachedRoute, SessionCache, STALE_TTL_FACTOR};

const KEY_PREFIX: &str = "plano:affinity:";

/// Wire format for Redis entries. The physical Redis TTL is the stale window
/// (`ttl * STALE_TTL_FACTOR`); `logical_expires_at` marks the fresh/stale boundary.
/// Entries written by older versions lack the field and are treated as fresh until
/// their (shorter) physical TTL evicts them.
#[derive(Serialize, Deserialize)]
struct StoredEntry {
    #[serde(flatten)]
    route: CachedRoute,
    logical_expires_at: Option<u64>,
}

fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

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
    async fn get(&self, key: &str) -> Option<CacheLookup> {
        let mut conn = self.conn.clone();
        let value: Option<String> = conn.get(Self::make_key(key)).await.ok()?;
        let entry: StoredEntry = value.and_then(|v| serde_json::from_str(&v).ok())?;
        let is_stale = entry
            .logical_expires_at
            .is_some_and(|expires_at| now_epoch_secs() >= expires_at);
        Some(CacheLookup {
            route: entry.route,
            is_stale,
        })
    }

    async fn put(&self, key: &str, route: CachedRoute, ttl: Duration) {
        let mut conn = self.conn.clone();
        let ttl_secs = ttl.as_secs().max(1);
        let entry = StoredEntry {
            route,
            logical_expires_at: Some(now_epoch_secs() + ttl_secs),
        };
        let Ok(json) = serde_json::to_string(&entry) else {
            return;
        };
        let physical_ttl_secs = ttl_secs * STALE_TTL_FACTOR as u64;
        let _: Result<(), _> = conn
            .set_ex(Self::make_key(key), json, physical_ttl_secs)
            .await;
    }

    async fn remove(&self, key: &str) {
        let mut conn = self.conn.clone();
        let _: Result<(), _> = conn.del(Self::make_key(key)).await;
    }
}
