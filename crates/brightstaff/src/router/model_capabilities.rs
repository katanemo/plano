//! Tier 1 capability source for routing.
//!
//! Mirrors [`super::model_metrics::ModelMetricsService`] (and how DigitalOcean
//! pricing is handled): nothing is vendored into the binary. The catalog is
//! fetched from models.dev at startup, optionally refreshed on an interval, and
//! left empty on fetch failure (so absent models resolve to the conservative
//! default rather than failing the build with a committed snapshot). Per-model
//! capabilities resolve with the precedence:
//! `user config capabilities > models.dev > conservative default`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use common::configuration::{LlmProvider, ModelCapabilitiesSource};
use hermesllm::providers::capabilities::{
    canonical_model_key, CapabilitiesCatalog, CapabilitiesSnapshot,
};
use hermesllm::ModelCapabilities;
use tokio::sync::RwLock;
use tracing::{info, warn};

const DEFAULT_MODELS_DEV_URL: &str = "https://models.dev/api.json";

pub struct ModelCapabilitiesService {
    /// User config overrides, keyed by canonical `"<provider>/<model>"`.
    user_overrides: HashMap<String, ModelCapabilities>,
    /// models.dev-derived catalog, fetched at runtime (empty until first fetch).
    catalog: Arc<RwLock<CapabilitiesCatalog>>,
}

impl ModelCapabilitiesService {
    /// Build the service from configured providers and an optional capability
    /// source. Fetches the models.dev catalog once at startup (like DO pricing),
    /// then spawns a refresh loop if an interval is configured. On fetch failure
    /// the catalog stays empty and models resolve to user overrides or the
    /// conservative default.
    pub async fn new(
        providers: &[LlmProvider],
        source: Option<&ModelCapabilitiesSource>,
        client: reqwest::Client,
    ) -> Self {
        let mut user_overrides = HashMap::new();
        for p in providers {
            if let Some(caps) = &p.capabilities {
                let model_str = p.model.clone().unwrap_or_else(|| p.name.clone());
                let key = canonical_model_key(&model_str).unwrap_or(model_str);
                user_overrides.insert(key, caps.clone());
            }
        }

        let url = source
            .and_then(|s| s.url.clone())
            .unwrap_or_else(|| DEFAULT_MODELS_DEV_URL.to_string());

        // Fetch once at startup so capabilities are available immediately. On
        // failure the catalog is empty (conservative defaults), never fatal.
        let catalog = match fetch_catalog(&client, &url).await {
            Some(fresh) => {
                info!(models = fresh.len(), url = %url, user_overrides = user_overrides.len(), "fetched model capabilities from models.dev");
                fresh
            }
            None => {
                warn!(url = %url, "models.dev fetch failed at startup — capabilities default to conservative (text-only) until refresh");
                CapabilitiesCatalog::default()
            }
        };
        let catalog = Arc::new(RwLock::new(catalog));

        if let Some(interval_secs) = source.and_then(|s| s.refresh_interval) {
            let catalog_clone = Arc::clone(&catalog);
            let client_clone = client.clone();
            let url_clone = url.clone();
            let interval = Duration::from_secs(interval_secs);
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(interval).await;
                    if let Some(fresh) = fetch_catalog(&client_clone, &url_clone).await {
                        info!(models = fresh.len(), url = %url_clone, "refreshed model capabilities from models.dev");
                        *catalog_clone.write().await = fresh;
                    } else {
                        warn!(url = %url_clone, "models.dev refresh failed — keeping previous catalog");
                    }
                }
            });
        }

        ModelCapabilitiesService {
            user_overrides,
            catalog,
        }
    }

    /// Construct a service from an explicit catalog with no refresh (tests).
    pub fn from_catalog(
        catalog: CapabilitiesCatalog,
        user_overrides: HashMap<String, ModelCapabilities>,
    ) -> Self {
        ModelCapabilitiesService {
            user_overrides,
            catalog: Arc::new(RwLock::new(catalog)),
        }
    }

    /// Resolve capabilities for a model, applying
    /// `user override > models.dev > conservative default`.
    pub async fn capabilities_for(&self, model: &str) -> ModelCapabilities {
        let from_catalog = {
            let catalog = self.catalog.read().await;
            catalog.get(model).cloned().unwrap_or_default()
        };
        let key = canonical_model_key(model);
        let user = key
            .as_ref()
            .and_then(|k| self.user_overrides.get(k))
            .or_else(|| self.user_overrides.get(model));
        match user {
            Some(u) => u.fill_from(&from_catalog),
            None => from_catalog,
        }
    }
}

/// Fetch + map a models.dev catalog. Returns `None` on any network/parse error
/// so the caller can fall back to the previous (seed) catalog.
async fn fetch_catalog(client: &reqwest::Client, url: &str) -> Option<CapabilitiesCatalog> {
    let bytes = match client.get(url).send().await {
        Ok(resp) => match resp.bytes().await {
            Ok(b) => b,
            Err(err) => {
                warn!(error = %err, url = %url, "failed to read models.dev response");
                return None;
            }
        },
        Err(err) => {
            warn!(error = %err, url = %url, "failed to fetch models.dev");
            return None;
        }
    };
    match CapabilitiesSnapshot::from_models_dev_json(&bytes) {
        Ok(snapshot) => Some(CapabilitiesCatalog::from_snapshot(snapshot)),
        Err(err) => {
            warn!(error = %err, url = %url, "failed to parse models.dev payload");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(window: Option<u32>, vision: Option<bool>) -> ModelCapabilities {
        ModelCapabilities {
            context_window: window,
            supports_vision: vision,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn user_override_wins_over_catalog() {
        let mut catalog_models = HashMap::new();
        catalog_models.insert("openai/gpt-4o".to_string(), caps(Some(200000), Some(true)));
        let catalog = CapabilitiesCatalog::new(catalog_models);

        let mut overrides = HashMap::new();
        overrides.insert("openai/gpt-4o".to_string(), caps(Some(128000), None));

        let svc = ModelCapabilitiesService::from_catalog(catalog, overrides);
        let resolved = svc.capabilities_for("openai/gpt-4o").await;
        // user override wins for window, catalog backfills vision
        assert_eq!(resolved.window(), Some(128000));
        assert!(resolved.vision());
    }

    #[tokio::test]
    async fn unknown_model_falls_back_to_conservative_default() {
        let svc =
            ModelCapabilitiesService::from_catalog(CapabilitiesCatalog::default(), HashMap::new());
        let resolved = svc.capabilities_for("openai/unknown").await;
        assert!(!resolved.vision());
        assert_eq!(resolved.window(), None);
    }
}
