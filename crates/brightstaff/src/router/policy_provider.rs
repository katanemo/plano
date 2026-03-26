//! External HTTP routing policy provider.
//!
//! Fetches routing policies from an external HTTP endpoint with caching support.
//! Policies are cached by `policy_id` with revision-aware invalidation.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::configuration::ModelUsagePreference;
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::{debug, warn};

/// Configuration for the external HTTP policy provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyProviderConfig {
    /// URL of the external policy endpoint.
    pub url: String,
    /// Optional headers to include in requests (e.g., Authorization).
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Cache TTL in seconds. Defaults to 300 (5 minutes).
    #[serde(default = "default_ttl_seconds")]
    pub ttl_seconds: u64,
}

fn default_ttl_seconds() -> u64 {
    300
}

impl Default for PolicyProviderConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            headers: HashMap::new(),
            ttl_seconds: 300,
        }
    }
}

/// Response from the external policy endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyResponse {
    pub policy_id: String,
    pub revision: u64,
    pub schema_version: String,
    pub routing_preferences: Vec<ModelUsagePreference>,
}

/// Cached policy entry with revision and expiration.
#[derive(Debug, Clone)]
struct CachedPolicy {
    policy: Vec<ModelUsagePreference>,
    revision: u64,
    cached_at: Instant,
    ttl: Duration,
}

impl CachedPolicy {
    fn is_expired(&self) -> bool {
        self.cached_at.elapsed() > self.ttl
    }
}

#[derive(Debug, Error)]
pub enum PolicyProviderError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Failed to parse policy response: {0}")]
    Parse(String),

    #[error("Unsupported schema version: {0}")]
    UnsupportedSchemaVersion(String),

    #[error("Policy ID mismatch: expected {expected}, got {actual}")]
    PolicyIdMismatch { expected: String, actual: String },

    #[error("No policy provider configured")]
    NotConfigured,
}

/// External HTTP routing policy provider with caching.
pub struct PolicyProvider {
    config: PolicyProviderConfig,
    client: reqwest::Client,
    cache: RwLock<HashMap<String, CachedPolicy>>,
}

impl PolicyProvider {
    pub fn new(config: PolicyProviderConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// Fetches routing policy for the given policy_id and revision.
    ///
    /// Resolution order:
    /// 1. If cached and cached revision >= requested revision, use cache
    /// 2. Otherwise, fetch from external endpoint
    ///
    /// Returns `None` if no policy_id is provided or if the provider is not configured.
    pub async fn get_policy(
        &self,
        policy_id: &str,
        revision: Option<u64>,
    ) -> Result<Option<Vec<ModelUsagePreference>>, PolicyProviderError> {
        if self.config.url.is_empty() {
            return Err(PolicyProviderError::NotConfigured);
        }

        let revision = revision.unwrap_or(0);

        // Check cache first
        {
            let cache = self.cache.read().await;
            if let Some(cached) = cache.get(policy_id) {
                if !cached.is_expired() && cached.revision >= revision {
                    debug!(
                        policy_id = %policy_id,
                        cached_revision = cached.revision,
                        requested_revision = revision,
                        "using cached policy"
                    );
                    return Ok(Some(cached.policy.clone()));
                }
            }
        }

        // Fetch from external endpoint
        let policy = self.fetch_policy(policy_id, revision).await?;

        // Update cache
        {
            let mut cache = self.cache.write().await;
            cache.insert(
                policy_id.to_string(),
                CachedPolicy {
                    policy: policy.routing_preferences.clone(),
                    revision: policy.revision,
                    cached_at: Instant::now(),
                    ttl: Duration::from_secs(self.config.ttl_seconds),
                },
            );
        }

        debug!(
            policy_id = %policy_id,
            revision = policy.revision,
            num_models = policy.routing_preferences.len(),
            "fetched and cached policy from external endpoint"
        );

        Ok(Some(policy.routing_preferences))
    }

    async fn fetch_policy(
        &self,
        policy_id: &str,
        revision: u64,
    ) -> Result<PolicyResponse, PolicyProviderError> {
        let url = format!(
            "{}?policy_id={}&revision={}",
            self.config.url,
            urlencoding::encode(policy_id),
            revision
        );

        let mut headers = HeaderMap::new();
        for (key, value) in &self.config.headers {
            if let Ok(header_name) = key.parse() {
                if let Ok(header_value) = value.parse() {
                    headers.insert(header_name, header_value);
                }
            }
        }

        debug!(url = %url, "fetching policy from external endpoint");

        let response = self.client.get(&url).headers(headers).send().await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(PolicyProviderError::Parse(format!(
                "HTTP {} from policy endpoint: {}",
                status, body
            )));
        }

        let policy: PolicyResponse = response.json().await.map_err(|e| {
            PolicyProviderError::Parse(format!("Failed to parse JSON response: {}", e))
        })?;

        // Validate schema version
        if policy.schema_version != "v1" {
            return Err(PolicyProviderError::UnsupportedSchemaVersion(
                policy.schema_version,
            ));
        }

        // Validate policy_id matches
        if policy.policy_id != policy_id {
            return Err(PolicyProviderError::PolicyIdMismatch {
                expected: policy_id.to_string(),
                actual: policy.policy_id,
            });
        }

        Ok(policy)
    }

    /// Clears the cache for a specific policy_id or all policies.
    pub async fn clear_cache(&self, policy_id: Option<&str>) {
        let mut cache = self.cache.write().await;
        match policy_id {
            Some(id) => {
                cache.remove(id);
            }
            None => {
                cache.clear();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_ttl() {
        let config = PolicyProviderConfig::default();
        assert_eq!(config.ttl_seconds, 300);
    }

    #[test]
    fn test_cached_policy_expiration() {
        let cached = CachedPolicy {
            policy: vec![],
            revision: 1,
            cached_at: Instant::now() - Duration::from_secs(400),
            ttl: Duration::from_secs(300),
        };
        assert!(cached.is_expired());

        let fresh = CachedPolicy {
            policy: vec![],
            revision: 1,
            cached_at: Instant::now(),
            ttl: Duration::from_secs(300),
        };
        assert!(!fresh.is_expired());
    }
}
