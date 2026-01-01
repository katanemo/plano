use crate::apis::{AmazonBedrockApi, AnthropicApi, ApiDefinition, OpenAIApi};
use crate::ProviderId;
use std::fmt;

/// Unified enum representing all supported API endpoints across providers
#[derive(Debug, Clone, PartialEq)]
pub enum SupportedAPIsFromClient {
    OpenAIChatCompletions(OpenAIApi),
    AnthropicMessagesAPI(AnthropicApi),
    OpenAIResponsesAPI(OpenAIApi),
}

#[derive(Debug, Clone, PartialEq)]
pub enum SupportedUpstreamAPIs {
    OpenAIChatCompletions(OpenAIApi),
    AnthropicMessagesAPI(AnthropicApi),
    AmazonBedrockConverse(AmazonBedrockApi),
    AmazonBedrockConverseStream(AmazonBedrockApi),
    OpenAIResponsesAPI(OpenAIApi),
}

impl fmt::Display for SupportedAPIsFromClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SupportedAPIsFromClient::OpenAIChatCompletions(api) => {
                write!(f, "OpenAI ({})", api.endpoint())
            }
            SupportedAPIsFromClient::AnthropicMessagesAPI(api) => {
                write!(f, "Anthropic AI ({})", api.endpoint())
            }
            SupportedAPIsFromClient::OpenAIResponsesAPI(api) => {
                write!(f, "OpenAI Responses ({})", api.endpoint())
            }
        }
    }
}

impl fmt::Display for SupportedUpstreamAPIs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SupportedUpstreamAPIs::OpenAIChatCompletions(api) => {
                write!(f, "OpenAI ({})", api.endpoint())
            }
            SupportedUpstreamAPIs::AnthropicMessagesAPI(api) => {
                write!(f, "Anthropic ({})", api.endpoint())
            }
            SupportedUpstreamAPIs::AmazonBedrockConverse(api) => {
                write!(f, "Amazon Bedrock ({})", api.endpoint())
            }
            SupportedUpstreamAPIs::AmazonBedrockConverseStream(api) => {
                write!(f, "Amazon Bedrock ({})", api.endpoint())
            }
            SupportedUpstreamAPIs::OpenAIResponsesAPI(api) => {
                write!(f, "OpenAI Responses ({})", api.endpoint())
            }
        }
    }
}

impl SupportedAPIsFromClient {
    /// Create a SupportedApi from an endpoint path
    pub fn from_endpoint(endpoint: &str) -> Option<Self> {
        if let Some(openai_api) = OpenAIApi::from_endpoint(endpoint) {
            // Check if this is the Responses API endpoint
            if openai_api == OpenAIApi::Responses {
                return Some(SupportedAPIsFromClient::OpenAIResponsesAPI(openai_api));
            }
            // Otherwise it's ChatCompletions
            return Some(SupportedAPIsFromClient::OpenAIChatCompletions(openai_api));
        }

        if let Some(anthropic_api) = AnthropicApi::from_endpoint(endpoint) {
            return Some(SupportedAPIsFromClient::AnthropicMessagesAPI(anthropic_api));
        }

        None
    }

    /// Get the endpoint path for this API
    pub fn endpoint(&self) -> &'static str {
        match self {
            SupportedAPIsFromClient::OpenAIChatCompletions(api) => api.endpoint(),
            SupportedAPIsFromClient::AnthropicMessagesAPI(api) => api.endpoint(),
            SupportedAPIsFromClient::OpenAIResponsesAPI(api) => api.endpoint(),
        }
    }

    pub fn target_endpoint_for_provider(
        &self,
        provider_id: &ProviderId,
        request_path: &str,
        model_id: &str,
        is_streaming: bool,
        base_url_path_prefix: Option<&str>,
    ) -> String {
        // Helper function to build endpoint with optional prefix override
        let build_endpoint = |provider_prefix: &str, suffix: &str| -> String {
            let prefix = base_url_path_prefix
                .map(|p| p.trim_matches('/'))
                .filter(|p| !p.is_empty())
                .unwrap_or(provider_prefix.trim_matches('/'));

            let suffix = suffix.trim_start_matches('/');
            if prefix.is_empty() {
                format!("/{}", suffix)
            } else {
                format!("/{}/{}", prefix, suffix)
            }
        };

        // Helper function to route based on provider with a specific endpoint suffix
        let route_by_provider = |endpoint_suffix: &str| -> String {
            match provider_id {
                ProviderId::Groq => {
                    if request_path.starts_with("/v1/") {
                        build_endpoint("/openai", request_path)
                    } else {
                        build_endpoint("/v1", endpoint_suffix)
                    }
                }
                ProviderId::Zhipu => {
                    if request_path.starts_with("/v1/") {
                        build_endpoint("/api/paas/v4", endpoint_suffix)
                    } else {
                        build_endpoint("/v1", endpoint_suffix)
                    }
                }
                ProviderId::Qwen => {
                    if request_path.starts_with("/v1/") {
                        build_endpoint("/compatible-mode/v1", endpoint_suffix)
                    } else {
                        build_endpoint("/v1", endpoint_suffix)
                    }
                }
                ProviderId::AzureOpenAI => {
                    if request_path.starts_with("/v1/") {
                        let suffix = endpoint_suffix.trim_start_matches('/');
                        build_endpoint(
                            "/openai/deployments",
                            &format!("/{}/{}?api-version=2025-01-01-preview", model_id, suffix),
                        )
                    } else {
                        build_endpoint("/v1", endpoint_suffix)
                    }
                }
                ProviderId::Gemini => {
                    if request_path.starts_with("/v1/") {
                        build_endpoint("/v1beta/openai", endpoint_suffix)
                    } else {
                        build_endpoint("/v1", endpoint_suffix)
                    }
                }
                ProviderId::AmazonBedrock => {
                    if request_path.starts_with("/v1/") {
                        if !is_streaming {
                            build_endpoint("", &format!("/model/{}/converse", model_id))
                        } else {
                            build_endpoint("", &format!("/model/{}/converse-stream", model_id))
                        }
                    } else {
                        build_endpoint("/v1", endpoint_suffix)
                    }
                }
                _ => build_endpoint("/v1", endpoint_suffix),
            }
        };

        match self {
            SupportedAPIsFromClient::AnthropicMessagesAPI(AnthropicApi::Messages) => {
                match provider_id {
                    ProviderId::Anthropic => build_endpoint("/v1", "/messages"),
                    ProviderId::AmazonBedrock => {
                        if request_path.starts_with("/v1/") && !is_streaming {
                            build_endpoint("", &format!("/model/{}/converse", model_id))
                        } else if request_path.starts_with("/v1/") && is_streaming {
                            build_endpoint("", &format!("/model/{}/converse-stream", model_id))
                        } else {
                            build_endpoint("/v1", "/chat/completions")
                        }
                    }
                    _ => build_endpoint("/v1", "/chat/completions"),
                }
            }
            SupportedAPIsFromClient::OpenAIResponsesAPI(_) => {
                // For Responses API, check if provider supports it, otherwise translate to chat/completions
                match provider_id {
                    // OpenAI and compatible providers that support /v1/responses
                    ProviderId::OpenAI => route_by_provider("/responses"),
                    // All other providers: translate to /chat/completions
                    _ => route_by_provider("/chat/completions"),
                }
            }
            SupportedAPIsFromClient::OpenAIChatCompletions(_) => {
                // For Chat Completions API, use the standard chat/completions path
                route_by_provider("/chat/completions")
            }
        }
    }
}

impl SupportedUpstreamAPIs {
    /// Create a SupportedUpstreamApi from an endpoint path
    pub fn from_endpoint(endpoint: &str) -> Option<Self> {
        if let Some(openai_api) = OpenAIApi::from_endpoint(endpoint) {
            // Check if this is the Responses API endpoint
            if openai_api == OpenAIApi::Responses {
                return Some(SupportedUpstreamAPIs::OpenAIResponsesAPI(openai_api));
            }
            // Otherwise it's ChatCompletions
            return Some(SupportedUpstreamAPIs::OpenAIChatCompletions(openai_api));
        }

        if let Some(anthropic_api) = AnthropicApi::from_endpoint(endpoint) {
            return Some(SupportedUpstreamAPIs::AnthropicMessagesAPI(anthropic_api));
        }

        if let Some(bedrock_api) = AmazonBedrockApi::from_endpoint(endpoint) {
            match bedrock_api {
                AmazonBedrockApi::Converse => {
                    return Some(SupportedUpstreamAPIs::AmazonBedrockConverse(bedrock_api))
                }
                AmazonBedrockApi::ConverseStream => {
                    return Some(SupportedUpstreamAPIs::AmazonBedrockConverseStream(
                        bedrock_api,
                    ))
                }
            }
        }

        None
    }
}

/// Get all supported endpoint paths
pub fn supported_endpoints() -> Vec<&'static str> {
    let mut endpoints = Vec::new();

    // Add all OpenAI endpoints
    for api in OpenAIApi::all_variants() {
        endpoints.push(api.endpoint());
    }

    // Add all Anthropic endpoints
    for api in AnthropicApi::all_variants() {
        endpoints.push(api.endpoint());
    }

    endpoints
}

/// Identify which provider supports a given endpoint
pub fn identify_provider(endpoint: &str) -> Option<&'static str> {
    if OpenAIApi::from_endpoint(endpoint).is_some() {
        return Some("openai");
    }

    if AnthropicApi::from_endpoint(endpoint).is_some() {
        return Some("anthropic");
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_supported_endpoint() {
        // OpenAI endpoints
        assert!(SupportedAPIsFromClient::from_endpoint("/v1/chat/completions").is_some());
        // Anthropic endpoints
        assert!(SupportedAPIsFromClient::from_endpoint("/v1/messages").is_some());

        // Unsupported endpoints
        assert!(SupportedAPIsFromClient::from_endpoint("/v1/unknown").is_none());
        assert!(SupportedAPIsFromClient::from_endpoint("/v2/chat").is_none());
        assert!(SupportedAPIsFromClient::from_endpoint("").is_none());
    }

    #[test]
    fn test_supported_endpoints() {
        let endpoints = supported_endpoints();
        assert_eq!(endpoints.len(), 3); // We have 3 APIs defined
        assert!(endpoints.contains(&"/v1/chat/completions"));
        assert!(endpoints.contains(&"/v1/messages"));
        assert!(endpoints.contains(&"/v1/responses"));
    }

    #[test]
    fn test_identify_provider() {
        assert_eq!(identify_provider("/v1/chat/completions"), Some("openai"));
        assert_eq!(identify_provider("/v1/messages"), Some("anthropic"));
        assert_eq!(identify_provider("/v1/unknown"), None);
    }

    #[test]
    fn test_endpoints_generated_from_api_definitions() {
        let endpoints = supported_endpoints();

        // Verify that we get endpoints from all API variants
        let openai_endpoints: Vec<_> = OpenAIApi::all_variants()
            .iter()
            .map(|api| api.endpoint())
            .collect();
        let anthropic_endpoints: Vec<_> = AnthropicApi::all_variants()
            .iter()
            .map(|api| api.endpoint())
            .collect();

        // All OpenAI endpoints should be in the result
        for endpoint in openai_endpoints {
            assert!(
                endpoints.contains(&endpoint),
                "Missing OpenAI endpoint: {}",
                endpoint
            );
        }

        // All Anthropic endpoints should be in the result
        for endpoint in anthropic_endpoints {
            assert!(
                endpoints.contains(&endpoint),
                "Missing Anthropic endpoint: {}",
                endpoint
            );
        }
        // Total should match
        assert_eq!(
            endpoints.len(),
            OpenAIApi::all_variants().len() + AnthropicApi::all_variants().len()
        );
    }

    #[test]
    fn test_target_endpoint_without_base_url_prefix() {
        let api = SupportedAPIsFromClient::OpenAIChatCompletions(OpenAIApi::ChatCompletions);

        // Test default OpenAI provider
        assert_eq!(
            api.target_endpoint_for_provider(
                &ProviderId::OpenAI,
                "/v1/chat/completions",
                "gpt-4",
                false,
                None
            ),
            "/v1/chat/completions"
        );

        // Test Groq provider
        assert_eq!(
            api.target_endpoint_for_provider(
                &ProviderId::Groq,
                "/v1/chat/completions",
                "llama2",
                false,
                None
            ),
            "/openai/v1/chat/completions"
        );

        // Test Zhipu provider
        assert_eq!(
            api.target_endpoint_for_provider(
                &ProviderId::Zhipu,
                "/v1/chat/completions",
                "chatglm",
                false,
                None
            ),
            "/api/paas/v4/chat/completions"
        );

        // Test Qwen provider
        assert_eq!(
            api.target_endpoint_for_provider(
                &ProviderId::Qwen,
                "/v1/chat/completions",
                "qwen-turbo",
                false,
                None
            ),
            "/compatible-mode/v1/chat/completions"
        );

        // Test Azure OpenAI provider
        assert_eq!(
            api.target_endpoint_for_provider(
                &ProviderId::AzureOpenAI,
                "/v1/chat/completions",
                "gpt-4",
                false,
                None
            ),
            "/openai/deployments/gpt-4/chat/completions?api-version=2025-01-01-preview"
        );

        // Test Gemini provider
        assert_eq!(
            api.target_endpoint_for_provider(
                &ProviderId::Gemini,
                "/v1/chat/completions",
                "gemini-pro",
                false,
                None
            ),
            "/v1beta/openai/chat/completions"
        );
    }

    #[test]
    fn test_target_endpoint_with_base_url_prefix() {
        let api = SupportedAPIsFromClient::OpenAIChatCompletions(OpenAIApi::ChatCompletions);

        // Test Zhipu with custom base_url_path_prefix
        assert_eq!(
            api.target_endpoint_for_provider(
                &ProviderId::Zhipu,
                "/v1/chat/completions",
                "chatglm",
                false,
                Some("/api/coding/paas/v4")
            ),
            "/api/coding/paas/v4/chat/completions"
        );

        // Test with prefix without leading slash
        assert_eq!(
            api.target_endpoint_for_provider(
                &ProviderId::Zhipu,
                "/v1/chat/completions",
                "chatglm",
                false,
                Some("api/coding/paas/v4")
            ),
            "/api/coding/paas/v4/chat/completions"
        );

        // Test with prefix with trailing slash
        assert_eq!(
            api.target_endpoint_for_provider(
                &ProviderId::Zhipu,
                "/v1/chat/completions",
                "chatglm",
                false,
                Some("/api/coding/paas/v4/")
            ),
            "/api/coding/paas/v4/chat/completions"
        );

        // Test OpenAI with custom prefix
        assert_eq!(
            api.target_endpoint_for_provider(
                &ProviderId::OpenAI,
                "/v1/chat/completions",
                "gpt-4",
                false,
                Some("/custom/api/v2")
            ),
            "/custom/api/v2/chat/completions"
        );

        // Test Groq with custom prefix
        assert_eq!(
            api.target_endpoint_for_provider(
                &ProviderId::Groq,
                "/v1/chat/completions",
                "llama2",
                false,
                Some("/api/v2")
            ),
            "/api/v2/v1/chat/completions"
        );
    }

    #[test]
    fn test_target_endpoint_with_empty_base_url_prefix() {
        let api = SupportedAPIsFromClient::OpenAIChatCompletions(OpenAIApi::ChatCompletions);

        // Test with just slashes - trims to empty, uses provider default
        assert_eq!(
            api.target_endpoint_for_provider(
                &ProviderId::Zhipu,
                "/v1/chat/completions",
                "chatglm",
                false,
                Some("/")
            ),
            "/api/paas/v4/chat/completions"
        );

        // Test with None - uses provider default
        assert_eq!(
            api.target_endpoint_for_provider(
                &ProviderId::Zhipu,
                "/v1/chat/completions",
                "chatglm",
                false,
                None
            ),
            "/api/paas/v4/chat/completions"
        );
    }

    #[test]
    fn test_amazon_bedrock_endpoints() {
        let api = SupportedAPIsFromClient::AnthropicMessagesAPI(AnthropicApi::Messages);

        // Test Bedrock non-streaming without prefix
        assert_eq!(
            api.target_endpoint_for_provider(
                &ProviderId::AmazonBedrock,
                "/v1/messages",
                "us.amazon.nova-pro-v1:0",
                false,
                None
            ),
            "/model/us.amazon.nova-pro-v1:0/converse"
        );

        // Test Bedrock streaming without prefix
        assert_eq!(
            api.target_endpoint_for_provider(
                &ProviderId::AmazonBedrock,
                "/v1/messages",
                "us.amazon.nova-pro-v1:0",
                true,
                None
            ),
            "/model/us.amazon.nova-pro-v1:0/converse-stream"
        );

        // Test Bedrock non-streaming with prefix (prefix shouldn't affect bedrock)
        assert_eq!(
            api.target_endpoint_for_provider(
                &ProviderId::AmazonBedrock,
                "/v1/messages",
                "us.amazon.nova-pro-v1:0",
                false,
                Some("/custom/path")
            ),
            "/custom/path/model/us.amazon.nova-pro-v1:0/converse"
        );

        // Test Bedrock streaming with prefix
        assert_eq!(
            api.target_endpoint_for_provider(
                &ProviderId::AmazonBedrock,
                "/v1/messages",
                "us.amazon.nova-pro-v1:0",
                true,
                Some("/custom/path")
            ),
            "/custom/path/model/us.amazon.nova-pro-v1:0/converse-stream"
        );
    }

    #[test]
    fn test_anthropic_messages_endpoint() {
        let api = SupportedAPIsFromClient::AnthropicMessagesAPI(AnthropicApi::Messages);

        // Test Anthropic without prefix
        assert_eq!(
            api.target_endpoint_for_provider(
                &ProviderId::Anthropic,
                "/v1/messages",
                "claude-3-opus",
                false,
                None
            ),
            "/v1/messages"
        );

        // Test Anthropic with prefix
        assert_eq!(
            api.target_endpoint_for_provider(
                &ProviderId::Anthropic,
                "/v1/messages",
                "claude-3-opus",
                false,
                Some("/api/v2")
            ),
            "/api/v2/messages"
        );
    }

    #[test]
    fn test_non_v1_request_paths() {
        let api = SupportedAPIsFromClient::OpenAIChatCompletions(OpenAIApi::ChatCompletions);

        // Test Groq with non-v1 path (should use default)
        assert_eq!(
            api.target_endpoint_for_provider(
                &ProviderId::Groq,
                "/custom/path",
                "llama2",
                false,
                None
            ),
            "/v1/chat/completions"
        );

        // Test Zhipu with non-v1 path
        assert_eq!(
            api.target_endpoint_for_provider(
                &ProviderId::Zhipu,
                "/custom/path",
                "chatglm",
                false,
                None
            ),
            "/v1/chat/completions"
        );

        // Test with prefix on non-v1 path
        assert_eq!(
            api.target_endpoint_for_provider(
                &ProviderId::Zhipu,
                "/custom/path",
                "chatglm",
                false,
                Some("/api/v2")
            ),
            "/api/v2/chat/completions"
        );
    }

    #[test]
    fn test_azure_openai_with_query_params() {
        let api = SupportedAPIsFromClient::OpenAIChatCompletions(OpenAIApi::ChatCompletions);

        // Test Azure without prefix - should include query params
        assert_eq!(
            api.target_endpoint_for_provider(
                &ProviderId::AzureOpenAI,
                "/v1/chat/completions",
                "gpt-4-deployment",
                false,
                None
            ),
            "/openai/deployments/gpt-4-deployment/chat/completions?api-version=2025-01-01-preview"
        );

        // Test Azure with prefix - prefix should replace /openai/deployments
        assert_eq!(
            api.target_endpoint_for_provider(
                &ProviderId::AzureOpenAI,
                "/v1/chat/completions",
                "gpt-4-deployment",
                false,
                Some("/custom/azure/path")
            ),
            "/custom/azure/path/gpt-4-deployment/chat/completions?api-version=2025-01-01-preview"
        );
    }
}
