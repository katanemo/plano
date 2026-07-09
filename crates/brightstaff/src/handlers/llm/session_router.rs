//! Session-cache-aware routing.
//!
//! Routing itself stays cache-blind: the `llm_router` (quality) still picks a
//! candidate model for every request. This module then decides whether to *honor*
//! that candidate or stick to the session's warm anchor, based on:
//!
//! * **Cache warmth** — inferred structurally from how long ago the session was last
//!   used vs. the provider's cache window ([`hermesllm::provider_cache_capability`]),
//!   so it works on the decision path with no provider response in hand.
//! * **A cumulative per-session switch budget** — a paid switch (the candidate must
//!   re-ingest the context at its uncached rate) is allowed only while budget remains;
//!   an outright-cheaper switch is free but never credits the budget back.
//!
//! The default posture is to stick. Quality and cost stay separate: the router decides
//! whether a switch *improves quality*; the budget decides whether it is *affordable*.
//!
//! Prompt-cache *marker injection* is a separate concern — see [`super::prompt_caching`].

use std::time::{Duration, SystemTime};

use common::configuration::EffectiveRoutingBudget;
use hermesllm::apis::openai::Message;
use hermesllm::{provider_cache_capability, ProviderCacheCapability, ProviderId};
use opentelemetry::trace::get_active_span;
use opentelemetry::KeyValue;
use tracing::{debug, info};

use crate::affinity::derive_implicit_affinity;
use crate::metrics as bs_metrics;
use crate::metrics::labels as metric_labels;
use crate::router::orchestrator::OrchestratorService;
use crate::session_cache::SessionBinding;
use crate::tracing::plano as tracing_plano;

/// Resolved session identity for one request.
pub struct SessionResolution {
    /// Stable prefix hash (system + tools + first user message), independent of
    /// `prompt_caching.enabled` so it can still drive the `x-plano-prefix-hash`
    /// RING_HASH replica-stickiness header. `None` when the request opted out or has
    /// no anchorable prompt.
    pub request_prefix_hash: Option<u64>,
    /// Session key: the explicit `X-Model-Affinity` value, or the implicit prefix-hash
    /// key when implicit affinity is active. `None` when there is nothing to anchor to.
    pub session_id: Option<String>,
}

/// Resolve the session key and prefix hash from the (already filtered / state-merged)
/// request. An explicit affinity header always anchors; the implicit key is derived
/// when `implicit_affinity_enabled` is set — true when either prompt caching's
/// `session_affinity` or the routing budget is active, so stickiness works whether or
/// not prompt caching is enabled. The prefix hash is derived regardless (only
/// `X-Plano-Cache: off` or an unanchorable prompt suppresses it) so the
/// `x-plano-prefix-hash` RING_HASH replica-stickiness header still works.
pub fn resolve_session(
    explicit_session_id: Option<String>,
    messages: &[Message],
    tool_names: Option<&[String]>,
    tenant_id: Option<&str>,
    implicit_affinity_enabled: bool,
    cache_off_for_request: bool,
) -> SessionResolution {
    let implicit_affinity = if cache_off_for_request {
        None
    } else {
        derive_implicit_affinity(messages, tool_names, tenant_id)
    };
    let request_prefix_hash = implicit_affinity.as_ref().map(|a| a.prefix_hash);

    let session_id = match explicit_session_id {
        Some(sid) => Some(sid),
        None if implicit_affinity_enabled && !cache_off_for_request => {
            implicit_affinity.as_ref().map(|a| a.session_key.clone())
        }
        None => None,
    };

    SessionResolution {
        request_prefix_hash,
        session_id,
    }
}

/// Extra memory retention beyond the warmth window, so a still-warm binding is never
/// GC'd out from under the router before it could plausibly go cold.
const GC_SLACK: Duration = Duration::from_secs(60);

/// Stable request facts the router reasons about. Independent of the transport (full
/// proxy vs. decision endpoint) so both paths route identically.
pub struct RouteFacts<'a> {
    /// Session key (explicit `X-Model-Affinity` or the implicit prefix key). `None`
    /// disables stickiness for this request (nothing to anchor to).
    pub session_id: Option<&'a str>,
    pub tenant_id: Option<&'a str>,
    /// Stable prompt-prefix hash; a mismatch vs. the stored binding means the provider
    /// cache is already lost, so a switch is free.
    pub prefix_hash: Option<u64>,
    /// Estimated context size in tokens (the tokens a switch would re-ingest).
    pub est_context_tokens: u64,
    /// The model the quality router picked for this request.
    pub candidate_model: &'a str,
    pub candidate_route: Option<&'a str>,
}

/// The routing decision plus the session state to carry into the response side.
pub struct RouteDecision {
    /// The model to actually dispatch to (the anchor when a switch was vetoed).
    pub model: String,
    pub route_name: Option<String>,
    /// Whether the session's cache was inferred warm at decision time.
    pub warm: bool,
    /// Remaining switch budget after this decision.
    pub switch_budget_usd: f64,
    /// Cumulative switches taken this session (after this decision).
    pub switches: u32,
    /// Context-token estimate persisted with the binding (refined later from usage).
    pub cached_tokens: u64,
    /// GC bound the binding was stored with (reused when the response side refreshes).
    pub gc_ttl: Duration,
}

/// Estimate the request's context size in tokens. Uses the tiktoken-based counter when
/// available, falling back to the chars/4 heuristic. Precision is not critical — it only
/// scales the switch-cost estimate, and both sides of the comparison scale with it.
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

/// Resolve a provider-qualified model id (e.g. `openai/gpt-4o`) to its cache window.
/// Unknown providers fall back to the conservative default.
fn capability_for_model(model: &str) -> ProviderCacheCapability {
    let provider_part = model.split_once('/').map(|(p, _)| p).unwrap_or(model);
    ProviderId::try_from(provider_part)
        .map(provider_cache_capability)
        .unwrap_or_default()
}

/// How long a binding on this model can sit idle before its cache is certainly cold.
fn warmth_window(cap: &ProviderCacheCapability) -> Duration {
    if cap.extended_retention {
        cap.extended_ttl
    } else {
        cap.idle_ttl.min(cap.hard_ttl)
    }
}

/// Whether the session's provider cache is plausibly still warm given how long ago it
/// was last used. Returns the warmth verdict and the measured idle gap.
fn warmth(
    binding: &SessionBinding,
    cap: &ProviderCacheCapability,
    now: SystemTime,
) -> (bool, Duration) {
    let idle = now
        .duration_since(binding.last_used)
        .unwrap_or(Duration::ZERO);
    let warm = if cap.extended_retention {
        idle <= cap.extended_ttl
    } else {
        idle <= cap.idle_ttl && idle <= cap.hard_ttl
    };
    (warm, idle)
}

/// Decide the final model for this request and persist the updated session binding.
///
/// Never overrides the router on a *cold* session — it only protects a warm cache. The
/// returned [`RouteDecision`] carries the model to dispatch plus the session state the
/// response side reuses when it refreshes the binding from real usage.
pub async fn route(
    orchestrator: &OrchestratorService,
    routing_budget: Option<&EffectiveRoutingBudget>,
    facts: RouteFacts<'_>,
) -> RouteDecision {
    let now = SystemTime::now();
    let candidate_gc_ttl = warmth_window(&capability_for_model(facts.candidate_model)) + GC_SLACK;

    // No session to anchor to: honor the candidate, persist nothing.
    let Some(session_id) = facts.session_id else {
        return RouteDecision {
            model: facts.candidate_model.to_string(),
            route_name: facts.candidate_route.map(str::to_string),
            warm: false,
            switch_budget_usd: 0.0,
            switches: 0,
            cached_tokens: facts.est_context_tokens,
            gc_ttl: candidate_gc_ttl,
        };
    };

    let existing = orchestrator.get_binding(session_id, facts.tenant_id).await;

    // Warmth + prefix drift. A drifted prefix means the cache is already cold.
    let (warm, idle) = match &existing {
        Some(b) => warmth(b, &capability_for_model(&b.anchor_model), now),
        None => (false, Duration::ZERO),
    };
    let drifted = match (
        existing.as_ref().and_then(|b| b.prefix_hash),
        facts.prefix_hash,
    ) {
        (Some(stored), Some(current)) => stored != current,
        _ => false,
    };
    let effective_warm = warm && !drifted;

    let seed = routing_budget.map(|s| s.seed_usd).unwrap_or(0.0);

    // Resolve the final model, budget, switch count, and decision telemetry.
    let mut model = facts.candidate_model.to_string();
    let mut route_name = facts.candidate_route.map(str::to_string);
    let budget_before;
    let mut budget;
    let mut switches;
    let mut cost_opt: Option<f64> = None;
    let mut counterfactual: Option<String> = None;
    let decision_label: &'static str;
    let reason: &'static str;

    match existing.as_ref() {
        Some(b) if effective_warm => {
            budget_before = b.switch_budget_usd;
            budget = b.switch_budget_usd;
            switches = b.switches;
            if facts.candidate_model == b.anchor_model {
                // Router agrees with the anchor — stick, no cost.
                decision_label = metric_labels::SWITCH_DECISION_ALLOWED;
                reason = metric_labels::SWITCH_REASON_SAME_ANCHOR;
            } else if let Some(cfg) = routing_budget {
                let context_tokens = if b.cached_tokens > 0 {
                    b.cached_tokens
                } else {
                    facts.est_context_tokens
                };
                match orchestrator
                    .estimate_switch_cost_in_usd(
                        context_tokens,
                        &b.anchor_model,
                        facts.candidate_model,
                        cfg.cache_read_discount,
                    )
                    .await
                {
                    // No pricing for one side — fail open (switch freely) rather than
                    // veto the router on guesswork.
                    None => {
                        switches += 1;
                        decision_label = metric_labels::SWITCH_DECISION_ALLOWED;
                        reason = metric_labels::SWITCH_REASON_NO_PRICING;
                        debug!(
                            anchor = %b.anchor_model,
                            candidate = %facts.candidate_model,
                            "switch allowed — missing pricing data, cannot gate"
                        );
                    }
                    Some(cost) => {
                        cost_opt = Some(cost);
                        if cost <= 0.0 {
                            // Outright cheaper: allowed for free. Does NOT credit the
                            // budget back — the "saving" is vs a path we didn't take,
                            // not real spendable money.
                            switches += 1;
                            decision_label = metric_labels::SWITCH_DECISION_ALLOWED;
                            reason = metric_labels::SWITCH_REASON_FREE;
                            info!(
                                anchor = %b.anchor_model,
                                candidate = %facts.candidate_model,
                                switch_cost_in_usd = cost,
                                "switch allowed — candidate undercuts the cached rate"
                            );
                        } else if cost <= budget {
                            budget -= cost;
                            switches += 1;
                            decision_label = metric_labels::SWITCH_DECISION_ALLOWED;
                            reason = metric_labels::SWITCH_REASON_WITHIN_BUDGET;
                            info!(
                                anchor = %b.anchor_model,
                                candidate = %facts.candidate_model,
                                switch_cost_in_usd = cost,
                                budget_remaining_in_usd = budget,
                                "switch allowed — within session switch budget"
                            );
                        } else {
                            // Unaffordable: retain the warm anchor.
                            if cfg.record_counterfactual {
                                counterfactual = Some(match route_name.as_deref() {
                                    Some(rn) if !rn.is_empty() && rn != "none" => {
                                        format!("{} ({rn})", facts.candidate_model)
                                    }
                                    _ => facts.candidate_model.to_string(),
                                });
                            }
                            model = b.anchor_model.clone();
                            route_name = b.route_name.clone();
                            decision_label = metric_labels::SWITCH_DECISION_RETAINED;
                            reason = metric_labels::SWITCH_REASON_OVER_BUDGET;
                            info!(
                                anchor = %b.anchor_model,
                                candidate = %facts.candidate_model,
                                switch_cost_in_usd = cost,
                                budget_remaining_in_usd = budget,
                                "switch vetoed — cost exceeds remaining budget, retaining anchor"
                            );
                        }
                    }
                }
            } else {
                // Warm but no budget configured — follow the router freely.
                switches += 1;
                decision_label = metric_labels::SWITCH_DECISION_ALLOWED;
                reason = metric_labels::SWITCH_REASON_FREE;
            }
            bs_metrics::record_session_switch_decision(decision_label, reason);
        }
        _ => {
            // Cold (or no binding, or drifted): honor the candidate and (re)seed a
            // fresh warm episode. Switches reset — this is a new cache lifetime.
            budget_before = seed;
            budget = match (routing_budget, existing.as_ref()) {
                (Some(cfg), Some(b)) if !cfg.replenish_on_rebind => b.switch_budget_usd,
                _ => seed,
            };
            switches = 0;
        }
    }

    // Context estimate persisted with the binding (refined later from real usage).
    let cached_tokens = if facts.est_context_tokens > 0 {
        facts.est_context_tokens
    } else {
        existing.as_ref().map(|b| b.cached_tokens).unwrap_or(0)
    };
    let gc_ttl = warmth_window(&capability_for_model(&model)) + GC_SLACK;

    // Observability: cache warmth + budget/switch state on the current span.
    get_active_span(|span| {
        span.set_attribute(KeyValue::new(tracing_plano::CACHE_WARM, effective_warm));
        span.set_attribute(KeyValue::new(
            tracing_plano::CACHE_IDLE_MS,
            idle.as_millis() as i64,
        ));
        if routing_budget.is_some() {
            span.set_attribute(KeyValue::new(
                tracing_plano::SESSION_BUDGET_REMAINING_IN_USD,
                budget,
            ));
            span.set_attribute(KeyValue::new(
                tracing_plano::SESSION_SWITCHES,
                switches as i64,
            ));
        }
        if let Some(cost) = cost_opt {
            span.set_attribute(KeyValue::new(tracing_plano::SWITCH_COST_IN_USD, cost));
            span.set_attribute(KeyValue::new(
                tracing_plano::SWITCH_THRESHOLD_IN_USD,
                budget_before,
            ));
            span.set_attribute(KeyValue::new(
                tracing_plano::SWITCH_DECISION,
                if model == facts.candidate_model {
                    metric_labels::SWITCH_DECISION_ALLOWED
                } else {
                    metric_labels::SWITCH_DECISION_RETAINED
                },
            ));
        }
        if let Some(ref cf) = counterfactual {
            span.set_attribute(KeyValue::new(
                tracing_plano::SWITCH_COUNTERFACTUAL_ROUTE,
                cf.clone(),
            ));
        }
    });

    orchestrator
        .store_binding(
            session_id,
            facts.tenant_id,
            SessionBinding {
                anchor_model: model.clone(),
                route_name: route_name.clone(),
                prefix_hash: facts.prefix_hash,
                last_used: now,
                cached_tokens,
                switch_budget_usd: budget,
                switches,
            },
            Some(gc_ttl),
        )
        .await;

    RouteDecision {
        model,
        route_name,
        warm: effective_warm,
        switch_budget_usd: budget,
        switches,
        cached_tokens,
        gc_ttl,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap_5m_1h() -> ProviderCacheCapability {
        ProviderCacheCapability {
            idle_ttl: Duration::from_secs(300),
            hard_ttl: Duration::from_secs(3600),
            extended_retention: false,
            extended_ttl: Duration::from_secs(3600),
        }
    }

    fn binding_used_ago(secs: u64) -> SessionBinding {
        SessionBinding {
            anchor_model: "anthropic/claude-sonnet-4-5".to_string(),
            route_name: None,
            prefix_hash: Some(1),
            last_used: SystemTime::now() - Duration::from_secs(secs),
            cached_tokens: 100_000,
            switch_budget_usd: 0.10,
            switches: 0,
        }
    }

    #[test]
    fn warm_within_idle_window() {
        let (warm, _) = warmth(&binding_used_ago(60), &cap_5m_1h(), SystemTime::now());
        assert!(warm);
    }

    #[test]
    fn cold_past_idle_window() {
        let (warm, _) = warmth(&binding_used_ago(600), &cap_5m_1h(), SystemTime::now());
        assert!(!warm);
    }

    #[test]
    fn extended_retention_keeps_warm_past_idle() {
        let cap = ProviderCacheCapability {
            extended_retention: true,
            ..cap_5m_1h()
        };
        // 10 minutes idle: cold under 5m, warm under the 1h extended window.
        let (warm, _) = warmth(&binding_used_ago(600), &cap, SystemTime::now());
        assert!(warm);
    }

    #[test]
    fn capability_resolves_from_model_prefix() {
        // Known provider prefix resolves; unknown falls back to the default.
        let anthropic = capability_for_model("anthropic/claude-sonnet-4-5");
        assert_eq!(anthropic, ProviderCacheCapability::default());
        let unknown = capability_for_model("madeup/model-x");
        assert_eq!(unknown, ProviderCacheCapability::default());
    }

    // ---- route() budget behavior ----

    use crate::router::model_metrics::{ModelMetricsService, ModelRates};
    use crate::session_cache::memory::MemorySessionCache;
    use std::collections::HashMap;
    use std::sync::Arc;

    // Anchor cached rate 0.3, candidate `pricey` input 5.0, candidate `cheap` input 0.1.
    // With a 100k-token context the paid switch costs 0.1 * (5.0 - 0.3) = $0.47 and the
    // cheap switch is 0.1 * (0.1 - 0.3) = -$0.02 (a credit).
    fn orch_with_rates() -> OrchestratorService {
        let mut rates = HashMap::new();
        rates.insert(
            "anthropic/expensive".to_string(),
            ModelRates {
                input_per_million: 3.0,
                output_per_million: 15.0,
                cache_read_per_million: Some(0.3),
            },
        );
        rates.insert(
            "openai/pricey".to_string(),
            ModelRates {
                input_per_million: 5.0,
                output_per_million: 15.0,
                cache_read_per_million: Some(0.5),
            },
        );
        rates.insert(
            "google/cheap".to_string(),
            ModelRates {
                input_per_million: 0.1,
                output_per_million: 0.4,
                cache_read_per_million: Some(0.01),
            },
        );
        let metrics = Arc::new(ModelMetricsService::from_rates_for_test(rates));
        let cache = Arc::new(MemorySessionCache::new(100));
        OrchestratorService::with_routing(
            "http://localhost/v1/chat/completions".to_string(),
            "m".to_string(),
            "p".to_string(),
            None,
            Some(metrics),
            Some(600),
            cache,
            None,
            8192,
        )
    }

    fn routing_budget(seed: f64) -> EffectiveRoutingBudget {
        EffectiveRoutingBudget {
            seed_usd: seed,
            replenish_on_rebind: true,
            cache_read_discount: 0.1,
            record_counterfactual: false,
        }
    }

    async fn seed_warm_binding(orch: &OrchestratorService, budget: f64, idle_secs: u64) {
        orch.store_binding(
            "s1",
            None,
            SessionBinding {
                anchor_model: "anthropic/expensive".to_string(),
                route_name: None,
                prefix_hash: Some(1),
                last_used: SystemTime::now() - Duration::from_secs(idle_secs),
                cached_tokens: 100_000,
                switch_budget_usd: budget,
                switches: 0,
            },
            Some(Duration::from_secs(3600)),
        )
        .await;
    }

    fn facts_for<'a>(candidate: &'a str) -> RouteFacts<'a> {
        RouteFacts {
            session_id: Some("s1"),
            tenant_id: None,
            prefix_hash: Some(1),
            est_context_tokens: 0,
            candidate_model: candidate,
            candidate_route: None,
        }
    }

    #[tokio::test]
    async fn paid_switch_within_budget_is_allowed_and_debits() {
        let orch = orch_with_rates();
        seed_warm_binding(&orch, 1.0, 30).await;
        let st = routing_budget(1.0);
        let d = route(&orch, Some(&st), facts_for("openai/pricey")).await;

        assert_eq!(d.model, "openai/pricey");
        assert!(d.warm);
        assert_eq!(d.switches, 1);
        assert!(
            (d.switch_budget_usd - 0.53).abs() < 1e-6,
            "budget {} != 0.53",
            d.switch_budget_usd
        );
    }

    #[tokio::test]
    async fn paid_switch_over_budget_retains_anchor() {
        let orch = orch_with_rates();
        seed_warm_binding(&orch, 0.10, 30).await;
        let st = routing_budget(0.10);
        let d = route(&orch, Some(&st), facts_for("openai/pricey")).await;

        assert_eq!(d.model, "anthropic/expensive");
        assert!(d.warm);
        assert_eq!(d.switches, 0);
        assert!((d.switch_budget_usd - 0.10).abs() < 1e-6);
    }

    #[tokio::test]
    async fn cheaper_switch_is_free_and_does_not_change_budget() {
        let orch = orch_with_rates();
        seed_warm_binding(&orch, 0.10, 30).await;
        let st = routing_budget(0.10);
        let d = route(&orch, Some(&st), facts_for("google/cheap")).await;

        assert_eq!(d.model, "google/cheap");
        assert!(d.warm);
        assert_eq!(d.switches, 1);
        // Free switches do not credit the budget — it stays at 0.10.
        assert!(
            (d.switch_budget_usd - 0.10).abs() < 1e-6,
            "budget {} != 0.10",
            d.switch_budget_usd
        );
    }

    #[tokio::test]
    async fn cold_session_reseeds_budget_and_follows_router() {
        let orch = orch_with_rates();
        // 10 minutes idle: past Anthropic's 5m idle window -> cold. Budget was drained.
        seed_warm_binding(&orch, 0.0, 600).await;
        let st = routing_budget(1.0);
        let d = route(&orch, Some(&st), facts_for("openai/pricey")).await;

        assert_eq!(d.model, "openai/pricey");
        assert!(!d.warm);
        assert_eq!(d.switches, 0);
        assert!(
            (d.switch_budget_usd - 1.0).abs() < 1e-6,
            "budget {} != seed",
            d.switch_budget_usd
        );
    }

    #[tokio::test]
    async fn no_session_honors_candidate() {
        let orch = orch_with_rates();
        let st = routing_budget(1.0);
        let facts = RouteFacts {
            session_id: None,
            tenant_id: None,
            prefix_hash: Some(1),
            est_context_tokens: 0,
            candidate_model: "openai/pricey",
            candidate_route: None,
        };
        let d = route(&orch, Some(&st), facts).await;
        assert_eq!(d.model, "openai/pricey");
        assert!(!d.warm);
    }
}
