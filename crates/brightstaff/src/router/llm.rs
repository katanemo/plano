use std::{collections::HashMap, sync::Arc, time::Duration};

use common::{
    configuration::TopLevelRoutingPreference,
    consts::{ARCH_PROVIDER_HINT_HEADER, REQUEST_ID_HEADER, TRACE_PARENT_HEADER},
};

use super::router_model::{ModelUsagePreference, RoutingPreference};
use hermesllm::apis::openai::Message;
use hyper::header;
use thiserror::Error;
use tracing::{debug, info};

use super::http::{self, post_and_extract_content};
use super::model_metrics::ModelMetricsService;
use super::router_model::RouterModel;

use crate::router::router_model_v1;
use crate::session_cache::SessionCache;

pub use crate::session_cache::CachedRoute;

const DEFAULT_SESSION_TTL_SECONDS: u64 = 600;

pub struct RouterService {
    router_url: String,
    client: reqwest::Client,
    router_model: Arc<dyn RouterModel>,
    routing_provider_name: String,
    top_level_preferences: HashMap<String, TopLevelRoutingPreference>,
    metrics_service: Option<Arc<ModelMetricsService>>,
    session_cache: Arc<dyn SessionCache>,
    session_ttl: Duration,
}

#[derive(Debug, Error)]
pub enum RoutingError {
    #[error(transparent)]
    Http(#[from] http::HttpError),

    #[error("Router model error: {0}")]
    RouterModelError(#[from] super::router_model::RoutingModelError),
}

pub type Result<T> = std::result::Result<T, RoutingError>;

impl RouterService {
    pub fn new(
        top_level_prefs: Option<Vec<TopLevelRoutingPreference>>,
        metrics_service: Option<Arc<ModelMetricsService>>,
        router_url: String,
        routing_model_name: String,
        routing_provider_name: String,
        session_ttl_seconds: Option<u64>,
        session_cache: Arc<dyn SessionCache>,
    ) -> Self {
        let top_level_preferences: HashMap<String, TopLevelRoutingPreference> = top_level_prefs
            .map_or_else(HashMap::new, |prefs| {
                prefs.into_iter().map(|p| (p.name.clone(), p)).collect()
            });

        // Build sentinel routes for RouterModelV1: route_name → first model.
        // RouterModelV1 uses this to build its prompt; RouterService overrides
        // the model selection via rank_models() after the route is determined.
        let sentinel_routes: HashMap<String, Vec<RoutingPreference>> = top_level_preferences
            .iter()
            .filter_map(|(name, pref)| {
                pref.models.first().map(|first_model| {
                    (
                        first_model.clone(),
                        vec![RoutingPreference {
                            name: name.clone(),
                            description: pref.description.clone(),
                        }],
                    )
                })
            })
            .collect();

        let router_model = Arc::new(router_model_v1::RouterModelV1::new(
            sentinel_routes,
            routing_model_name,
            router_model_v1::MAX_TOKEN_LEN,
        ));

        let session_ttl =
            Duration::from_secs(session_ttl_seconds.unwrap_or(DEFAULT_SESSION_TTL_SECONDS));

        RouterService {
            router_url,
            client: reqwest::Client::new(),
            router_model,
            routing_provider_name,
            top_level_preferences,
            metrics_service,
            session_cache,
            session_ttl,
        }
    }

    /// Look up a cached routing decision by session ID.
    /// Returns None if not found or expired.
    pub async fn get_cached_route(&self, session_id: &str) -> Option<CachedRoute> {
        self.session_cache.get(session_id).await
    }

    /// Store a routing decision in the session cache.
    pub async fn cache_route(
        &self,
        session_id: String,
        model_name: String,
        route_name: Option<String>,
    ) {
        self.session_cache
            .put(
                &session_id,
                CachedRoute {
                    model_name,
                    route_name,
                },
                self.session_ttl,
            )
            .await;
    }

    /// Remove all expired entries from the session cache.
    pub async fn cleanup_expired_sessions(&self) {
        self.session_cache.cleanup_expired().await;
    }

    /// Log a routing decision, used to surface affinity hits in structured logs.
    pub fn log_affinity_hit(session_id: &str, model_name: &str, route_name: &Option<String>) {
        info!(
            session_id = %session_id,
            model = %model_name,
            route = ?route_name,
            "returning pinned routing decision from cache"
        );
    }

    pub async fn determine_route(
        &self,
        messages: &[Message],
        traceparent: &str,
        inline_routing_preferences: Option<Vec<TopLevelRoutingPreference>>,
        request_id: &str,
    ) -> Result<Option<(String, Vec<String>)>> {
        if messages.is_empty() {
            return Ok(None);
        }

        // Build inline top-level map from request if present (inline overrides config).
        let inline_top_map: Option<HashMap<String, TopLevelRoutingPreference>> =
            inline_routing_preferences
                .map(|prefs| prefs.into_iter().map(|p| (p.name.clone(), p)).collect());

        // No routing defined — skip the router call entirely.
        if inline_top_map.is_none() && self.top_level_preferences.is_empty() {
            return Ok(None);
        }

        // For inline overrides, build synthetic ModelUsagePreference list so RouterModelV1
        // generates the correct prompt (route name + description pairs).
        // For config-level prefs the sentinel routes are already baked into RouterModelV1.
        let effective_usage_preferences: Option<Vec<ModelUsagePreference>> =
            inline_top_map.as_ref().map(|inline_map| {
                inline_map
                    .values()
                    .map(|p| ModelUsagePreference {
                        model: p.models.first().cloned().unwrap_or_default(),
                        routing_preferences: vec![RoutingPreference {
                            name: p.name.clone(),
                            description: p.description.clone(),
                        }],
                    })
                    .collect()
            });

        let router_request = self
            .router_model
            .generate_request(messages, &effective_usage_preferences);

        debug!(
            model = %self.router_model.get_model_name(),
            endpoint = %self.router_url,
            "sending request to arch-router"
        );

        let body = serde_json::to_string(&router_request)
            .map_err(super::router_model::RoutingModelError::from)?;
        debug!(body = %body, "arch router request");

        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("application/json"),
        );
        if let Ok(val) = header::HeaderValue::from_str(&self.routing_provider_name) {
            headers.insert(
                header::HeaderName::from_static(ARCH_PROVIDER_HINT_HEADER),
                val,
            );
        }
        if let Ok(val) = header::HeaderValue::from_str(traceparent) {
            headers.insert(header::HeaderName::from_static(TRACE_PARENT_HEADER), val);
        }
        if let Ok(val) = header::HeaderValue::from_str(request_id) {
            headers.insert(header::HeaderName::from_static(REQUEST_ID_HEADER), val);
        }
        headers.insert(
            header::HeaderName::from_static("model"),
            header::HeaderValue::from_static("arch-router"),
        );

        let Some((content, elapsed)) =
            post_and_extract_content(&self.client, &self.router_url, headers, body).await?
        else {
            return Ok(None);
        };

        // Parse the route name from the router response.
        let parsed = self
            .router_model
            .parse_response(&content, &effective_usage_preferences)?;

        let result = if let Some((route_name, _sentinel)) = parsed {
            let top_pref = inline_top_map
                .as_ref()
                .and_then(|m| m.get(&route_name))
                .or_else(|| self.top_level_preferences.get(&route_name));

            if let Some(pref) = top_pref {
                let ranked = match &self.metrics_service {
                    Some(svc) => svc.rank_models(&pref.models, &pref.selection_policy).await,
                    None => pref.models.clone(),
                };
                Some((route_name, ranked))
            } else {
                None
            }
        } else {
            None
        };

        info!(
            content = %content.replace("\n", "\\n"),
            selected_model = ?result,
            response_time_ms = elapsed.as_millis(),
            "arch-router determined route"
        );

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_cache::memory::MemorySessionCache;

    fn make_router_service(ttl_seconds: u64, max_entries: usize) -> RouterService {
        let ttl = Duration::from_secs(ttl_seconds);
        let session_cache = Arc::new(MemorySessionCache::new(ttl, max_entries));
        RouterService::new(
            None,
            None,
            "http://localhost:12001/v1/chat/completions".to_string(),
            "Arch-Router".to_string(),
            "arch-router".to_string(),
            Some(ttl_seconds),
            session_cache,
        )
    }

    #[tokio::test]
    async fn test_cache_miss_returns_none() {
        let svc = make_router_service(600, 100);
        assert!(svc.get_cached_route("unknown-session").await.is_none());
    }

    #[tokio::test]
    async fn test_cache_hit_returns_cached_route() {
        let svc = make_router_service(600, 100);
        svc.cache_route(
            "s1".to_string(),
            "gpt-4o".to_string(),
            Some("code".to_string()),
        )
        .await;

        let cached = svc.get_cached_route("s1").await.unwrap();
        assert_eq!(cached.model_name, "gpt-4o");
        assert_eq!(cached.route_name, Some("code".to_string()));
    }

    #[tokio::test]
    async fn test_cache_expired_entry_returns_none() {
        let svc = make_router_service(0, 100);
        svc.cache_route("s1".to_string(), "gpt-4o".to_string(), None)
            .await;
        assert!(svc.get_cached_route("s1").await.is_none());
    }

    #[tokio::test]
    async fn test_cleanup_removes_expired() {
        let svc = make_router_service(0, 100);
        svc.cache_route("s1".to_string(), "gpt-4o".to_string(), None)
            .await;
        svc.cache_route("s2".to_string(), "claude".to_string(), None)
            .await;

        svc.cleanup_expired_sessions().await;

        // After cleanup, both expired entries should be gone
        assert!(svc.get_cached_route("s1").await.is_none());
        assert!(svc.get_cached_route("s2").await.is_none());
    }

    #[tokio::test]
    async fn test_cache_evicts_oldest_when_full() {
        let svc = make_router_service(600, 2);
        svc.cache_route("s1".to_string(), "model-a".to_string(), None)
            .await;
        tokio::time::sleep(Duration::from_millis(10)).await;
        svc.cache_route("s2".to_string(), "model-b".to_string(), None)
            .await;

        svc.cache_route("s3".to_string(), "model-c".to_string(), None)
            .await;

        // s1 should be evicted (oldest); s2 and s3 should remain
        assert!(svc.get_cached_route("s1").await.is_none());
        assert!(svc.get_cached_route("s2").await.is_some());
        assert!(svc.get_cached_route("s3").await.is_some());
    }

    #[tokio::test]
    async fn test_cache_update_existing_session_does_not_evict() {
        let svc = make_router_service(600, 2);
        svc.cache_route("s1".to_string(), "model-a".to_string(), None)
            .await;
        svc.cache_route("s2".to_string(), "model-b".to_string(), None)
            .await;

        svc.cache_route(
            "s1".to_string(),
            "model-a-updated".to_string(),
            Some("route".to_string()),
        )
        .await;

        // Both sessions should still be present
        let s1 = svc.get_cached_route("s1").await.unwrap();
        assert_eq!(s1.model_name, "model-a-updated");
        assert!(svc.get_cached_route("s2").await.is_some());
    }
}
