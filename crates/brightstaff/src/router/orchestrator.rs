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

use crate::metrics as bs_metrics;
use crate::metrics::labels as metric_labels;
use crate::router::orchestrator_model_v1;
use crate::session_cache::SessionCache;

pub use crate::session_cache::SessionBinding;

const DEFAULT_SESSION_TTL_SECONDS: u64 = 600;
const TOKENS_PER_MILLION: f64 = 1_000_000.0;

/// Input-cost of abandoning a plausibly-warm cache to switch to `candidate`.
///
/// Deliberately input-only: output-token savings are unknowable before the response
/// is generated (reasoning models can emit 5-10x the tokens for the same task), so they
/// are never credited. Negative when the candidate's uncached input rate undercuts the
/// anchor's cached rate — those switches are outright cheaper. Rates are USD per million
/// tokens; the caller draws a positive cost down from the session's switch budget.
pub fn switch_cost_in_usd(
    context_tokens: u64,
    anchor_cached_rate: f64,
    candidate_uncached_rate: f64,
) -> f64 {
    let context_millions = context_tokens as f64 / TOKENS_PER_MILLION;
    context_millions * (candidate_uncached_rate - anchor_cached_rate)
}

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

    /// Look up a session binding. Warmth is the caller's concern (time since
    /// `last_used`); this only reports whether a binding exists.
    pub async fn get_binding(
        &self,
        session_id: &str,
        tenant_id: Option<&str>,
    ) -> Option<SessionBinding> {
        let cache = self.session_cache.as_ref()?;
        let result = cache.get(&Self::session_key(tenant_id, session_id)).await;
        bs_metrics::record_session_cache_event(match result {
            Some(_) => metric_labels::SESSION_CACHE_HIT,
            None => metric_labels::SESSION_CACHE_MISS,
        });
        result
    }

    /// The GC bound for a session binding: the per-scope override when provided,
    /// otherwise the global `routing.session_ttl_seconds`. This only governs when an
    /// idle binding is reclaimed from memory — not whether its cache is warm.
    pub fn effective_session_ttl(&self, ttl_override_seconds: Option<u64>) -> Duration {
        ttl_override_seconds
            .map(Duration::from_secs)
            .unwrap_or(self.session_ttl)
    }

    /// Persist a session binding with a GC bound (defaults to `routing.session_ttl_seconds`
    /// when `gc_ttl` is `None`).
    pub async fn store_binding(
        &self,
        session_id: &str,
        tenant_id: Option<&str>,
        binding: SessionBinding,
        gc_ttl: Option<Duration>,
    ) {
        if let Some(ref cache) = self.session_cache {
            cache
                .put(
                    &Self::session_key(tenant_id, session_id),
                    binding,
                    gc_ttl.unwrap_or(self.session_ttl),
                )
                .await;
            bs_metrics::record_session_cache_event(metric_labels::SESSION_CACHE_STORE);
        }
    }

    /// Structured per-million pricing for a model, from the configured cost feed.
    /// `None` when no cost source is configured or the model is unknown to the feed.
    pub async fn model_rates(&self, model: &str) -> Option<super::model_metrics::ModelRates> {
        self.metrics_service.as_ref()?.model_rates(model).await
    }

    /// Estimate the input-cost (USD) of switching a warm session from `anchor_model`
    /// (the model that handled the latest request, i.e. the one the session is currently
    /// warm on) to `candidate_model`. Fetches per-model rates from the configured cost
    /// feed; returns `None` when pricing is missing for either side so the caller can
    /// fail open (switch freely) rather than veto the router on guesswork.
    /// `cache_read_discount` estimates the anchor's cached-read rate when the feed
    /// doesn't publish one. Negative when the switch is outright cheaper.
    pub async fn estimate_switch_cost_in_usd(
        &self,
        context_tokens: u64,
        anchor_model: &str,
        candidate_model: &str,
        cache_read_discount: f64,
    ) -> Option<f64> {
        let anchor = self.model_rates(anchor_model).await?;
        let candidate = self.model_rates(candidate_model).await?;
        Some(switch_cost_in_usd(
            context_tokens,
            anchor.cached_input_rate(cache_read_discount),
            candidate.input_per_million,
        ))
    }

    /// This turn's contribution to the session's *never-switch* baseline: the USD cost of
    /// reading `context_tokens` at `model`'s cached input rate. `model` is the session's
    /// `default_model` — what it would have paid by never switching — not the (possibly
    /// drifted) current anchor. Summed across turns, this is the denominator the
    /// percentage overhead cap is measured against. `None` when the model has no pricing
    /// (the caller then can't grow the baseline this turn).
    pub async fn cached_read_cost_in_usd(
        &self,
        context_tokens: u64,
        model: &str,
        cache_read_discount: f64,
    ) -> Option<f64> {
        let rates = self.model_rates(model).await?;
        let context_millions = context_tokens as f64 / TOKENS_PER_MILLION;
        Some(context_millions * rates.cached_input_rate(cache_read_discount))
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

    fn binding(model: &str, route_name: Option<&str>) -> SessionBinding {
        SessionBinding {
            anchor_model: model.to_string(),
            default_model: model.to_string(),
            route_name: route_name.map(|r| r.to_string()),
            prefix_hash: None,
            last_used: std::time::SystemTime::now(),
            cached_tokens: 0,
            baseline_usd: 0.0,
            switch_spend_usd: 0.0,
            switches: 0,
            session_cost_usd: 0.0,
        }
    }

    #[tokio::test]
    async fn test_cache_miss_returns_none() {
        let svc = make_orchestrator_service(600, 100);
        assert!(svc.get_binding("unknown-session", None).await.is_none());
    }

    #[tokio::test]
    async fn test_cache_hit_returns_binding() {
        let svc = make_orchestrator_service(600, 100);
        svc.store_binding("s1", None, binding("gpt-4o", Some("code")), None)
            .await;

        let cached = svc.get_binding("s1", None).await.unwrap();
        assert_eq!(cached.anchor_model, "gpt-4o");
        assert_eq!(cached.route_name, Some("code".to_string()));
    }

    #[tokio::test]
    async fn test_cache_expired_entry_returns_none() {
        let svc = make_orchestrator_service(0, 100);
        svc.store_binding("s1", None, binding("gpt-4o", None), None)
            .await;
        assert!(svc.get_binding("s1", None).await.is_none());
    }

    #[tokio::test]
    async fn test_expired_entries_not_returned() {
        let svc = make_orchestrator_service(0, 100);
        svc.store_binding("s1", None, binding("gpt-4o", None), None)
            .await;
        svc.store_binding("s2", None, binding("claude", None), None)
            .await;

        assert!(svc.get_binding("s1", None).await.is_none());
        assert!(svc.get_binding("s2", None).await.is_none());
    }

    #[tokio::test]
    async fn test_cache_evicts_oldest_when_full() {
        let svc = make_orchestrator_service(600, 2);
        svc.store_binding("s1", None, binding("model-a", None), None)
            .await;
        tokio::time::sleep(Duration::from_millis(10)).await;
        svc.store_binding("s2", None, binding("model-b", None), None)
            .await;

        svc.store_binding("s3", None, binding("model-c", None), None)
            .await;

        assert!(svc.get_binding("s1", None).await.is_none());
        assert!(svc.get_binding("s2", None).await.is_some());
        assert!(svc.get_binding("s3", None).await.is_some());
    }

    #[tokio::test]
    async fn test_cache_update_existing_session_does_not_evict() {
        let svc = make_orchestrator_service(600, 2);
        svc.store_binding("s1", None, binding("model-a", None), None)
            .await;
        svc.store_binding("s2", None, binding("model-b", None), None)
            .await;

        svc.store_binding("s1", None, binding("model-a-updated", Some("route")), None)
            .await;

        let s1 = svc.get_binding("s1", None).await.unwrap();
        assert_eq!(s1.anchor_model, "model-a-updated");
        assert!(svc.get_binding("s2", None).await.is_some());
    }

    #[tokio::test]
    async fn test_gc_ttl_override_extends_binding_lifetime() {
        // Global GC bound of 0 would reclaim immediately; the per-call override keeps it.
        let svc = make_orchestrator_service(0, 100);
        svc.store_binding(
            "s1",
            None,
            binding("gpt-4o", None),
            Some(Duration::from_secs(600)),
        )
        .await;
        let cached = svc.get_binding("s1", None).await.unwrap();
        assert_eq!(cached.anchor_model, "gpt-4o");
    }

    #[tokio::test]
    async fn test_binding_fields_round_trip_through_cache() {
        let svc = make_orchestrator_service(600, 100);
        let mut b = binding("gpt-4o", None);
        b.prefix_hash = Some(0xdead_beef);
        b.cached_tokens = 12_345;
        b.baseline_usd = 1.5;
        b.switch_spend_usd = 0.42;
        b.switches = 3;
        b.session_cost_usd = 2.75;
        svc.store_binding("s1", None, b, None).await;

        let cached = svc.get_binding("s1", None).await.unwrap();
        assert_eq!(cached.prefix_hash, Some(0xdead_beef));
        assert_eq!(cached.cached_tokens, 12_345);
        assert!((cached.baseline_usd - 1.5).abs() < 1e-9);
        assert!((cached.switch_spend_usd - 0.42).abs() < 1e-9);
        assert_eq!(cached.switches, 3);
        assert!((cached.session_cost_usd - 2.75).abs() < 1e-9);
    }

    // ---- switch-cost math ----
    //
    // Real models.dev rates (USD per million input tokens):
    //   claude-opus-4-1:   input 15,  cache_read 1.5
    //   claude-sonnet-4-5: input 3,   cache_read 0.3
    //   claude-haiku-4-5:  input 1,   cache_read 0.1
    //   gpt-4.1:           input 2,   cache_read 0.5

    #[test]
    fn negative_cost_when_candidate_undercuts_cached_rate() {
        // Anchor opus (cached 1.5) -> haiku (uncached 1.0) over 100k context:
        // cost = 0.1M x (1.0 - 1.5) = -$0.05 — cheaper even after re-reading.
        let cost = switch_cost_in_usd(100_000, 1.5, 1.0);
        assert!((cost - (-0.05)).abs() < 1e-9);
    }

    #[test]
    fn positive_cost_when_candidate_pricier_than_cached_rate() {
        // Anchor opus (cached 1.5) -> gpt-4.1 (uncached 2.0) over 100k:
        // cost = 0.1M x (2.0 - 1.5) = +$0.05.
        let cost = switch_cost_in_usd(100_000, 1.5, 2.0);
        assert!((cost - 0.05).abs() < 1e-9);
    }

    #[test]
    fn large_context_amplifies_cost() {
        // Anchor sonnet (cached 0.3) -> gpt-5.5-class (uncached 5.0) over 150k:
        // cost = 0.15M x (5.0 - 0.3) = +$0.705.
        let cost = switch_cost_in_usd(150_000, 0.3, 5.0);
        assert!((cost - 0.705).abs() < 1e-9);
    }

    #[test]
    fn cost_scales_linearly_with_context() {
        let small = switch_cost_in_usd(10_000, 0.3, 0.8);
        let large = switch_cost_in_usd(1_000_000, 0.3, 0.8);
        assert!((large / small - 100.0).abs() < 1e-6);
    }

    #[test]
    fn tiny_context_cost_is_negligible() {
        // 2k-token chat: even an expensive candidate costs ~$0.009.
        let cost = switch_cost_in_usd(2_000, 0.3, 5.0);
        assert!(cost < 0.01);
    }
}
