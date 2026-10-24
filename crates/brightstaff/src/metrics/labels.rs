//! Fixed label-value constants so callers never emit free-form strings
//! (which would blow up cardinality).

// Handler enum — derived from the path+method match in `route()`.
pub const HANDLER_AGENT_CHAT: &str = "agent_chat";
pub const HANDLER_ROUTING_DECISION: &str = "routing_decision";
pub const HANDLER_LLM_CHAT: &str = "llm_chat";
pub const HANDLER_EMBEDDINGS: &str = "embeddings";
pub const HANDLER_FUNCTION_CALLING: &str = "function_calling";
pub const HANDLER_LIST_MODELS: &str = "list_models";
pub const HANDLER_CORS_PREFLIGHT: &str = "cors_preflight";
pub const HANDLER_NOT_FOUND: &str = "not_found";

// Router "route" class — which brightstaff endpoint prompted the decision.
pub const ROUTE_AGENT: &str = "agent";
pub const ROUTE_ROUTING: &str = "routing";
pub const ROUTE_LLM: &str = "llm";

// Org label on the token counters (tracing.attribution_header). Values come
// from a request header, so they are normalized to a bounded alphabet here;
// anything else collapses to one of these two constants.
pub const ORG_UNATTRIBUTED: &str = "unattributed";
pub const ORG_INVALID: &str = "invalid";
pub const ORG_MAX_LEN: usize = 64;

/// Map a raw attribution-header value to a safe metric label value.
pub fn normalize_org(raw: Option<&str>) -> String {
    let Some(value) = raw.map(str::trim).filter(|v| !v.is_empty()) else {
        return ORG_UNATTRIBUTED.to_string();
    };
    let ok = value.len() <= ORG_MAX_LEN
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    if ok {
        value.to_string()
    } else {
        ORG_INVALID.to_string()
    }
}

/// Service label value for a missing or blank `X-Fikar-Service`.
pub const SERVICE_UNATTRIBUTED: &str = "unattributed";
/// Label value for a present header whose value fails validation.
pub const ATTR_INVALID: &str = "invalid";

/// `^[a-z][a-z0-9-]{1,31}$`; missing or blank is `unattributed`, a mismatch is `invalid`.
pub fn normalize_service(raw: Option<&str>) -> String {
    let Some(v) = raw.map(str::trim).filter(|v| !v.is_empty()) else {
        return SERVICE_UNATTRIBUTED.to_string();
    };
    let b = v.as_bytes();
    let ok = (2..=32).contains(&b.len())
        && b[0].is_ascii_lowercase()
        && b[1..]
            .iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-');
    if ok {
        v.to_string()
    } else {
        ATTR_INVALID.to_string()
    }
}

/// `^([0-9]{1,12}|system)$`; missing or blank is `""`, a mismatch is `invalid`.
pub fn normalize_user(raw: Option<&str>) -> String {
    let Some(v) = raw.map(str::trim).filter(|v| !v.is_empty()) else {
        return String::new();
    };
    let ok =
        v == "system" || ((1..=12).contains(&v.len()) && v.bytes().all(|c| c.is_ascii_digit()));
    if ok {
        v.to_string()
    } else {
        ATTR_INVALID.to_string()
    }
}

/// `^[A-Za-z0-9._:-]{1,64}$`; missing or blank is `""`, a mismatch is `invalid`.
pub fn normalize_execution(raw: Option<&str>) -> String {
    let Some(v) = raw.map(str::trim).filter(|v| !v.is_empty()) else {
        return String::new();
    };
    let ok = v.len() <= 64
        && v.bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b':' | b'-'));
    if ok {
        v.to_string()
    } else {
        ATTR_INVALID.to_string()
    }
}

// Token kind for brightstaff_llm_tokens_total.
pub const TOKEN_KIND_PROMPT: &str = "prompt";
pub const TOKEN_KIND_COMPLETION: &str = "completion";
/// Input tokens served from the provider's prompt cache (billed at the cached rate).
pub const TOKEN_KIND_CACHE_READ: &str = "cache_read";
/// Input tokens written into the provider's prompt cache (cache-creation surcharge).
pub const TOKEN_KIND_CACHE_WRITE: &str = "cache_write";

// LLM error_class values (match docstring in metrics/mod.rs).
pub const LLM_ERR_NONE: &str = "none";
pub const LLM_ERR_TIMEOUT: &str = "timeout";
pub const LLM_ERR_CONNECT: &str = "connect";
pub const LLM_ERR_PARSE: &str = "parse";
pub const LLM_ERR_OTHER: &str = "other";
pub const LLM_ERR_STREAM: &str = "stream";

// Routing service outcome values.
pub const ROUTING_SVC_DECISION_SERVED: &str = "decision_served";
pub const ROUTING_SVC_NO_CANDIDATES: &str = "no_candidates";
pub const ROUTING_SVC_POLICY_ERROR: &str = "policy_error";

// Session cache outcome values.
pub const SESSION_CACHE_HIT: &str = "hit";
pub const SESSION_CACHE_MISS: &str = "miss";
pub const SESSION_CACHE_STORE: &str = "store";
/// Lookup failed in the backend. Kept apart from `miss` so an unreachable cache is never
/// read as "cold session" — that looks identical to healthy traffic while stickiness is
/// entirely off.
pub const SESSION_CACHE_ERROR: &str = "error";
/// Store failed in the backend: the binding was not persisted, so the next turn of this
/// session cannot pin.
pub const SESSION_CACHE_STORE_ERROR: &str = "store_error";

// Prompt cache outcome values (brightstaff_prompt_cache_requests_total).
pub const PROMPT_CACHE_HIT: &str = "hit";
pub const PROMPT_CACHE_MISS: &str = "miss";

// Session binding lifecycle events (brightstaff_session_binding_events_total).
/// An existing binding was refreshed from observed usage (TTL extended, token counts
/// and running cost updated) after a turn completed.
pub const BINDING_EVENT_REFRESH: &str = "refresh";

// Session-stickiness decisions (brightstaff_session_switch_decisions_total).
// `decision` label — the coarse outcome:
/// The proposed switch was honored (free, within the overhead cap, or unpriced fail-open).
pub const SWITCH_DECISION_ALLOWED: &str = "allowed";
/// The switch would have exceeded the session's overhead cap — the warm anchor was retained.
pub const SWITCH_DECISION_RETAINED: &str = "retained";

// `reason` label — why the decision was made:
/// The router agreed with the warm anchor; no switch was needed.
pub const SWITCH_REASON_SAME_ANCHOR: &str = "same_anchor";
/// The candidate was outright cheaper (negative cost) — a free switch.
pub const SWITCH_REASON_FREE: &str = "free";
/// A paid switch that kept cumulative spend within the session's overhead cap.
pub const SWITCH_REASON_WITHIN_CAP: &str = "within_cap";
/// A paid switch that would have pushed cumulative spend over the overhead cap — retained.
pub const SWITCH_REASON_OVER_CAP: &str = "over_cap";
/// Pricing was missing for one side, so the switch was allowed without a cost gate.
pub const SWITCH_REASON_NO_PRICING: &str = "no_pricing";

// Quality-routing skips (brightstaff_router_skips_total).
/// The last message was not a user turn, so the prior routing decision was replayed
/// instead of re-running the router.
pub const ROUTING_SKIP_NOT_USER_TURN: &str = "not_user_turn";

#[cfg(test)]
mod org_tests {
    use super::*;

    #[test]
    fn missing_or_blank_is_unattributed() {
        assert_eq!(normalize_org(None), ORG_UNATTRIBUTED);
        assert_eq!(normalize_org(Some("")), ORG_UNATTRIBUTED);
        assert_eq!(normalize_org(Some("   ")), ORG_UNATTRIBUTED);
    }

    #[test]
    fn plain_ids_pass_through_trimmed() {
        assert_eq!(normalize_org(Some(" 3 ")), "3");
        assert_eq!(normalize_org(Some("org_4.a-b")), "org_4.a-b");
    }

    #[test]
    fn malformed_values_collapse_to_invalid() {
        assert_eq!(normalize_org(Some("a b")), ORG_INVALID);
        assert_eq!(normalize_org(Some("x".repeat(65).as_str())), ORG_INVALID);
        assert_eq!(normalize_org(Some("3;drop")), ORG_INVALID);
        assert_eq!(normalize_org(Some("org{id}")), ORG_INVALID);
    }
}

#[cfg(test)]
mod attribution_tests {
    use super::*;

    #[test]
    fn service_rules() {
        assert_eq!(normalize_service(None), "unattributed");
        assert_eq!(normalize_service(Some("  ")), "unattributed");
        assert_eq!(normalize_service(Some(" chat ")), "chat");
        assert_eq!(normalize_service(Some("chat-backend2")), "chat-backend2");
        for bad in ["a", "Chat", "1abc", "-ab", "a_b", "a b", &"a".repeat(33)] {
            assert_eq!(normalize_service(Some(bad)), "invalid", "{bad}");
        }
        assert_eq!(normalize_service(Some(&"a".repeat(32))), "a".repeat(32));
    }

    #[test]
    fn user_rules() {
        assert_eq!(normalize_user(None), "");
        assert_eq!(normalize_user(Some(" ")), "");
        assert_eq!(normalize_user(Some("42")), "42");
        assert_eq!(normalize_user(Some("system")), "system");
        assert_eq!(normalize_user(Some("123456789012")), "123456789012");
        for bad in ["1234567890123", "-1", "4 2", "System", "bob", "1.5"] {
            assert_eq!(normalize_user(Some(bad)), "invalid", "{bad}");
        }
    }

    #[test]
    fn execution_rules() {
        assert_eq!(normalize_execution(None), "");
        assert_eq!(normalize_execution(Some("")), "");
        assert_eq!(normalize_execution(Some("turn-8c1d2e")), "turn-8c1d2e");
        assert_eq!(normalize_execution(Some("A.b_c:d-1")), "A.b_c:d-1");
        assert_eq!(normalize_execution(Some(&"x".repeat(64))), "x".repeat(64));
        for bad in ["a b", "a/b", "é", &"x".repeat(65)] {
            assert_eq!(normalize_execution(Some(bad)), "invalid", "{bad}");
        }
    }
}
