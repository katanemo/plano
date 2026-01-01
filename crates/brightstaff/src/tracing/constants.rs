/// OpenTelemetry Semantic Conventions
///
/// This module defines standard attribute keys following OTEL semantic conventions.
/// See: https://opentelemetry.io/docs/specs/semconv/
// =============================================================================
// Span Attributes - HTTP
// =============================================================================
/// Semantic conventions for HTTP-related span attributes
pub mod http {
    /// HTTP request method
    /// Example: "GET", "POST", "PUT"
    pub const METHOD: &str = "http.method";

    /// HTTP response status code
    /// Example: "200", "404", "500"
    pub const STATUS_CODE: &str = "http.status_code";

    /// Full HTTP request URL
    pub const URL: &str = "http.url";

    /// HTTP request target (path + query)
    /// Example: "/v1/chat/completions?stream=true"
    pub const TARGET: &str = "http.target";

    /// Upstream target path after routing transformation
    /// Example: "/api/paas/v4/chat/completions" (for Zhipu provider)
    pub const UPSTREAM_TARGET: &str = "http.upstream_target";

    /// HTTP request scheme
    /// Example: "http", "https"
    pub const SCHEME: &str = "http.scheme";

    /// Value of the HTTP User-Agent header
    pub const USER_AGENT: &str = "http.user_agent";

    /// Size of the request payload body in bytes
    pub const REQUEST_CONTENT_LENGTH: &str = "http.request_content_length";

    /// Size of the response payload body in bytes
    pub const RESPONSE_CONTENT_LENGTH: &str = "http.response_content_length";
}

// =============================================================================
// Span Attributes - LLM Specific
// =============================================================================

/// Custom attributes for LLM operations
/// These follow the emerging OTEL GenAI semantic conventions
pub mod llm {
    /// Name of the LLM model being called
    /// Example: "gpt-4", "claude-3-sonnet", "llama-2-70b"
    pub const MODEL_NAME: &str = "llm.model";

    /// Provider of the LLM
    /// Example: "openai", "anthropic", "azure-openai"
    pub const PROVIDER: &str = "llm.provider";

    /// Type of LLM operation
    /// Example: "chat", "completion", "embedding"
    pub const OPERATION_TYPE: &str = "llm.operation_type";

    /// Whether the request is streaming
    pub const IS_STREAMING: &str = "llm.is_streaming";

    /// Total bytes received in the response
    pub const RESPONSE_BYTES: &str = "llm.response_bytes";

    /// Duration of the LLM call in milliseconds
    pub const DURATION_MS: &str = "llm.duration_ms";

    /// Time to first token in milliseconds (streaming only)
    pub const TIME_TO_FIRST_TOKEN_MS: &str = "llm.time_to_first_token";

    /// Number of prompt tokens used
    pub const PROMPT_TOKENS: &str = "llm.usage.prompt_tokens";

    /// Number of completion tokens generated
    pub const COMPLETION_TOKENS: &str = "llm.usage.completion_tokens";

    /// Total tokens used (prompt + completion)
    pub const TOTAL_TOKENS: &str = "llm.usage.total_tokens";

    /// Temperature parameter used
    pub const TEMPERATURE: &str = "llm.temperature";

    /// Max tokens parameter used
    pub const MAX_TOKENS: &str = "llm.max_tokens";

    /// Top-p parameter used
    pub const TOP_P: &str = "llm.top_p";

    /// List of tool names provided in the request
    pub const TOOLS: &str = "llm.tools";

    /// Preview of the user message (truncated)
    pub const USER_MESSAGE_PREVIEW: &str = "llm.user_message_preview";
}

// =============================================================================
// Span Attributes - Routing & Gateway
// =============================================================================

/// Attributes specific to LLM routing and gateway operations
pub mod routing {
    /// Strategy used to select the LLM endpoint
    /// Example: "round-robin", "least-latency", "cost-optimized"
    pub const STRATEGY: &str = "routing.strategy";

    /// Selected upstream endpoint
    pub const UPSTREAM_ENDPOINT: &str = "routing.upstream_endpoint";

    /// Time taken to determine the route in milliseconds
    pub const ROUTE_DETERMINATION_MS: &str = "routing.determination_ms";

    /// Whether a fallback endpoint was used
    pub const IS_FALLBACK: &str = "routing.is_fallback";

    /// Reason for route selection
    pub const SELECTION_REASON: &str = "routing.selection_reason";
}

// =============================================================================
// Span Attributes - Error Handling
// =============================================================================

/// Attributes for error and exception tracking
pub mod error {
    /// Whether an error occurred
    pub const ERROR: &str = "error";

    /// Type/class of the error
    /// Example: "TimeoutError", "AuthenticationError"
    pub const TYPE: &str = "error.type";

    /// Error message
    pub const MESSAGE: &str = "error.message";

    /// Stack trace of the error
    pub const STACK_TRACE: &str = "error.stack_trace";
}

// =============================================================================
// Span Attributes - Agentic Signals
// =============================================================================

/// Behavioral quality indicators for agent interactions
/// These signals are computed automatically from conversation patterns
pub mod signals {
    /// Overall quality assessment
    /// Values: "Excellent", "Good", "Neutral", "Poor", "Severe"
    pub const QUALITY: &str = "signals.quality";

    /// Total number of turns in the conversation
    pub const TURN_COUNT: &str = "signals.turn_count";

    /// Efficiency score (0.0-1.0)
    pub const EFFICIENCY_SCORE: &str = "signals.efficiency_score";

    /// Number of repair attempts detected
    pub const REPAIR_COUNT: &str = "signals.follow_up.repair.count";

    /// Ratio of repairs to user turns
    pub const REPAIR_RATIO: &str = "signals.follow_up.repair.ratio";

    /// Number of frustration indicators detected
    pub const FRUSTRATION_COUNT: &str = "signals.frustration.count";

    /// Frustration severity level (0-3)
    pub const FRUSTRATION_SEVERITY: &str = "signals.frustration.severity";

    /// Number of repetition instances detected
    pub const REPETITION_COUNT: &str = "signals.repetition.count";

    /// Whether escalation was requested (user asked for human help)
    pub const ESCALATION_REQUESTED: &str = "signals.escalation.requested";

    /// Number of positive feedback indicators detected
    pub const POSITIVE_FEEDBACK_COUNT: &str = "signals.positive_feedback.count";
}

// =============================================================================
// Operation Names
// =============================================================================

/// Canonical operation name components for Arch Gateway
pub mod operation_component {
    /// Inbound request handling
    pub const INBOUND: &str = "plano(inbound)";

    /// Orchestrator for llm route selection
    pub const ROUTING: &str = "plano(routing)";

    /// Orchestrator for agent selection
    pub const ORCHESTRATOR: &str = "plano(orchestrator)";

    /// Handoff to upstream service
    pub const HANDOFF: &str = "plano(handoff)";

    /// Agent filter execution
    pub const AGENT_FILTER: &str = "plano(filter)";

    /// Agent execution
    pub const AGENT: &str = "plano(agent)";

    /// LLM call
    pub const LLM: &str = "plano(llm)";
}

/// Builder for constructing standardized operation names
///
/// Format: `{method} {path} {target}`
///
/// The operation component (e.g., "archgw(llm)") is now part of the service name,
/// so the operation name focuses on the HTTP request details and target.
///
/// # Examples
/// ```
/// use brightstaff::tracing::OperationNameBuilder;
///
/// // LLM call operation: "POST /v1/chat/completions gpt-4"
/// // (service name will be "archgw(llm)")
/// let op = OperationNameBuilder::new()
///     .with_method("POST")
///     .with_path("/v1/chat/completions")
///     .with_target("gpt-4")
///     .build();
///
/// // Agent filter operation: "POST /agents/v1/chat/completions hallucination-detector"
/// // (service name will be "archgw(agent filter)")
/// let op = OperationNameBuilder::new()
///     .with_method("POST")
///     .with_path("/agents/v1/chat/completions")
///     .with_target("hallucination-detector")
///     .build();
///
/// // Routing operation: "POST /v1/chat/completions"
/// // (service name will be "archgw(routing)")
/// let op = OperationNameBuilder::new()
///     .with_method("POST")
///     .with_path("/v1/chat/completions")
///     .build();
/// ```
pub struct OperationNameBuilder {
    method: Option<String>,
    path: Option<String>,
    operation: Option<String>,
    target: Option<String>,
}

impl OperationNameBuilder {
    /// Create a new operation name builder
    pub fn new() -> Self {
        Self {
            method: None,
            path: None,
            operation: None,
            target: None,
        }
    }

    /// Set the HTTP method
    ///
    /// # Arguments
    /// * `method` - HTTP method (e.g., "GET", "POST", "PUT")
    pub fn with_method(mut self, method: impl Into<String>) -> Self {
        self.method = Some(method.into());
        self
    }

    /// Set the request path
    ///
    /// # Arguments
    /// * `path` - Request path (e.g., "/v1/chat/completions", "/agents/v1/chat/completions")
    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// Set the operation type (optional, for MCP operations)
    ///
    /// # Arguments
    /// * `operation` - Operation type (e.g., "tool_call", "session_init", "notification")
    pub fn with_operation(mut self, operation: impl Into<String>) -> Self {
        self.operation = Some(operation.into());
        self
    }

    /// Set the target (model name, agent name, or filter name)
    ///
    /// # Arguments
    /// * `target` - Target identifier (e.g., "gpt-4", "my-agent", "hallucination-detector")
    pub fn with_target(mut self, target: impl Into<String>) -> Self {
        self.target = Some(target.into());
        self
    }

    /// Build the operation name string
    ///
    /// # Format
    /// - With all components: `{method} {path} ({operation}) {target}`
    /// - Without operation: `{method} {path} {target}`
    /// - Without target: `{method} {path}`
    /// - Without path: `{method}`
    /// - Empty: returns empty string
    pub fn build(self) -> String {
        let mut parts = Vec::new();

        if let Some(method) = self.method {
            parts.push(method);
        }

        if let Some(path) = self.path {
            if let Some(operation) = self.operation {
                parts.push(format!("{} ({})", path, operation));
            } else {
                parts.push(path);
            }
        }

        if let Some(target) = self.target {
            parts.push(target);
        }

        parts.join(" ")
    }
}

impl Default for OperationNameBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_operation_name_full() {
        let op = OperationNameBuilder::new()
            .with_method("POST")
            .with_path("/v1/chat/completions")
            .with_target("gpt-4")
            .build();

        assert_eq!(op, "POST /v1/chat/completions gpt-4");
    }

    #[test]
    fn test_operation_name_no_target() {
        let op = OperationNameBuilder::new()
            .with_method("POST")
            .with_path("/v1/chat/completions")
            .build();

        assert_eq!(op, "POST /v1/chat/completions");
    }

    #[test]
    fn test_operation_name_agent_filter() {
        let op = OperationNameBuilder::new()
            .with_method("POST")
            .with_path("/agents/v1/chat/completions")
            .with_target("content-filter")
            .build();

        assert_eq!(op, "POST /agents/v1/chat/completions content-filter");
    }

    #[test]
    fn test_operation_name_minimal() {
        let op = OperationNameBuilder::new().build();
        assert_eq!(op, "");
    }
}
