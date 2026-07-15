//! Fixed label-value constants so callers never emit free-form strings
//! (which would blow up cardinality).

// Handler enum — derived from the path+method match in `route()`.
pub const HANDLER_AGENT_CHAT: &str = "agent_chat";
pub const HANDLER_ROUTING_DECISION: &str = "routing_decision";
pub const HANDLER_LLM_CHAT: &str = "llm_chat";
pub const HANDLER_FUNCTION_CALLING: &str = "function_calling";
pub const HANDLER_LIST_MODELS: &str = "list_models";
pub const HANDLER_CORS_PREFLIGHT: &str = "cors_preflight";
pub const HANDLER_NOT_FOUND: &str = "not_found";

// Router "route" class — which brightstaff endpoint prompted the decision.
pub const ROUTE_AGENT: &str = "agent";
pub const ROUTE_ROUTING: &str = "routing";
pub const ROUTE_LLM: &str = "llm";

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
