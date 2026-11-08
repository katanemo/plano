pub const RATELIMIT_SELECTOR_HEADER_KEY: &str = "x-arch-ratelimit-selector";
pub const SYSTEM_ROLE: &str = "system";
pub const USER_ROLE: &str = "user";
pub const TOOL_ROLE: &str = "tool";
pub const ASSISTANT_ROLE: &str = "assistant";
pub const MODEL_SERVER_REQUEST_TIMEOUT_MS: u64 = 30000; // 30 seconds
pub const MODEL_SERVER_NAME: &str = "bright_staff";
pub const ARCH_ROUTING_HEADER: &str = "x-arch-llm-provider";
pub const MESSAGES_KEY: &str = "messages";
pub const ARCH_PROVIDER_HINT_HEADER: &str = "x-arch-llm-provider-hint";
pub const ARCH_IS_STREAMING_HEADER: &str = "x-arch-streaming-request";
pub const CHAT_COMPLETIONS_PATH: &str = "/v1/chat/completions";
pub const OPENAI_RESPONSES_API_PATH: &str = "/v1/responses";
pub const MESSAGES_PATH: &str = "/v1/messages";
pub const HEALTHZ_PATH: &str = "/healthz";
pub const REQUEST_ID_HEADER: &str = "x-request-id";
pub const MODEL_AFFINITY_HEADER: &str = "x-model-affinity";
/// Per-request prompt-caching control. `off` disables implicit session affinity and
/// cache-control injection for that single request.
pub const PLANO_CACHE_HEADER: &str = "x-plano-cache";
/// Hash of the stable prompt prefix, forwarded upstream so self-hosted multi-replica
/// backends can do KV-aware (consistent-hash) replica routing at the LB/Envoy layer.
pub const PLANO_PREFIX_HASH_HEADER: &str = "x-plano-prefix-hash";
/// Per-request override for `routing.routing_budget.max_switch_spend_pct` (0–100,
/// e.g. `20` = 20%). When absent, the configured default applies.
pub const PLANO_MAX_SWITCH_SPEND_PCT_HEADER: &str = "x-plano-max-switch-spend-pct";
pub const ENVOY_ORIGINAL_PATH_HEADER: &str = "x-envoy-original-path";
pub const TRACE_PARENT_HEADER: &str = "traceparent";
pub const ARCH_INTERNAL_CLUSTER_NAME: &str = "arch_internal";
pub const ARCH_UPSTREAM_HOST_HEADER: &str = "x-arch-upstream";
pub const OTEL_COLLECTOR_HTTP: &str = "opentelemetry_collector_http";
pub const LLM_ROUTE_HEADER: &str = "x-arch-llm-route";
pub const ENVOY_RETRY_HEADER: &str = "x-envoy-max-retries";
pub const BRIGHT_STAFF_SERVICE_NAME: &str = "brightstaff";
pub const PLANO_CLUSTER: &str = "plano";
