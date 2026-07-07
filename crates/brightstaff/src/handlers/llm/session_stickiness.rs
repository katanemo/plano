//! Session-stickiness handling for the LLM path.
//!
//! This module owns the "keep a conversation on the same model" concern, grouped in
//! one place for readability:
//!
//! * [`resolve_session`] — derive the session key (explicit header > implicit prompt
//!   prefix hash) and the prefix hash used for cluster-level replica stickiness.
//! * [`lookup_session_pin`] — read the pin, classifying it as usable, a stale hint, or
//!   drifted (prefix changed → provider cache already cold).
//! * [`apply_cache_regret_gate`] — when a re-route is being considered, veto a switch
//!   whose input-cost regret exceeds the developer's threshold (the cost math lives in
//!   [`crate::router::cache_regret`]).
//! * [`plan_pin_after_routing`] — decide the response-side pin action.
//!
//! Prompt-cache *marker injection* is a separate concern — see [`super::prompt_caching`].

use std::sync::Arc;

use common::configuration::{EffectivePromptCaching, EffectiveSessionStickiness};
use hermesllm::apis::openai::Message;
use opentelemetry::trace::get_active_span;
use opentelemetry::KeyValue;
use tracing::{debug, info};

use crate::affinity::derive_implicit_affinity;
use crate::metrics as bs_metrics;
use crate::metrics::labels as metric_labels;
use crate::router::cache_regret;
use crate::router::orchestrator::OrchestratorService;
use crate::session_cache::CachedRoute;
use crate::streaming::PinAction;
use crate::tracing::plano as tracing_plano;

/// Resolved session identity for one request.
pub struct SessionResolution {
    /// Stable prefix hash (system + tools + first user message), independent of
    /// `prompt_caching.enabled` so it can still drive the `x-plano-prefix-hash`
    /// RING_HASH replica stickiness header. `None` when the request opted out or has
    /// no anchorable prompt.
    pub request_prefix_hash: Option<u64>,
    /// Session key: the explicit `X-Model-Affinity` value, or the implicit prefix
    /// hash key when implicit pinning is active.
    pub session_id: Option<String>,
    /// True when the session was derived implicitly (pin-after-hit applies).
    pub is_implicit_session: bool,
}

/// Outcome of a session-pin lookup.
pub struct PinLookup {
    /// A warm, non-drifted pin that should short-circuit routing.
    pub pinned_route: Option<CachedRoute>,
    /// An expired or drifted pin, retained only so the cache-regret gate can weigh
    /// the cost of abandoning a plausibly-warm provider cache.
    pub previous_route: Option<CachedRoute>,
    /// Whether the previous pin's stored prefix hash no longer matches (cache cold).
    pub previous_prefix_drifted: bool,
}

/// Estimate the request's context size in tokens for the cache-regret gate.
/// Uses the tiktoken-based counter when available, falling back to the chars/4
/// heuristic. Precision is not critical — the estimate only feeds a threshold
/// comparison, and both sides of the regret formula scale with the same number.
pub fn estimate_context_tokens(messages: &[Message], model: &str) -> u64 {
    let text: String = messages
        .iter()
        .filter_map(|m| m.content.as_ref().map(|c| c.to_string()))
        .collect::<Vec<_>>()
        .join("\n");
    match common::tokenizer::token_count(model, &text) {
        Ok(count) => count as u64,
        Err(_) => (text.len() / 4) as u64,
    }
}

/// Resolve the session key and prefix hash from the (already filtered / state-merged)
/// request. An explicit affinity header always pins; implicit pinning is gated on
/// prompt caching + session affinity. The prefix hash is derived even when caching is
/// off (only `X-Plano-Cache: off` or an unanchorable prompt suppresses it).
pub fn resolve_session(
    explicit_session_id: Option<String>,
    messages: &[Message],
    tool_names: Option<&[String]>,
    tenant_id: Option<&str>,
    prompt_caching: &EffectivePromptCaching,
    cache_off_for_request: bool,
) -> SessionResolution {
    let implicit_affinity = if cache_off_for_request {
        None
    } else {
        derive_implicit_affinity(messages, tool_names, tenant_id)
    };
    let request_prefix_hash = implicit_affinity.as_ref().map(|a| a.prefix_hash);

    let (session_id, is_implicit_session) = match explicit_session_id {
        Some(sid) => (Some(sid), false),
        None if prompt_caching.session_affinity && !cache_off_for_request => (
            implicit_affinity.as_ref().map(|a| a.session_key.clone()),
            true,
        ),
        None => (None, false),
    };

    SessionResolution {
        request_prefix_hash,
        session_id,
        is_implicit_session,
    }
}

/// Look up the session pin and classify it. Logically-expired pins never
/// short-circuit routing, but they (and prefix-drifted pins) are returned as
/// `previous_route` so the cache-regret gate can reason about the switch cost.
pub async fn lookup_session_pin(
    orchestrator: &OrchestratorService,
    session_id: Option<&str>,
    tenant_id: Option<&str>,
    request_prefix_hash: Option<u64>,
) -> PinLookup {
    let mut result = PinLookup {
        pinned_route: None,
        previous_route: None,
        previous_prefix_drifted: false,
    };

    let Some(sid) = session_id else {
        return result;
    };
    let Some(lookup) = orchestrator.get_cached_route(sid, tenant_id).await else {
        return result;
    };

    // Prefix drift is independent of staleness: a stored prefix hash that no longer
    // matches means the provider cache is already cold, so the regret gate must not
    // treat the previous pin as plausibly warm.
    let prefix_drifted = match (lookup.route.prefix_hash, request_prefix_hash) {
        (Some(stored), Some(current)) => stored != current,
        _ => false,
    };

    if lookup.is_stale {
        debug!(
            session_id = %sid,
            model = %lookup.route.model_name,
            prefix_drifted,
            "session pin expired — re-routing fresh"
        );
        bs_metrics::record_session_pin_event(metric_labels::PIN_EVENT_STALE_HINT);
        result.previous_prefix_drifted = prefix_drifted;
        result.previous_route = Some(lookup.route);
    } else if prefix_drifted {
        info!(
            session_id = %sid,
            model = %lookup.route.model_name,
            "prompt prefix drifted — provider cache already lost, re-routing fresh"
        );
        bs_metrics::record_session_pin_event(metric_labels::PIN_EVENT_PREFIX_DRIFT);
        result.previous_prefix_drifted = true;
        result.previous_route = Some(lookup.route);
    } else {
        result.pinned_route = Some(lookup.route);
    }

    result
}

/// Cache-regret gate: the router has proposed a model, but if the previous pin's
/// provider cache is plausibly still warm, abandoning it forces the candidate to
/// re-ingest the full context at its uncached input rate. Only switch when that
/// regret is within the developer's threshold; otherwise retain the previous model by
/// mutating `model`/`route_name` in place. Quality and cost stay separate — the gate
/// never picks a "better" model, it only vetoes an unaffordable switch.
///
/// Returns whether the retained model's warm state must be preserved on re-pin.
pub async fn apply_cache_regret_gate(
    stickiness: Option<EffectiveSessionStickiness>,
    previous_route: Option<&CachedRoute>,
    previous_prefix_drifted: bool,
    est_context_tokens: u64,
    orchestrator: &OrchestratorService,
    model: &mut String,
    route_name: &mut Option<String>,
) -> bool {
    let Some(stickiness) = stickiness else {
        return false;
    };
    let Some(previous) = previous_route else {
        return false;
    };
    if !cache_regret::gate_applies(previous, previous_prefix_drifted, model.as_str()) {
        return false;
    }

    let previous_rates = orchestrator.model_rates(&previous.model_name).await;
    let candidate_rates = orchestrator.model_rates(model.as_str()).await;
    let (Some(prev_rates), Some(cand_rates)) = (previous_rates, candidate_rates) else {
        // Without pricing for both sides the regret is unknowable — fail open (switch
        // freely) rather than silently overriding the router on guesswork.
        debug!(
            previous_model = %previous.model_name,
            candidate_model = %model,
            "cache-regret gate skipped — missing pricing data for one or both models"
        );
        return false;
    };

    let evaluation = cache_regret::evaluate_switch(
        &stickiness.switch_cost,
        est_context_tokens,
        prev_rates.cached_input_rate(stickiness.cache_read_discount),
        cand_rates.input_per_million,
    );
    get_active_span(|span| {
        span.set_attribute(KeyValue::new(
            tracing_plano::SWITCH_REGRET_USD,
            evaluation.regret_usd,
        ));
        span.set_attribute(KeyValue::new(
            tracing_plano::SWITCH_THRESHOLD_USD,
            evaluation.threshold_usd,
        ));
    });

    match evaluation.decision {
        cache_regret::SwitchDecision::Allow => {
            info!(
                previous_model = %previous.model_name,
                candidate_model = %model,
                regret_usd = evaluation.regret_usd,
                threshold_usd = evaluation.threshold_usd,
                "switch allowed — regret within threshold"
            );
            bs_metrics::record_session_switch_decision(metric_labels::SWITCH_DECISION_ALLOWED);
            get_active_span(|span| {
                span.set_attribute(KeyValue::new(
                    tracing_plano::SWITCH_DECISION,
                    metric_labels::SWITCH_DECISION_ALLOWED,
                ));
            });
            false
        }
        cache_regret::SwitchDecision::RetainPrevious => {
            info!(
                previous_model = %previous.model_name,
                candidate_model = %model,
                regret_usd = evaluation.regret_usd,
                threshold_usd = evaluation.threshold_usd,
                est_context_tokens,
                "switch vetoed — cache regret exceeds threshold, retaining previous model"
            );
            bs_metrics::record_session_switch_decision(metric_labels::SWITCH_DECISION_RETAINED);
            get_active_span(|span| {
                span.set_attribute(KeyValue::new(
                    tracing_plano::SWITCH_DECISION,
                    metric_labels::SWITCH_DECISION_RETAINED,
                ));
            });
            // Counterfactual: `model`/`route_name` still hold the router's pick — the
            // route we *would* have taken had the switch been allowed. Record it
            // (telemetry only) before overwriting with the retained model, so evals
            // can quantify the road not taken.
            if stickiness.record_counterfactual {
                let counterfactual_route = match route_name.as_deref() {
                    Some(rn) if !rn.is_empty() && rn != "none" => format!("{model} ({rn})"),
                    _ => model.clone(),
                };
                get_active_span(|span| {
                    span.set_attribute(KeyValue::new(
                        tracing_plano::SWITCH_COUNTERFACTUAL_ROUTE,
                        counterfactual_route,
                    ));
                });
            }
            *model = previous.model_name.clone();
            *route_name = previous.route_name.clone();
            // gate_applies guarantees the previous pin observed a cache hit; keep that
            // warm-state so the refreshed pin isn't demoted to cold.
            previous.observed_cache_hit
        }
    }
}

/// Decide the response-side pin action after routing resolves the final model.
/// Implicit sessions defer their pin until the response proves cache activity
/// (pin-after-hit); explicit sessions pin immediately (refreshing on later turns).
/// Returns `None` when caching is off or there is no session to pin.
#[allow(clippy::too_many_arguments)]
pub async fn plan_pin_after_routing(
    orchestrator: &Arc<OrchestratorService>,
    prompt_caching: &EffectivePromptCaching,
    session_id: Option<&str>,
    is_implicit_session: bool,
    tenant_id: Option<&str>,
    model: &str,
    route_name: Option<&str>,
    request_prefix_hash: Option<u64>,
    retained_previous_observed_hit: bool,
) -> Option<PinAction> {
    if !prompt_caching.enabled {
        return None;
    }
    let sid = session_id?;

    if is_implicit_session {
        // Pin-after-hit: defer the pin until the response proves the workload
        // actually benefits from caching.
        return Some(PinAction::CommitOnCacheActivity);
    }

    // Explicit sessions keep pin-on-turn-1 behavior. When the gate retained a warm
    // previous model, carry its observed-hit state forward so the pin stays warm
    // rather than resetting to cold.
    orchestrator
        .cache_route(
            sid,
            tenant_id,
            CachedRoute {
                model_name: model.to_string(),
                route_name: route_name.map(|s| s.to_string()),
                prefix_hash: request_prefix_hash,
                observed_cache_hit: retained_previous_observed_hit,
            },
            prompt_caching.session_ttl_seconds,
        )
        .await;
    Some(PinAction::Refresh {
        previously_observed_hit: retained_previous_observed_hit,
    })
}
