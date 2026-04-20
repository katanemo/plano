use std::{borrow::Cow, collections::HashMap, sync::Arc, time::Duration};

use common::{
    configuration::{AgentUsagePreference, OrchestrationPreference, TopLevelRoutingPreference},
    consts::{ARCH_PROVIDER_HINT_HEADER, REQUEST_ID_HEADER},
};
use hermesllm::apis::openai::Message;
use hyper::header;
use opentelemetry::global;
use opentelemetry_http::HeaderInjector;
use thiserror::Error;
use tracing::{debug, info};

use super::http::{self, post_and_extract_content};
use super::model_metrics::ModelMetricsService;
use super::orchestrator_model::OrchestratorModel;

use crate::router::orchestrator_model_v1;
use crate::session_cache::SessionCache;

pub use crate::session_cache::CachedRoute;

const DEFAULT_SESSION_TTL_SECONDS: u64 = 600;

pub struct OrchestratorService {
    orchestrator_url: String,
    client: reqwest::Client,
    orchestrator_model: Arc<dyn OrchestratorModel>,
    orchestrator_provider_name: String,
    top_level_preferences: HashMap<String, TopLevelRoutingPreference>,
    metrics_service: Option<Arc<ModelMetricsService>>,
    session_cache: Option<Arc<dyn SessionCache>>,
    session_ttl: Duration,
    tenant_header: Option<String>,
}

#[derive(Debug, Error)]
pub enum OrchestrationError {
    #[error(transparent)]
    Http(#[from] http::HttpError),

    #[error("Orchestrator model error: {0}")]
    OrchestratorModelError(#[from] super::orchestrator_model::OrchestratorModelError),
}

pub type Result<T> = std::result::Result<T, OrchestrationError>;

impl OrchestratorService {
    pub fn new(
        orchestrator_url: String,
        orchestration_model_name: String,
        orchestrator_provider_name: String,
        max_token_length: usize,
    ) -> Self {
        let orchestrator_model = Arc::new(orchestrator_model_v1::OrchestratorModelV1::new(
            HashMap::new(),
            orchestration_model_name,
            max_token_length,
        ));

        OrchestratorService {
            orchestrator_url,
            client: reqwest::Client::new(),
            orchestrator_model,
            orchestrator_provider_name,
            top_level_preferences: HashMap::new(),
            metrics_service: None,
            session_cache: None,
            session_ttl: Duration::from_secs(DEFAULT_SESSION_TTL_SECONDS),
            tenant_header: None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_routing(
        orchestrator_url: String,
        orchestration_model_name: String,
        orchestrator_provider_name: String,
        top_level_prefs: Option<Vec<TopLevelRoutingPreference>>,
        metrics_service: Option<Arc<ModelMetricsService>>,
        session_ttl_seconds: Option<u64>,
        session_cache: Arc<dyn SessionCache>,
        tenant_header: Option<String>,
        max_token_length: usize,
    ) -> Self {
        let top_level_preferences: HashMap<String, TopLevelRoutingPreference> = top_level_prefs
            .map_or_else(HashMap::new, |prefs| {
                prefs.into_iter().map(|p| (p.name.clone(), p)).collect()
            });

        let orchestrator_model = Arc::new(orchestrator_model_v1::OrchestratorModelV1::new(
            HashMap::new(),
            orchestration_model_name,
            max_token_length,
        ));

        let session_ttl =
            Duration::from_secs(session_ttl_seconds.unwrap_or(DEFAULT_SESSION_TTL_SECONDS));

        OrchestratorService {
            orchestrator_url,
            client: reqwest::Client::new(),
            orchestrator_model,
            orchestrator_provider_name,
            top_level_preferences,
            metrics_service,
            session_cache: Some(session_cache),
            session_ttl,
            tenant_header,
        }
    }

    // ---- Session cache methods ----

    #[must_use]
    pub fn tenant_header(&self) -> Option<&str> {
        self.tenant_header.as_deref()
    }

    fn session_key<'a>(tenant_id: Option<&str>, session_id: &'a str) -> Cow<'a, str> {
        match tenant_id {
            Some(t) => Cow::Owned(format!("{t}:{session_id}")),
            None => Cow::Borrowed(session_id),
        }
    }

    pub async fn get_cached_route(
        &self,
        session_id: &str,
        tenant_id: Option<&str>,
    ) -> Option<CachedRoute> {
        let cache = self.session_cache.as_ref()?;
        cache.get(&Self::session_key(tenant_id, session_id)).await
    }

    pub async fn cache_route(
        &self,
        session_id: String,
        tenant_id: Option<&str>,
        model_name: String,
        route_name: Option<String>,
    ) {
        if let Some(ref cache) = self.session_cache {
            cache
                .put(
                    &Self::session_key(tenant_id, &session_id),
                    CachedRoute {
                        model_name,
                        route_name,
                    },
                    self.session_ttl,
                )
                .await;
        }
    }

    // ---- LLM routing ----

    pub async fn determine_route(
        &self,
        messages: &[Message],
        inline_routing_preferences: Option<Vec<TopLevelRoutingPreference>>,
        request_id: &str,
    ) -> Result<Option<(String, Vec<String>)>> {
        if messages.is_empty() {
            return Ok(None);
        }

        let inline_top_map: Option<HashMap<String, TopLevelRoutingPreference>> =
            inline_routing_preferences
                .map(|prefs| prefs.into_iter().map(|p| (p.name.clone(), p)).collect());

        if inline_top_map.is_none() && self.top_level_preferences.is_empty() {
            return Ok(None);
        }

        let effective_source = inline_top_map
            .as_ref()
            .unwrap_or(&self.top_level_preferences);

        let effective_prefs: Vec<AgentUsagePreference> = effective_source
            .values()
            .map(|p| AgentUsagePreference {
                model: p.models.first().cloned().unwrap_or_default(),
                orchestration_preferences: vec![OrchestrationPreference {
                    name: p.name.clone(),
                    description: p.description.clone(),
                }],
            })
            .collect();

        let orchestration_result = self
            .determine_orchestration(
                messages,
                Some(effective_prefs),
                Some(request_id.to_string()),
            )
            .await?;

        let result = if let Some(ref routes) = orchestration_result {
            if routes.len() > 1 {
                let all_routes: Vec<&str> = routes.iter().map(|(name, _)| name.as_str()).collect();
                info!(
                    routes = ?all_routes,
                    using = %all_routes.first().unwrap_or(&"none"),
                    "plano-orchestrator detected multiple intents, using first"
                );
            }

            if let Some((route_name, _)) = routes.first() {
                let top_pref = inline_top_map
                    .as_ref()
                    .and_then(|m| m.get(route_name))
                    .or_else(|| self.top_level_preferences.get(route_name));

                if let Some(pref) = top_pref {
                    let ranked = match &self.metrics_service {
                        Some(svc) => svc.rank_models(&pref.models, &pref.selection_policy).await,
                        None => pref.models.clone(),
                    };
                    Some((route_name.clone(), ranked))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        info!(
            selected_model = ?result,
            "plano-orchestrator determined route"
        );

        Ok(result)
    }

    // ---- Agent orchestration (existing) ----

    pub async fn determine_orchestration(
        &self,
        messages: &[Message],
        usage_preferences: Option<Vec<AgentUsagePreference>>,
        request_id: Option<String>,
    ) -> Result<Option<Vec<(String, String)>>> {
        if messages.is_empty() {
            return Ok(None);
        }

        if usage_preferences
            .as_ref()
            .is_none_or(|prefs| prefs.is_empty())
        {
            return Ok(None);
        }

        let orchestrator_request = self
            .orchestrator_model
            .generate_request(messages, &usage_preferences);

        debug!(
            model = %self.orchestrator_model.get_model_name(),
            endpoint = %self.orchestrator_url,
            "sending request to plano-orchestrator"
        );

        let body = serde_json::to_string(&orchestrator_request)
            .map_err(super::orchestrator_model::OrchestratorModelError::from)?;
        debug!(body = %body, "plano-orchestrator request");

        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("application/json"),
        );
        headers.insert(
            header::HeaderName::from_static(ARCH_PROVIDER_HINT_HEADER),
            header::HeaderValue::from_str(&self.orchestrator_provider_name)
                .unwrap_or_else(|_| header::HeaderValue::from_static("plano-orchestrator")),
        );

        global::get_text_map_propagator(|propagator| {
            let cx =
                tracing_opentelemetry::OpenTelemetrySpanExt::context(&tracing::Span::current());
            propagator.inject_context(&cx, &mut HeaderInjector(&mut headers));
        });

        if let Some(ref request_id) = request_id {
            if let Ok(val) = header::HeaderValue::from_str(request_id) {
                headers.insert(header::HeaderName::from_static(REQUEST_ID_HEADER), val);
            }
        }

        headers.insert(
            header::HeaderName::from_static("model"),
            header::HeaderValue::from_static("plano-orchestrator"),
        );

        let Some((content, elapsed)) =
            post_and_extract_content(&self.client, &self.orchestrator_url, headers, body).await?
        else {
            return Ok(None);
        };

        let parsed = self
            .orchestrator_model
            .parse_response(&content, &usage_preferences)?;

        info!(
            content = %content.replace("\n", "\\n"),
            selected_routes = ?parsed,
            response_time_ms = elapsed.as_millis(),
            "plano-orchestrator determined routes"
        );

        Ok(parsed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_cache::memory::MemorySessionCache;

    fn make_orchestrator_service(ttl_seconds: u64, max_entries: usize) -> OrchestratorService {
        let session_cache = Arc::new(MemorySessionCache::new(max_entries));
        OrchestratorService::with_routing(
            "http://localhost:12001/v1/chat/completions".to_string(),
            "Plano-Orchestrator".to_string(),
            "plano-orchestrator".to_string(),
            None,
            None,
            Some(ttl_seconds),
            session_cache,
            None,
            orchestrator_model_v1::MAX_TOKEN_LEN,
        )
    }

    #[tokio::test]
    async fn test_cache_miss_returns_none() {
        let svc = make_orchestrator_service(600, 100);
        assert!(svc
            .get_cached_route("unknown-session", None)
            .await
            .is_none());
    }

    #[tokio::test]
    async fn test_cache_hit_returns_cached_route() {
        let svc = make_orchestrator_service(600, 100);
        svc.cache_route(
            "s1".to_string(),
            None,
            "gpt-4o".to_string(),
            Some("code".to_string()),
        )
        .await;

        let cached = svc.get_cached_route("s1", None).await.unwrap();
        assert_eq!(cached.model_name, "gpt-4o");
        assert_eq!(cached.route_name, Some("code".to_string()));
    }

    #[tokio::test]
    async fn test_cache_expired_entry_returns_none() {
        let svc = make_orchestrator_service(0, 100);
        svc.cache_route("s1".to_string(), None, "gpt-4o".to_string(), None)
            .await;
        assert!(svc.get_cached_route("s1", None).await.is_none());
    }

    #[tokio::test]
    async fn test_expired_entries_not_returned() {
        let svc = make_orchestrator_service(0, 100);
        svc.cache_route("s1".to_string(), None, "gpt-4o".to_string(), None)
            .await;
        svc.cache_route("s2".to_string(), None, "claude".to_string(), None)
            .await;

        assert!(svc.get_cached_route("s1", None).await.is_none());
        assert!(svc.get_cached_route("s2", None).await.is_none());
    }

    #[tokio::test]
    async fn test_cache_evicts_oldest_when_full() {
        let svc = make_orchestrator_service(600, 2);
        svc.cache_route("s1".to_string(), None, "model-a".to_string(), None)
            .await;
        tokio::time::sleep(Duration::from_millis(10)).await;
        svc.cache_route("s2".to_string(), None, "model-b".to_string(), None)
            .await;

        svc.cache_route("s3".to_string(), None, "model-c".to_string(), None)
            .await;

        assert!(svc.get_cached_route("s1", None).await.is_none());
        assert!(svc.get_cached_route("s2", None).await.is_some());
        assert!(svc.get_cached_route("s3", None).await.is_some());
    }

    #[tokio::test]
    async fn test_cache_update_existing_session_does_not_evict() {
        let svc = make_orchestrator_service(600, 2);
        svc.cache_route("s1".to_string(), None, "model-a".to_string(), None)
            .await;
        svc.cache_route("s2".to_string(), None, "model-b".to_string(), None)
            .await;

        svc.cache_route(
            "s1".to_string(),
            None,
            "model-a-updated".to_string(),
            Some("route".to_string()),
        )
        .await;

        let s1 = svc.get_cached_route("s1", None).await.unwrap();
        assert_eq!(s1.model_name, "model-a-updated");
        assert!(svc.get_cached_route("s2", None).await.is_some());
    }
}
