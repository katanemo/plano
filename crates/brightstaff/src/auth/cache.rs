use super::token_resolver::{resolve_token, AuthContext, AuthError};
use crate::db::DbPool;
use moka::future::Cache;
use std::sync::Arc;
use std::time::Duration;

/// TTL-based auth cache using moka
#[derive(Clone)]
pub struct AuthCache {
    cache: Cache<String, Arc<AuthContext>>,
}

impl AuthCache {
    pub fn new() -> Self {
        let cache = Cache::builder()
            .max_capacity(10_000)
            .time_to_live(Duration::from_secs(60))
            .build();
        Self { cache }
    }

    /// Get auth context from cache or resolve from DB
    pub async fn get_or_resolve(
        &self,
        pool: &DbPool,
        token_hash: &str,
        raw_token: &str,
    ) -> Result<Arc<AuthContext>, AuthError> {
        if let Some(ctx) = self.cache.get(token_hash).await {
            return Ok(ctx);
        }

        let ctx = resolve_token(pool, raw_token).await?;
        let ctx = Arc::new(ctx);
        self.cache.insert(token_hash.to_string(), ctx.clone()).await;
        Ok(ctx)
    }

    /// Invalidate a cached token
    pub async fn invalidate(&self, token_hash: &str) {
        self.cache.invalidate(token_hash).await;
    }
}
