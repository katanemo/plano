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
