use std::{borrow::Cow, collections::HashMap, sync::Arc, time::Duration};

use common::{
    configuration::{
        AgentUsagePreference, OrchestrationPreference, SkillRef, TopLevelRoutingPreference,
    },
    consts::{ARCH_PROVIDER_HINT_HEADER, REQUEST_ID_HEADER},
    skills_runtime::{referenced_skills_catalog, resolve_for_route, resolve_selected_skills},
};
use hermesllm::apis::openai::Message;
use hyper::header;
use opentelemetry::global;
use opentelemetry_http::HeaderInjector;
use thiserror::Error;
use tracing::{debug, info, warn};

use super::http::{self, post_and_extract_content};
use super::model_metrics::ModelMetricsService;
use super::orchestrator_model::{OrchestratorModel, OrchestratorSelection};

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
    /// Agent Skills catalog (deduplicated by name) attached to any
    /// `routing_preferences[].skills` list. Empty when no route has skills.
    skills_catalog: Vec<SkillRef>,
    metrics_service: Option<Arc<ModelMetricsService>>,
    session_cache: Option<Arc<dyn SessionCache>>,
    session_ttl: Duration,
    tenant_header: Option<String>,
}

/// Result of `determine_route`: which route was picked (if any), the
/// ranked candidate models for that route, and the Agent Skill bodies the
/// orchestrator chose to activate alongside it.
///
/// Two valid shapes:
///
/// * **Route + skills (typical):** `route_name = Some(...)`, `models`
///   non-empty, `activated_skills` may be non-empty. Skills are resolved
///   against `routing_preferences[<route>].skills`, so picks that aren't
///   allow-listed for the route are dropped with a `warn!`.
/// * **Skills-only:** `route_name = None`, `models` empty,
///   `activated_skills` non-empty. The orchestrator decided no route
///   needed to change but the user's intent matches one or more skills.
///   Per `docs/source/resources/skills.rst`, the request falls back to the
///   originally-requested model and the skill bodies are injected the
///   same way. Allow-list filtering uses the catalog union (effectively
///   the catalog itself, which is pre-filtered to skills referenced by
///   some route).
#[derive(Debug, Clone, Default)]
pub struct RouteDecision {
    pub route_name: Option<String>,
    pub models: Vec<String>,
    pub activated_skills: Vec<SkillRef>,
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
            skills_catalog: Vec::new(),
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
        Self::with_routing_and_skills(
            orchestrator_url,
            orchestration_model_name,
            orchestrator_provider_name,
            top_level_prefs,
            None,
            metrics_service,
            session_ttl_seconds,
            session_cache,
            tenant_header,
            max_token_length,
        )
    }

    /// Like `with_routing`, but also seeds the orchestrator with a catalog of
    /// Agent Skills referenced by `routing_preferences[].skills`. The
    /// orchestrator gets a `<skills>` block in its system prompt and may
    /// select zero or more skills alongside the picked route; this enables
    /// the LLM handler to inject the chosen SKILL.md bodies into the
    /// upstream request.
    #[allow(clippy::too_many_arguments)]
    pub fn with_routing_and_skills(
        orchestrator_url: String,
        orchestration_model_name: String,
        orchestrator_provider_name: String,
        top_level_prefs: Option<Vec<TopLevelRoutingPreference>>,
        skills_catalog: Option<Vec<SkillRef>>,
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

        let skills_catalog = referenced_skills_catalog(
            skills_catalog.as_deref().unwrap_or(&[]),
            &top_level_preferences,
        );

        let orchestrator_model = Arc::new(orchestrator_model_v1::OrchestratorModelV1::with_skills(
            HashMap::new(),
            skills_catalog.clone(),
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
            skills_catalog,
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
    ) -> Result<Option<RouteDecision>> {
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

        let result = if let Some(ref selection) = orchestration_result {
            if selection.routes.len() > 1 {
                let all_routes: Vec<&str> = selection
                    .routes
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .collect();
                info!(
                    routes = ?all_routes,
                    using = %all_routes.first().unwrap_or(&"none"),
                    "plano-orchestrator detected multiple intents, using first"
                );
            }

            if let Some((route_name, _)) = selection.routes.first() {
                // Route + (optional) skills path.
                let top_pref = inline_top_map
                    .as_ref()
                    .and_then(|m| m.get(route_name))
                    .or_else(|| self.top_level_preferences.get(route_name));

                if let Some(pref) = top_pref {
                    let ranked = match &self.metrics_service {
                        Some(svc) => svc.rank_models(&pref.models, &pref.selection_policy).await,
                        None => pref.models.clone(),
                    };
                    let resolution = resolve_for_route(
                        &self.skills_catalog,
                        pref.skills.as_deref().unwrap_or(&[]),
                        &selection.skills,
                    );
                    log_skill_drops(route_name, &resolution);
                    let activated_skills: Vec<SkillRef> =
                        resolution.activated.into_iter().cloned().collect();
                    Some(RouteDecision {
                        route_name: Some(route_name.clone()),
                        models: ranked,
                        activated_skills,
                    })
                } else {
                    None
                }
            } else if !selection.skills.is_empty() {
                // Skills-only path: orchestrator picked no route but flagged
                // skills. Per the documented contract the request still goes
                // through with the originally-requested model and the skill
                // bodies are injected. The catalog itself is the effective
                // allow-list (it's already the union across every route's
                // allow-list, so anything in it was deemed safe to expose).
                let activated: Vec<SkillRef> =
                    resolve_selected_skills(&self.skills_catalog, &selection.skills)
                        .into_iter()
                        .cloned()
                        .collect();
                if activated.is_empty() {
                    None
                } else {
                    Some(RouteDecision {
                        route_name: None,
                        models: Vec::new(),
                        activated_skills: activated,
                    })
                }
            } else {
                None
            }
        } else {
            None
        };

        info!(
            selected_route = ?result.as_ref().map(|r| (&r.route_name, r.models.first(), r.activated_skills.iter().map(|s| s.name.as_str()).collect::<Vec<_>>())),
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
    ) -> Result<Option<OrchestratorSelection>> {
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

/// Emit `warn!` for any skill names the orchestrator selected but the
/// resolver dropped. Surfacing these is critical for debuggability — a
/// silently-dropped skill is hard to diagnose, and the most common causes
/// (forgetting to add a skill to a route's allow-list, or the orchestrator
/// hallucinating a name) are both fixable once visible.
fn log_skill_drops(route_name: &str, resolution: &common::skills_runtime::SkillResolution<'_>) {
    if !resolution.dropped_not_allowed.is_empty() {
        warn!(
            route = %route_name,
            skills = ?resolution.dropped_not_allowed,
            "orchestrator selected Agent Skills that are not on this route's allow-list; \
             dropping (add them to routing_preferences[].skills if you want this route to use them)"
        );
    }
    if !resolution.dropped_unknown.is_empty() {
        warn!(
            route = %route_name,
            skills = ?resolution.dropped_unknown,
            "orchestrator selected Agent Skills that are not in the runtime catalog \
             (likely hallucinated or removed)"
        );
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

    // ---- RouteDecision construction ----

    fn skill_ref(name: &str) -> SkillRef {
        SkillRef {
            name: name.to_string(),
            description: format!("desc for {name}"),
            path: None,
            base_dir: None,
            body: Some(format!("body for {name}")),
            scope: Some("project".to_string()),
            compatibility: None,
            license: None,
            metadata: None,
            allowed_tools: None,
        }
    }

    #[test]
    fn route_decision_holds_optional_route_name_for_skills_only_path() {
        // Regression guard for the docs promise at skills.rst:153-155: a
        // skills-only decision must be representable, with no route_name and
        // empty models, so the LLM handler falls back to the original model.
        let decision = RouteDecision {
            route_name: None,
            models: Vec::new(),
            activated_skills: vec![skill_ref("pdf")],
        };
        assert!(decision.route_name.is_none());
        assert!(decision.models.is_empty());
        assert_eq!(decision.activated_skills.len(), 1);
    }

    #[test]
    fn log_skill_drops_does_not_panic_on_empty_resolution() {
        // The logger is fire-and-forget. We can't easily assert on the
        // emitted warns here without setting up a tracing subscriber, so the
        // contract under test is: empty resolutions are silent (no warn
        // attempt). Confidence in the warn paths comes from
        // common::skills_runtime tests for resolve_for_route, which is the
        // function whose dropped_* lists drive this logger.
        let empty = common::skills_runtime::SkillResolution::default();
        log_skill_drops("any", &empty);
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
