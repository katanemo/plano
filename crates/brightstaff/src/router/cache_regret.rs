//! Cache-regret cost gate for session stickiness.
//!
//! When a session re-route is already being considered (stale pin or prefix drift)
//! and the previously-pinned model's provider prompt cache is plausibly still warm,
//! switching models forces the new model to re-ingest the full context at its
//! uncached input rate. The *regret* of that switch is the input-cost delta:
//!
//! ```text
//! regret_usd = context_tokens x (candidate_uncached_input - previous_cached_input) / 1e6
//! ```
//!
//! The gate is deliberately input-only: output-token savings are unknowable before
//! the response is generated (reasoning models can emit 5-10x the tokens for the
//! same task), so they are never credited against the regret. Negative regret — the
//! candidate's uncached input rate undercuts the previous model's cached rate —
//! always allows the switch. Positive regret is compared against the
//! developer-defined threshold; Plano never invents one.
//!
//! Quality and cost stay separate: the router decides *which* model is better, this
//! gate only decides whether the switch is *affordable*.

use common::configuration::SwitchCostThreshold;

use crate::session_cache::CachedRoute;

const TOKENS_PER_MILLION: f64 = 1_000_000.0;

/// Whether the cost gate applies to a proposed switch away from `previous`.
///
/// The gate only fires when there is a warm cache worth protecting:
/// * the previous pin has actually observed cache activity (`observed_cache_hit`) —
///   a pin that never produced a hit has nothing to lose, and
/// * the prompt prefix has NOT drifted — a drifted prefix means the provider cache
///   is already cold, so switching is free, and
/// * the router actually proposed a different model.
pub fn gate_applies(previous: &CachedRoute, prefix_drifted: bool, candidate_model: &str) -> bool {
    previous.observed_cache_hit && !prefix_drifted && previous.model_name != candidate_model
}

/// Outcome of the cost gate for a proposed model switch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SwitchDecision {
    /// The switch is within the developer's cost policy (or is outright cheaper).
    Allow,
    /// The regret exceeds the threshold — keep the previously-pinned model.
    RetainPrevious,
}

/// Estimated regret for one proposed switch, for observability.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SwitchEvaluation {
    pub decision: SwitchDecision,
    /// Estimated input-cost regret in USD (negative when the candidate is cheaper).
    pub regret_usd: f64,
    /// The resolved ceiling in USD the regret was compared against.
    pub threshold_usd: f64,
}

/// Evaluate whether abandoning a plausibly-warm cache for `candidate` is within the
/// developer's cost policy. Rates are USD per million tokens.
///
/// * `previous_cached_rate` — the pinned model's cached (prompt-cache read) input rate.
/// * `candidate_uncached_rate` — the candidate's full input rate (it starts cold).
pub fn evaluate_switch(
    threshold: &SwitchCostThreshold,
    est_context_tokens: u64,
    previous_cached_rate: f64,
    candidate_uncached_rate: f64,
) -> SwitchEvaluation {
    let context_millions = est_context_tokens as f64 / TOKENS_PER_MILLION;
    let regret_usd = context_millions * (candidate_uncached_rate - previous_cached_rate);
    // Cost to stay: re-reading the context on the warm model at its cached rate.
    let cached_baseline_usd = context_millions * previous_cached_rate;

    let threshold_usd = match *threshold {
        SwitchCostThreshold::MaxRegretUsd { value } => value,
        SwitchCostThreshold::MaxRegretPctOfCached { value } => {
            (value / 100.0) * cached_baseline_usd
        }
    };

    // Case 1: negative (or zero) regret — the candidate is cheap enough on input
    // that losing the cache doesn't matter. Always allow.
    // Case 3: positive regret — allow only within the developer's ceiling.
    // Case 2 (output savings) is deliberately never credited.
    let decision = if regret_usd <= threshold_usd {
        SwitchDecision::Allow
    } else {
        SwitchDecision::RetainPrevious
    };

    SwitchEvaluation {
        decision,
        regret_usd,
        threshold_usd,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real models.dev rates (USD per million input tokens):
    //   claude-opus-4-1:   input 15,  cache_read 1.5
    //   claude-sonnet-4-5: input 3,   cache_read 0.3
    //   claude-haiku-4-5:  input 1,   cache_read 0.1
    //   gpt-4.1:           input 2,   cache_read 0.5

    const USD_10_CENTS: SwitchCostThreshold = SwitchCostThreshold::MaxRegretUsd { value: 0.10 };

    #[test]
    fn case1_negative_regret_always_allows() {
        // Pinned opus (cached 1.5) -> haiku (uncached 1.0) over 100k context:
        // regret = 0.1M x (1.0 - 1.5) = -$0.05 — cheaper even after re-reading.
        let eval = evaluate_switch(&USD_10_CENTS, 100_000, 1.5, 1.0);
        assert_eq!(eval.decision, SwitchDecision::Allow);
        assert!((eval.regret_usd - (-0.05)).abs() < 1e-9);
    }

    #[test]
    fn case1_allows_even_with_zero_threshold() {
        // A zero ceiling still allows strictly-cheaper switches.
        let zero = SwitchCostThreshold::MaxRegretUsd { value: 0.0 };
        let eval = evaluate_switch(&zero, 100_000, 1.5, 1.0);
        assert_eq!(eval.decision, SwitchDecision::Allow);
    }

    #[test]
    fn case3_regret_under_ceiling_allows() {
        // Pinned opus (cached 1.5) -> gpt-4.1 (uncached 2.0) over 100k:
        // regret = 0.1M x (2.0 - 1.5) = +$0.05 <= $0.10 -> allow.
        let eval = evaluate_switch(&USD_10_CENTS, 100_000, 1.5, 2.0);
        assert_eq!(eval.decision, SwitchDecision::Allow);
        assert!((eval.regret_usd - 0.05).abs() < 1e-9);
        assert!((eval.threshold_usd - 0.10).abs() < 1e-9);
    }

    #[test]
    fn case3_regret_over_ceiling_retains_previous() {
        // Pinned sonnet (cached 0.3) -> gpt-5.5-class (uncached 5.0) over 150k:
        // regret = 0.15M x (5.0 - 0.3) = +$0.705 > $0.10 -> veto.
        let eval = evaluate_switch(&USD_10_CENTS, 150_000, 0.3, 5.0);
        assert_eq!(eval.decision, SwitchDecision::RetainPrevious);
        assert!((eval.regret_usd - 0.705).abs() < 1e-9);
    }

    #[test]
    fn raising_the_ceiling_is_the_case3_dial() {
        // Same switch clears with a quality-first ceiling of $1.00.
        let generous = SwitchCostThreshold::MaxRegretUsd { value: 1.0 };
        let eval = evaluate_switch(&generous, 150_000, 0.3, 5.0);
        assert_eq!(eval.decision, SwitchDecision::Allow);
    }

    #[test]
    fn pct_of_cached_threshold_scales_with_baseline() {
        // Baseline to stay on sonnet-cached over 100k = 0.1M x 0.3 = $0.03.
        // 200% ceiling -> allowed regret $0.06.
        let pct = SwitchCostThreshold::MaxRegretPctOfCached { value: 200.0 };

        // gpt-4o-class candidate (uncached 2.5): regret = 0.1M x 2.2 = $0.22 > $0.06.
        let veto = evaluate_switch(&pct, 100_000, 0.3, 2.5);
        assert_eq!(veto.decision, SwitchDecision::RetainPrevious);
        assert!((veto.threshold_usd - 0.06).abs() < 1e-9);

        // Cheap candidate (uncached 0.8): regret = 0.1M x 0.5 = $0.05 <= $0.06.
        let allow = evaluate_switch(&pct, 100_000, 0.3, 0.8);
        assert_eq!(allow.decision, SwitchDecision::Allow);
    }

    #[test]
    fn pct_threshold_scales_with_context_size() {
        // The same relative policy admits the same switch regardless of context
        // size, because both regret and baseline scale linearly with tokens.
        let pct = SwitchCostThreshold::MaxRegretPctOfCached { value: 200.0 };
        let small = evaluate_switch(&pct, 10_000, 0.3, 0.8);
        let large = evaluate_switch(&pct, 1_000_000, 0.3, 0.8);
        assert_eq!(small.decision, SwitchDecision::Allow);
        assert_eq!(large.decision, SwitchDecision::Allow);
    }

    #[test]
    fn tiny_context_regret_is_negligible() {
        // 2k-token chat: even an expensive candidate costs ~$0.009 of regret.
        let eval = evaluate_switch(&USD_10_CENTS, 2_000, 0.3, 5.0);
        assert_eq!(eval.decision, SwitchDecision::Allow);
        assert!(eval.regret_usd < 0.01);
    }

    fn previous_pin(model: &str, observed_cache_hit: bool) -> CachedRoute {
        CachedRoute {
            model_name: model.to_string(),
            route_name: None,
            prefix_hash: Some(42),
            observed_cache_hit,
        }
    }

    #[test]
    fn gate_applies_to_warm_pin_with_different_candidate() {
        let previous = previous_pin("anthropic/claude-sonnet-4-5", true);
        assert!(gate_applies(&previous, false, "openai/gpt-5.5"));
    }

    #[test]
    fn gate_skipped_on_prefix_drift() {
        // A drifted prefix means the provider cache is already cold — switch freely.
        let previous = previous_pin("anthropic/claude-sonnet-4-5", true);
        assert!(!gate_applies(&previous, true, "openai/gpt-5.5"));
    }

    #[test]
    fn gate_skipped_when_no_cache_activity_observed() {
        // A pin that never produced a cache hit has nothing worth protecting.
        let previous = previous_pin("anthropic/claude-sonnet-4-5", false);
        assert!(!gate_applies(&previous, false, "openai/gpt-5.5"));
    }

    #[test]
    fn gate_skipped_when_candidate_is_same_model() {
        let previous = previous_pin("anthropic/claude-sonnet-4-5", true);
        assert!(!gate_applies(
            &previous,
            false,
            "anthropic/claude-sonnet-4-5"
        ));
    }
}
