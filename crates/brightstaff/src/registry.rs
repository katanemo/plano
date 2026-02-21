use crate::db::DbPool;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info};
use uuid::Uuid;

/// Info about a registered API key's project
#[derive(Debug, Clone)]
pub struct RegisteredKeyInfo {
    pub project_id: Uuid,
    pub provider: String,
    pub upstream_url: String,
    pub display_name: Option<String>,
    pub is_active: bool,
    pub egress_ip: String,
}

/// In-memory registry of API key hashes to project info.
/// Loaded from DB, refreshed periodically by a background task.
#[derive(Clone)]
pub struct ApiKeyRegistry {
    entries: Arc<RwLock<HashMap<String, RegisteredKeyInfo>>>,
}

impl Default for ApiKeyRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ApiKeyRegistry {
    pub fn new() -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Look up a registered API key by its hash
    pub async fn lookup(&self, key_hash: &str) -> Option<RegisteredKeyInfo> {
        let entries = self.entries.read().await;
        entries.get(key_hash).cloned()
    }

    /// Reload the registry from the database
    pub async fn reload(&self, pool: &DbPool) -> Result<usize, Box<dyn std::error::Error>> {
        let client = pool.get_client().await?;
        let rows = client
            .query(
                r#"
                SELECT key_hash, project_id, provider, upstream_url, display_name, is_active, egress_ip
                FROM registered_api_keys
                WHERE is_active = true
                "#,
                &[],
            )
            .await?;

        let mut new_entries = HashMap::with_capacity(rows.len());
        for row in &rows {
            let key_hash: String = row.get("key_hash");
            let info = RegisteredKeyInfo {
                project_id: row.get("project_id"),
                provider: row.get("provider"),
                upstream_url: row.get("upstream_url"),
                display_name: row.get("display_name"),
                is_active: row.get("is_active"),
                egress_ip: row.get("egress_ip"),
            };
            new_entries.insert(key_hash, info);
        }

        let count = new_entries.len();
        *self.entries.write().await = new_entries;
        Ok(count)
    }

    /// Start a background task that refreshes the registry every `interval_secs` seconds
    pub fn start_refresh_task(self, pool: DbPool, interval_secs: u64) {
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(tokio::time::Duration::from_secs(interval_secs));

            loop {
                interval.tick().await;
                match self.reload(&pool).await {
                    Ok(count) => {
                        info!(keys = count, "refreshed API key registry");
                    }
                    Err(e) => {
                        error!(error = %e, "failed to refresh API key registry");
                    }
                }
            }
        });
    }
}
