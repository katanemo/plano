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
    Xiaomi,
    Mistral,
    Deepseek,
    Groq,
    Gemini,
    Anthropic,
    GitHub,
    Plano,
    AzureOpenAI,
    XAI,
    TogetherAI,
    Ollama,
    Moonshotai,
    Zhipu,
    Qwen,
    AmazonBedrock,
    ChatGPT,
    DigitalOcean,
    Vercel,
    OpenRouter,
}

impl TryFrom<&str> for ProviderId {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value.to_lowercase().as_str() {
            "openai" => Ok(ProviderId::OpenAI),
            "xiaomi" => Ok(ProviderId::Xiaomi),
            "mistral" => Ok(ProviderId::Mistral),
            "deepseek" => Ok(ProviderId::Deepseek),
            "groq" => Ok(ProviderId::Groq),
            "gemini" => Ok(ProviderId::Gemini),
            "google" => Ok(ProviderId::Gemini), // alias
            "anthropic" => Ok(ProviderId::Anthropic),
            "github" => Ok(ProviderId::GitHub),
            "plano" => Ok(ProviderId::Plano),
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
            "chatgpt" => Ok(ProviderId::ChatGPT),
            "digitalocean" => Ok(ProviderId::DigitalOcean),
            "do" => Ok(ProviderId::DigitalOcean),    // alias
            "do_ai" => Ok(ProviderId::DigitalOcean), // alias
            "vercel" => Ok(ProviderId::Vercel),
            "openrouter" => Ok(ProviderId::OpenRouter),
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
            ProviderId::Xiaomi => "xiaomi",
            ProviderId::Anthropic => "anthropic",
            ProviderId::Mistral => "mistralai",
            ProviderId::Deepseek => "deepseek",
            ProviderId::Groq => "groq",
            ProviderId::XAI => "x-ai",
            ProviderId::Moonshotai => "moonshotai",
            ProviderId::Zhipu => "z-ai",
            ProviderId::Qwen => "qwen",
            ProviderId::ChatGPT => "chatgpt",
            ProviderId::DigitalOcean => "digitalocean",
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

            // Vercel AI Gateway natively supports all three API types
            (ProviderId::Vercel, SupportedAPIsFromClient::AnthropicMessagesAPI(_)) => {
                SupportedUpstreamAPIs::AnthropicMessagesAPI(AnthropicApi::Messages)
            }
            (ProviderId::Vercel, SupportedAPIsFromClient::OpenAIChatCompletions(_)) => {
                SupportedUpstreamAPIs::OpenAIChatCompletions(OpenAIApi::ChatCompletions)
            }
            (ProviderId::Vercel, SupportedAPIsFromClient::OpenAIResponsesAPI(_)) => {
                SupportedUpstreamAPIs::OpenAIResponsesAPI(OpenAIApi::Responses)
            }

            // OpenAI-compatible providers only support OpenAI chat completions
            (
                ProviderId::OpenAI
                | ProviderId::Xiaomi
                | ProviderId::Groq
                | ProviderId::Mistral
                | ProviderId::Deepseek
                | ProviderId::Plano
                | ProviderId::Gemini
                | ProviderId::GitHub
                | ProviderId::AzureOpenAI
                | ProviderId::XAI
                | ProviderId::TogetherAI
                | ProviderId::Ollama
                | ProviderId::Moonshotai
                | ProviderId::Zhipu
                | ProviderId::Qwen
                | ProviderId::DigitalOcean
                | ProviderId::OpenRouter
                | ProviderId::ChatGPT,
                SupportedAPIsFromClient::AnthropicMessagesAPI(_),
            ) => SupportedUpstreamAPIs::OpenAIChatCompletions(OpenAIApi::ChatCompletions),

            (
                ProviderId::OpenAI
                | ProviderId::Xiaomi
                | ProviderId::Groq
                | ProviderId::Mistral
                | ProviderId::Deepseek
                | ProviderId::Plano
                | ProviderId::Gemini
                | ProviderId::GitHub
                | ProviderId::AzureOpenAI
                | ProviderId::XAI
                | ProviderId::TogetherAI
                | ProviderId::Ollama
                | ProviderId::Moonshotai
                | ProviderId::Zhipu
                | ProviderId::Qwen
                | ProviderId::DigitalOcean
                | ProviderId::OpenRouter
                | ProviderId::ChatGPT,
                SupportedAPIsFromClient::OpenAIChatCompletions(_),
            ) => SupportedUpstreamAPIs::OpenAIChatCompletions(OpenAIApi::ChatCompletions),

            // OpenAI Responses API - OpenAI, xAI, and ChatGPT support this natively
            (
                ProviderId::OpenAI | ProviderId::XAI | ProviderId::ChatGPT,
                SupportedAPIsFromClient::OpenAIResponsesAPI(_),
            ) => SupportedUpstreamAPIs::OpenAIResponsesAPI(OpenAIApi::Responses),

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
            ProviderId::Xiaomi => write!(f, "xiaomi"),
            ProviderId::Mistral => write!(f, "Mistral"),
            ProviderId::Deepseek => write!(f, "Deepseek"),
            ProviderId::Groq => write!(f, "Groq"),
            ProviderId::Gemini => write!(f, "Gemini"),
            ProviderId::Anthropic => write!(f, "Anthropic"),
            ProviderId::GitHub => write!(f, "GitHub"),
            ProviderId::Plano => write!(f, "Plano"),
            ProviderId::AzureOpenAI => write!(f, "azure_openai"),
            ProviderId::XAI => write!(f, "xai"),
            ProviderId::TogetherAI => write!(f, "together_ai"),
            ProviderId::Ollama => write!(f, "ollama"),
            ProviderId::Moonshotai => write!(f, "moonshotai"),
            ProviderId::Zhipu => write!(f, "zhipu"),
            ProviderId::Qwen => write!(f, "qwen"),
            ProviderId::AmazonBedrock => write!(f, "amazon_bedrock"),
            ProviderId::ChatGPT => write!(f, "chatgpt"),
            ProviderId::DigitalOcean => write!(f, "digitalocean"),
            ProviderId::Vercel => write!(f, "vercel"),
            ProviderId::OpenRouter => write!(f, "openrouter"),
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

    #[test]
    fn test_vercel_and_openrouter_parsing() {
        assert_eq!(ProviderId::try_from("vercel"), Ok(ProviderId::Vercel));
        assert!(ProviderId::try_from("vercel_ai").is_err());
        assert_eq!(
            ProviderId::try_from("openrouter"),
            Ok(ProviderId::OpenRouter)
        );
        assert!(ProviderId::try_from("open_router").is_err());
    }

    #[test]
    fn test_vercel_compatible_api() {
        use crate::clients::endpoints::{SupportedAPIsFromClient, SupportedUpstreamAPIs};

        let openai_client =
            SupportedAPIsFromClient::OpenAIChatCompletions(OpenAIApi::ChatCompletions);
        let upstream = ProviderId::Vercel.compatible_api_for_client(&openai_client, false);
        assert!(
            matches!(upstream, SupportedUpstreamAPIs::OpenAIChatCompletions(_)),
            "Vercel should map OpenAI client to OpenAIChatCompletions upstream"
        );

        let anthropic_client =
            SupportedAPIsFromClient::AnthropicMessagesAPI(AnthropicApi::Messages);
        let upstream = ProviderId::Vercel.compatible_api_for_client(&anthropic_client, false);
        assert!(
            matches!(upstream, SupportedUpstreamAPIs::AnthropicMessagesAPI(_)),
            "Vercel should map Anthropic client to AnthropicMessagesAPI upstream natively"
        );

        let responses_client = SupportedAPIsFromClient::OpenAIResponsesAPI(OpenAIApi::Responses);
        let upstream = ProviderId::Vercel.compatible_api_for_client(&responses_client, false);
        assert!(
            matches!(upstream, SupportedUpstreamAPIs::OpenAIResponsesAPI(_)),
            "Vercel should map Responses API client to OpenAIResponsesAPI upstream natively"
        );
    }

    #[test]
    fn test_openrouter_compatible_api() {
        use crate::clients::endpoints::{SupportedAPIsFromClient, SupportedUpstreamAPIs};

        let openai_client =
            SupportedAPIsFromClient::OpenAIChatCompletions(OpenAIApi::ChatCompletions);
        let upstream = ProviderId::OpenRouter.compatible_api_for_client(&openai_client, false);
        assert!(
            matches!(upstream, SupportedUpstreamAPIs::OpenAIChatCompletions(_)),
            "OpenRouter should map OpenAI client to OpenAIChatCompletions upstream"
        );

        let anthropic_client =
            SupportedAPIsFromClient::AnthropicMessagesAPI(AnthropicApi::Messages);
        let upstream = ProviderId::OpenRouter.compatible_api_for_client(&anthropic_client, false);
        assert!(
            matches!(upstream, SupportedUpstreamAPIs::OpenAIChatCompletions(_)),
            "OpenRouter should translate Anthropic client to OpenAIChatCompletions upstream"
        );

        let responses_client = SupportedAPIsFromClient::OpenAIResponsesAPI(OpenAIApi::Responses);
        let upstream = ProviderId::OpenRouter.compatible_api_for_client(&responses_client, false);
        assert!(
            matches!(upstream, SupportedUpstreamAPIs::OpenAIChatCompletions(_)),
            "OpenRouter should translate Responses API client to OpenAIChatCompletions upstream"
        );
    }

    #[test]
    fn test_vercel_and_openrouter_empty_models() {
        assert!(ProviderId::Vercel.models().is_empty());
        assert!(ProviderId::OpenRouter.models().is_empty());
    }

    #[test]
    fn test_xai_uses_responses_api_for_responses_clients() {
        use crate::clients::endpoints::{SupportedAPIsFromClient, SupportedUpstreamAPIs};

        let client_api = SupportedAPIsFromClient::OpenAIResponsesAPI(OpenAIApi::Responses);
        let upstream = ProviderId::XAI.compatible_api_for_client(&client_api, false);
        assert!(matches!(
            upstream,
            SupportedUpstreamAPIs::OpenAIResponsesAPI(OpenAIApi::Responses)
        ));
    }

    #[test]
    fn test_chatgpt_uses_responses_api_for_responses_clients() {
        use crate::clients::endpoints::{SupportedAPIsFromClient, SupportedUpstreamAPIs};

        let client_api = SupportedAPIsFromClient::OpenAIResponsesAPI(OpenAIApi::Responses);
        let upstream = ProviderId::ChatGPT.compatible_api_for_client(&client_api, false);
        assert!(matches!(
            upstream,
            SupportedUpstreamAPIs::OpenAIResponsesAPI(OpenAIApi::Responses)
        ));
    }
}
