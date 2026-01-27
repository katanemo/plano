use crate::apis::{AmazonBedrockApi, AnthropicApi, OpenAIApi};
use crate::clients::endpoints::{SupportedAPIsFromClient, SupportedUpstreamAPIs};
use serde::Deserialize;
use std::collections::HashMap;
use std::fmt::Display;
use std::sync::OnceLock;

static PROVIDER_MODELS_YAML: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/bin/provider_models.yaml"
));

#[derive(Deserialize)]
struct ProviderModelsFile {
    providers: HashMap<String, Vec<String>>,
}

fn load_provider_models() -> &'static HashMap<String, Vec<String>> {
    static MODELS: OnceLock<HashMap<String, Vec<String>>> = OnceLock::new();
    MODELS.get_or_init(|| {
        let ProviderModelsFile { providers } = serde_yaml::from_str(PROVIDER_MODELS_YAML)
            .expect("Failed to parse provider_models.yaml");
        providers
    })
}

/// Provider identifier enum - simple enum for identifying providers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderId {
    OpenAI,
    Mistral,
    Deepseek,
    Groq,
    Gemini,
    Anthropic,
    GitHub,
    Arch,
    AzureOpenAI,
    XAI,
    TogetherAI,
    Ollama,
    Moonshotai,
    Zhipu,
    Qwen,
    AmazonBedrock,
}

impl TryFrom<&str> for ProviderId {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value.to_lowercase().as_str() {
            "openai" => Ok(ProviderId::OpenAI),
            "mistral" => Ok(ProviderId::Mistral),
            "deepseek" => Ok(ProviderId::Deepseek),
            "groq" => Ok(ProviderId::Groq),
            "gemini" => Ok(ProviderId::Gemini),
            "google" => Ok(ProviderId::Gemini), // alias
            "anthropic" => Ok(ProviderId::Anthropic),
            "github" => Ok(ProviderId::GitHub),
            "arch" => Ok(ProviderId::Arch),
            "azure_openai" => Ok(ProviderId::AzureOpenAI),
            "xai" => Ok(ProviderId::XAI),
            "together_ai" => Ok(ProviderId::TogetherAI),
            "together" => Ok(ProviderId::TogetherAI), // alias
            "ollama" => Ok(ProviderId::Ollama),
            "moonshotai" => Ok(ProviderId::Moonshotai),
            "zhipu" => Ok(ProviderId::Zhipu),
            "qwen" => Ok(ProviderId::Qwen),
            "amazon_bedrock" => Ok(ProviderId::AmazonBedrock),
            "amazon" => Ok(ProviderId::AmazonBedrock), // alias
            _ => Err(format!("Unknown provider: {}", value)),
        }
    }
}

impl ProviderId {
    /// Get all available models for this provider
    /// Returns model names without the provider prefix (e.g., "gpt-4" not "openai/gpt-4")
    pub fn models(&self) -> Vec<String> {
        let provider_key = match self {
            ProviderId::AmazonBedrock => "amazon",
            ProviderId::AzureOpenAI => "openai",
            ProviderId::TogetherAI => "together",
            ProviderId::Gemini => "google",
            ProviderId::OpenAI => "openai",
            ProviderId::Anthropic => "anthropic",
            ProviderId::Mistral => "mistralai",
            ProviderId::Deepseek => "deepseek",
            ProviderId::Groq => "groq",
            ProviderId::XAI => "x-ai",
            ProviderId::Moonshotai => "moonshotai",
            ProviderId::Zhipu => "z-ai",
            ProviderId::Qwen => "qwen",
            _ => return Vec::new(),
        };

        load_provider_models()
            .get(provider_key)
            .map(|models| {
                models
                    .iter()
                    .filter_map(|model| {
                        // Strip provider prefix (e.g., "openai/gpt-4" -> "gpt-4")
                        model.split_once('/').map(|(_, name)| name.to_string())
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Given a client API, return the compatible upstream API for this provider
    pub fn compatible_api_for_client(
        &self,
        client_api: &SupportedAPIsFromClient,
        is_streaming: bool,
    ) -> SupportedUpstreamAPIs {
        match (self, client_api) {
            // Claude/Anthropic providers natively support Anthropic APIs
            (ProviderId::Anthropic, SupportedAPIsFromClient::AnthropicMessagesAPI(_)) => {
                SupportedUpstreamAPIs::AnthropicMessagesAPI(AnthropicApi::Messages)
            }
            (ProviderId::Anthropic, SupportedAPIsFromClient::OpenAIChatCompletions(_)) => {
                SupportedUpstreamAPIs::OpenAIChatCompletions(OpenAIApi::ChatCompletions)
            }

            // Anthropic doesn't support Responses API, fall back to chat completions
            (ProviderId::Anthropic, SupportedAPIsFromClient::OpenAIResponsesAPI(_)) => {
                SupportedUpstreamAPIs::OpenAIChatCompletions(OpenAIApi::ChatCompletions)
            }

            // OpenAI-compatible providers only support OpenAI chat completions
            (
                ProviderId::OpenAI
                | ProviderId::Groq
                | ProviderId::Mistral
                | ProviderId::Deepseek
                | ProviderId::Arch
                | ProviderId::Gemini
                | ProviderId::GitHub
                | ProviderId::AzureOpenAI
                | ProviderId::XAI
                | ProviderId::TogetherAI
                | ProviderId::Ollama
                | ProviderId::Moonshotai
                | ProviderId::Zhipu
                | ProviderId::Qwen,
                SupportedAPIsFromClient::AnthropicMessagesAPI(_),
            ) => SupportedUpstreamAPIs::OpenAIChatCompletions(OpenAIApi::ChatCompletions),

            (
                ProviderId::OpenAI
                | ProviderId::Groq
                | ProviderId::Mistral
                | ProviderId::Deepseek
                | ProviderId::Arch
                | ProviderId::Gemini
                | ProviderId::GitHub
                | ProviderId::AzureOpenAI
                | ProviderId::XAI
                | ProviderId::TogetherAI
                | ProviderId::Ollama
                | ProviderId::Moonshotai
                | ProviderId::Zhipu
                | ProviderId::Qwen,
                SupportedAPIsFromClient::OpenAIChatCompletions(_),
            ) => SupportedUpstreamAPIs::OpenAIChatCompletions(OpenAIApi::ChatCompletions),

            // OpenAI Responses API - only OpenAI supports this
            (ProviderId::OpenAI, SupportedAPIsFromClient::OpenAIResponsesAPI(_)) => {
                SupportedUpstreamAPIs::OpenAIResponsesAPI(OpenAIApi::Responses)
            }

            // Amazon Bedrock natively supports Bedrock APIs
            (ProviderId::AmazonBedrock, SupportedAPIsFromClient::OpenAIChatCompletions(_)) => {
                if is_streaming {
                    SupportedUpstreamAPIs::AmazonBedrockConverseStream(
                        AmazonBedrockApi::ConverseStream,
                    )
                } else {
                    SupportedUpstreamAPIs::AmazonBedrockConverse(AmazonBedrockApi::Converse)
                }
            }
            (ProviderId::AmazonBedrock, SupportedAPIsFromClient::AnthropicMessagesAPI(_)) => {
                if is_streaming {
                    SupportedUpstreamAPIs::AmazonBedrockConverseStream(
                        AmazonBedrockApi::ConverseStream,
                    )
                } else {
                    SupportedUpstreamAPIs::AmazonBedrockConverse(AmazonBedrockApi::Converse)
                }
            }
            (ProviderId::AmazonBedrock, SupportedAPIsFromClient::OpenAIResponsesAPI(_)) => {
                if is_streaming {
                    SupportedUpstreamAPIs::AmazonBedrockConverseStream(
                        AmazonBedrockApi::ConverseStream,
                    )
                } else {
                    SupportedUpstreamAPIs::AmazonBedrockConverse(AmazonBedrockApi::Converse)
                }
            }

            // Non-OpenAI providers: if client requested the Responses API, fall back to Chat Completions
            (_, SupportedAPIsFromClient::OpenAIResponsesAPI(_)) => {
                SupportedUpstreamAPIs::OpenAIChatCompletions(OpenAIApi::ChatCompletions)
            }
        }
    }
}

impl Display for ProviderId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderId::OpenAI => write!(f, "OpenAI"),
            ProviderId::Mistral => write!(f, "Mistral"),
            ProviderId::Deepseek => write!(f, "Deepseek"),
            ProviderId::Groq => write!(f, "Groq"),
            ProviderId::Gemini => write!(f, "Gemini"),
            ProviderId::Anthropic => write!(f, "Anthropic"),
            ProviderId::GitHub => write!(f, "GitHub"),
            ProviderId::Arch => write!(f, "Arch"),
            ProviderId::AzureOpenAI => write!(f, "azure_openai"),
            ProviderId::XAI => write!(f, "xai"),
            ProviderId::TogetherAI => write!(f, "together_ai"),
            ProviderId::Ollama => write!(f, "ollama"),
            ProviderId::Moonshotai => write!(f, "moonshotai"),
            ProviderId::Zhipu => write!(f, "zhipu"),
            ProviderId::Qwen => write!(f, "qwen"),
            ProviderId::AmazonBedrock => write!(f, "amazon_bedrock"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_models_loaded_from_yaml() {
        // Test that we can load models for each supported provider
        let openai_models = ProviderId::OpenAI.models();
        assert!(!openai_models.is_empty(), "OpenAI should have models");

        let anthropic_models = ProviderId::Anthropic.models();
        assert!(!anthropic_models.is_empty(), "Anthropic should have models");

        let mistral_models = ProviderId::Mistral.models();
        assert!(!mistral_models.is_empty(), "Mistral should have models");

        let deepseek_models = ProviderId::Deepseek.models();
        assert!(!deepseek_models.is_empty(), "Deepseek should have models");

        let gemini_models = ProviderId::Gemini.models();
        assert!(!gemini_models.is_empty(), "Gemini should have models");
    }

    #[test]
    fn test_model_names_without_provider_prefix() {
        // Test that model names don't include the provider/ prefix
        let openai_models = ProviderId::OpenAI.models();
        for model in &openai_models {
            assert!(
                !model.contains('/'),
                "Model name '{}' should not contain provider prefix",
                model
            );
        }

        let anthropic_models = ProviderId::Anthropic.models();
        for model in &anthropic_models {
            assert!(
                !model.contains('/'),
                "Model name '{}' should not contain provider prefix",
                model
            );
        }
    }

    #[test]
    fn test_specific_models_exist() {
        // Test that specific well-known models are present
        let openai_models = ProviderId::OpenAI.models();
        let has_gpt4 = openai_models.iter().any(|m| m.contains("gpt-4"));
        assert!(has_gpt4, "OpenAI models should include GPT-4 variants");

        let anthropic_models = ProviderId::Anthropic.models();
        let has_claude = anthropic_models.iter().any(|m| m.contains("claude"));
        assert!(
            has_claude,
            "Anthropic models should include Claude variants"
        );
    }

    #[test]
    fn test_unsupported_providers_return_empty() {
        // Providers without models should return empty vec
        let github_models = ProviderId::GitHub.models();
        assert!(
            github_models.is_empty(),
            "GitHub should return empty models list"
        );

        let ollama_models = ProviderId::Ollama.models();
        assert!(
            ollama_models.is_empty(),
            "Ollama should return empty models list"
        );
    }

    #[test]
    fn test_provider_name_mapping() {
        // Test that provider key mappings work correctly
        let xai_models = ProviderId::XAI.models();
        assert!(
            !xai_models.is_empty(),
            "XAI should have models (mapped to x-ai)"
        );

        let zhipu_models = ProviderId::Zhipu.models();
        assert!(
            !zhipu_models.is_empty(),
            "Zhipu should have models (mapped to z-ai)"
        );

        let amazon_models = ProviderId::AmazonBedrock.models();
        assert!(
            !amazon_models.is_empty(),
            "AmazonBedrock should have models (mapped to amazon)"
        );
    }
}
