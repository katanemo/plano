use std::{borrow::Cow, collections::HashMap, sync::Arc, time::Duration};

use common::{
    configuration::{
        AgentUsagePreference, EmptyPoolBehavior, OrchestrationPreference, TopLevelRoutingPreference,
    },
    consts::{ARCH_PROVIDER_HINT_HEADER, REQUEST_ID_HEADER},
};
use hermesllm::apis::openai::Message;
use hermesllm::RequiredCapabilities;
use hyper::header;
use opentelemetry::global;
use opentelemetry_http::HeaderInjector;
use thiserror::Error;
use tracing::{debug, info, warn};

use super::http::{self, post_and_extract_content};
use super::model_capabilities::ModelCapabilitiesService;
use super::model_metrics::ModelMetricsService;
use super::orchestrator_model::OrchestratorModel;

use crate::metrics as bs_metrics;
use crate::metrics::labels as metric_labels;
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
    capabilities_service: Option<Arc<ModelCapabilitiesService>>,
    empty_pool_behavior: EmptyPoolBehavior,
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

    /// Tier 1 capability filtering removed every candidate from the matched
    /// route and `empty_pool_behavior` is `error` (D3).
    #[error("no capable model for route '{route}': request requires {requirement}")]
    CapabilityFilterEmpty { route: String, requirement: String },
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
            capabilities_service: None,
            empty_pool_behavior: EmptyPoolBehavior::default(),
            session_cache: None,
            session_ttl: Duration::from_secs(DEFAULT_SESSION_TTL_SECONDS),
            tenant_header: None,
        }
    }

    /// Attach the Tier 1 capability filter and the empty-pool policy (D3).
    /// Builder-style so existing constructor call sites stay unchanged.
    #[must_use]
    pub fn with_capability_filter(
        mut self,
        capabilities_service: Option<Arc<ModelCapabilitiesService>>,
        empty_pool_behavior: EmptyPoolBehavior,
    ) -> Self {
        self.capabilities_service = capabilities_service;
        self.empty_pool_behavior = empty_pool_behavior;
        self
    }

    /// Whether a single model can serve a request with the given required
    /// capabilities (used for the no-preference-match validation path).
    pub async fn is_model_capable(&self, model: &str, required: &RequiredCapabilities) -> bool {
        match &self.capabilities_service {
            Some(svc) if !required.is_unconstrained() => {
                required.satisfied_by(&svc.capabilities_for(model).await)
            }
            _ => true,
        }
    }

    pub fn empty_pool_behavior(&self) -> EmptyPoolBehavior {
        self.empty_pool_behavior
    }

    pub fn has_capability_filter(&self) -> bool {
        self.capabilities_service.is_some()
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
            capabilities_service: None,
            empty_pool_behavior: EmptyPoolBehavior::default(),
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
        let result = cache.get(&Self::session_key(tenant_id, session_id)).await;
        bs_metrics::record_session_cache_event(if result.is_some() {
            metric_labels::SESSION_CACHE_HIT
        } else {
            metric_labels::SESSION_CACHE_MISS
        });
        result
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
            bs_metrics::record_session_cache_event(metric_labels::SESSION_CACHE_STORE);
        }
    }

    // ---- LLM routing ----

    pub async fn determine_route(
        &self,
        messages: &[Message],
        inline_routing_preferences: Option<Vec<TopLevelRoutingPreference>>,
        request_id: &str,
        required: &RequiredCapabilities,
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
                    // Tier 1: hard capability filter (intersection) before ranking.
                    let effective_models = self
                        .apply_capability_filter(route_name, &pref.models, required)
                        .await?;

                    // Tier 2: rank the surviving capable pool (preserves order for `none`).
                    let ranked = match &self.metrics_service {
                        Some(svc) => {
                            svc.rank_models(&effective_models, &pref.selection_policy)
                                .await
                        }
                        None => effective_models.clone(),
                    };
                    info!(
                        route = %route_name,
                        tier = "tier2",
                        selection_policy = ?pref.selection_policy,
                        capable_pool = effective_models.len(),
                        selected = %ranked.first().map(|s| s.as_str()).unwrap_or(""),
                        "Tier 2 preference ranking applied"
                    );
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

    /// Tier 1: intersect a route's model pool with the models capable of serving
    /// the request shape, preserving preference order among survivors. When the
    /// intersection is empty, honor `empty_pool_behavior` (D3): `error` returns a
    /// typed error; `warning` logs and proceeds with the pre-filter pool.
    async fn apply_capability_filter(
        &self,
        route_name: &str,
        models: &[String],
        required: &RequiredCapabilities,
    ) -> Result<Vec<String>> {
        let Some(svc) = &self.capabilities_service else {
            return Ok(models.to_vec());
        };
        if required.is_unconstrained() {
            return Ok(models.to_vec());
        }

        let started = std::time::Instant::now();
        let mut capable = Vec::new();
        for m in models {
            let caps = svc.capabilities_for(m).await;
            if required.satisfied_by(&caps) {
                capable.push(m.clone());
            }
        }
        let elapsed = started.elapsed();

        if capable.is_empty() {
            match self.empty_pool_behavior {
                EmptyPoolBehavior::Error => {
                    bs_metrics::record_capability_filter(
                        route_name,
                        metric_labels::CAPABILITY_FILTER_EMPTY_ERROR,
                        models.len(),
                        0,
                        elapsed,
                    );
                    return Err(OrchestrationError::CapabilityFilterEmpty {
                        route: route_name.to_string(),
                        requirement: required.describe(),
                    });
                }
                EmptyPoolBehavior::Warning => {
                    bs_metrics::record_capability_filter(
                        route_name,
                        metric_labels::CAPABILITY_FILTER_EMPTY_WARNING,
                        models.len(),
                        0,
                        elapsed,
                    );
                    warn!(
                        route = %route_name,
                        requirement = %required.describe(),
                        "Tier 1 capability filter emptied the pool; empty_pool_behavior=warning, proceeding with pre-filter pool"
                    );
                    return Ok(models.to_vec());
                }
            }
        }

        let outcome = if capable.len() == models.len() {
            metric_labels::CAPABILITY_FILTER_PASS
        } else {
            metric_labels::CAPABILITY_FILTER_FILTERED
        };
        bs_metrics::record_capability_filter(
            route_name,
            outcome,
            models.len(),
            capable.len(),
            elapsed,
        );
        info!(
            route = %route_name,
            tier = "tier1",
            pre_filter = models.len(),
            post_filter = capable.len(),
            requirement = %required.describe(),
            filter_us = elapsed.as_micros() as u64,
            "Tier 1 capability filter applied"
        );
        Ok(capable)
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
