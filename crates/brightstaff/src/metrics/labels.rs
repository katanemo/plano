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

// Session pin lifecycle events (brightstaff_session_pin_events_total).
/// Implicit session committed its pin after the first observed cache activity.
pub const PIN_EVENT_IMPLICIT_COMMIT: &str = "implicit_commit";
/// An existing pin was refreshed (TTL extended, observed-hit state updated).
pub const PIN_EVENT_REFRESH: &str = "refresh";
/// A pinned request's prefix hash no longer matched — cache already lost, re-routed.
pub const PIN_EVENT_PREFIX_DRIFT: &str = "prefix_drift";
/// A logically-expired pin was used as a soft switch-penalty hint for routing.
pub const PIN_EVENT_STALE_HINT: &str = "stale_hint";
/// A pinned session that previously produced cache hits stopped producing them.
pub const PIN_EVENT_VALIDATION_FAILED: &str = "validation_failed";

// Session-stickiness cost-gate decisions (brightstaff_session_switch_decisions_total).
/// The proposed switch was within the developer's switch-cost threshold.
pub const SWITCH_DECISION_ALLOWED: &str = "allowed";
/// The regret exceeded the threshold — the previous model was retained.
pub const SWITCH_DECISION_RETAINED: &str = "retained";
