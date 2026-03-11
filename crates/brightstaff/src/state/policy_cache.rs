use common::configuration::ModelUsagePreference;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

#[derive(Clone)]
struct CachedPolicy {
    preferences: Vec<ModelUsagePreference>,
    expires_at: Instant,
}

pub struct PolicyCache {
    entries: RwLock<HashMap<String, CachedPolicy>>,
}

impl Default for PolicyCache {
    fn default() -> Self {
        Self::new()
    }
}

impl PolicyCache {
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
        }
    }

    pub async fn get_valid(&self, policy_id: &str) -> Option<Vec<ModelUsagePreference>> {
        let now = Instant::now();
        let cached = {
            let entries = self.entries.read().await;
            entries.get(policy_id).cloned()
        };

        let cached = cached?;
        if cached.expires_at > now {
            return Some(cached.preferences);
        }

        self.entries.write().await.remove(policy_id);
        None
    }

    pub async fn insert(
        &self,
        policy_id: String,
        preferences: Vec<ModelUsagePreference>,
        ttl: Duration,
    ) {
        let expires_at = Instant::now() + ttl;
        self.entries.write().await.insert(
            policy_id,
            CachedPolicy {
                preferences,
                expires_at,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::PolicyCache;
    use common::configuration::{ModelUsagePreference, RoutingPreference};
    use std::time::Duration;

    fn sample_preferences() -> Vec<ModelUsagePreference> {
        vec![ModelUsagePreference {
            model: "openai/gpt-4o".to_string(),
            routing_preferences: vec![RoutingPreference {
                name: "quick response".to_string(),
                description: "fast lightweight responses".to_string(),
            }],
        }]
    }

    #[tokio::test]
    async fn returns_cached_policy_before_expiry() {
        let cache = PolicyCache::new();
        cache
            .insert(
                "customer-a".to_string(),
                sample_preferences(),
                Duration::from_secs(10),
            )
            .await;

        let cached = cache.get_valid("customer-a").await;
        assert!(cached.is_some());
        assert_eq!(cached.unwrap()[0].model, "openai/gpt-4o");
    }

    #[tokio::test]
    async fn expires_cached_policy_after_ttl() {
        let cache = PolicyCache::new();
        cache
            .insert(
                "customer-a".to_string(),
                sample_preferences(),
                Duration::from_millis(5),
            )
            .await;

        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(cache.get_valid("customer-a").await.is_none());
    }
}
