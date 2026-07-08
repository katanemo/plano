use crate::apis::{AmazonBedrockApi, AnthropicApi, OpenAIApi};
use crate::clients::endpoints::{SupportedAPIsFromClient, SupportedUpstreamAPIs};
use serde::Deserialize;
use std::collections::HashMap;
use std::fmt::Display;
use std::sync::OnceLock;
use std::time::Duration;

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
    Astraflow,
    AstraflowCN,
    Minimax,
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
            "astraflow" => Ok(ProviderId::Astraflow),
            "astraflow_cn" => Ok(ProviderId::AstraflowCN),
            "minimax" => Ok(ProviderId::Minimax),
            _ => Err(format!("Unknown provider: {}", value)),
        }
    }
}

/// How Plano should mark a request for prompt caching, resolved from the *combination*
/// of gateway provider, the underlying model family, and the upstream API shape — not
/// the gateway alone. This is what lets DigitalOcean-Anthropic and OpenRouter-Anthropic
/// (both OpenAI-compatible chat completions fronting Anthropic models) cache through one
/// path, while OpenAI-family models stay correctly automatic and unimplemented backends
/// (Bedrock) are an honest `None` rather than a silent no-op.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheMarkerStrategy {
    /// No known prompt-caching support for this combination — do nothing.
    None,
    /// Provider caches stable prefixes automatically (OpenAI-family anywhere); no
    /// request markers are needed. Plano only keeps the prefix byte-stable and pinned.
    Automatic,
    /// OpenAI-compatible chat completions fronting Anthropic-family models
    /// (DigitalOcean, OpenRouter): attach `cache_control` to content parts.
    OpenAiContentPartCacheControl {
        /// Minimum cacheable prefix length in tokens; injecting below this is a no-op.
        min_prefix_tokens: u32,
        /// Optional cache lifetime hint ("5m" | "1h").
        ttl: Option<String>,
    },
    /// Native Anthropic Messages API (native `anthropic/*`, Vercel-Anthropic): inject
    /// ephemeral breakpoints on the Anthropic-shaped request.
    AnthropicMessagesBreakpoints {
        /// Maximum number of cache breakpoints the provider accepts per request.
        max_breakpoints: u8,
        /// Minimum cacheable prefix length in tokens; injecting below this is a no-op.
        min_prefix_tokens: u32,
    },
    // BedrockCachePoint { .. } // left as explicit `None` until implemented.
}

/// Coarse model family, inferred from the model id. Works across gateway naming
/// conventions (DigitalOcean's `anthropic-claude-…` dash form and OpenRouter's
/// `anthropic/claude-…` slash form) via substring matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModelFamily {
    Anthropic,
    OpenAI,
    Other,
}

fn model_family(model_name: &str) -> ModelFamily {
    let m = model_name.to_ascii_lowercase();
    if m.contains("claude") || m.contains("anthropic") {
        ModelFamily::Anthropic
    } else if m.contains("gpt") || m.contains("openai") || m.contains("chatgpt") {
        ModelFamily::OpenAI
    } else {
        ModelFamily::Other
    }
}

/// Whether a gateway accepts Anthropic-style `cache_control` on OpenAI content parts
/// over its chat-completions endpoint.
fn accepts_openai_content_part_cache_control(provider: ProviderId) -> bool {
    matches!(provider, ProviderId::DigitalOcean | ProviderId::OpenRouter)
}

/// Whether a gateway/model relies on automatic prefix caching (no markers required)
/// over an OpenAI-compatible surface.
fn is_automatic_cache_provider(provider: ProviderId) -> bool {
    matches!(
        provider,
        ProviderId::OpenAI
            | ProviderId::AzureOpenAI
            | ProviderId::ChatGPT
            | ProviderId::Groq
            | ProviderId::Deepseek
            | ProviderId::Gemini
            | ProviderId::Moonshotai
            | ProviderId::XAI
            | ProviderId::DigitalOcean
            | ProviderId::OpenRouter
    )
}

/// Resolve the cache-marking strategy for a `(gateway provider × underlying model ×
/// upstream API)` combination.
///
/// - `model_name` is the id *after* the gateway prefix (e.g. `anthropic-claude-3-5-sonnet`
///   for DigitalOcean, `anthropic/claude-3.5-sonnet` for OpenRouter).
pub fn cache_marker_strategy(
    provider: ProviderId,
    model_name: &str,
    upstream_api: &SupportedUpstreamAPIs,
) -> CacheMarkerStrategy {
    // Anthropic minimum cacheable prefix is ~1024 tokens (2048 for Haiku-class);
    // callers may raise this via config.
    const ANTHROPIC_MIN_PREFIX_TOKENS: u32 = 1024;

    match upstream_api {
        // Native Anthropic Messages API — inject ephemeral breakpoints.
        SupportedUpstreamAPIs::AnthropicMessagesAPI(_) => {
            CacheMarkerStrategy::AnthropicMessagesBreakpoints {
                max_breakpoints: 4,
                min_prefix_tokens: ANTHROPIC_MIN_PREFIX_TOKENS,
            }
        }
        // OpenAI-compatible chat completions — strategy depends on the model family.
        SupportedUpstreamAPIs::OpenAIChatCompletions(_) => match model_family(model_name) {
            ModelFamily::Anthropic if accepts_openai_content_part_cache_control(provider) => {
                CacheMarkerStrategy::OpenAiContentPartCacheControl {
                    min_prefix_tokens: ANTHROPIC_MIN_PREFIX_TOKENS,
                    ttl: None,
                }
            }
            // Anthropic-family behind a gateway that doesn't accept content-part
            // cache_control over chat completions: no honest way to mark it.
            ModelFamily::Anthropic => CacheMarkerStrategy::None,
            ModelFamily::OpenAI => CacheMarkerStrategy::Automatic,
            ModelFamily::Other if is_automatic_cache_provider(provider) => {
                CacheMarkerStrategy::Automatic
            }
            ModelFamily::Other => CacheMarkerStrategy::None,
        },
        // OpenAI Responses API — OpenAI-family automatic prefix caching.
        SupportedUpstreamAPIs::OpenAIResponsesAPI(_) => CacheMarkerStrategy::Automatic,
        // Bedrock cache points not yet implemented — honest None instead of a
        // silent no-op.
        SupportedUpstreamAPIs::AmazonBedrockConverse(_)
        | SupportedUpstreamAPIs::AmazonBedrockConverseStream(_) => CacheMarkerStrategy::None,
    }
}

/// Provider prompt-cache retention behavior, used to decide whether a session's
/// upstream cache is still plausibly warm from the time since it was last used.
///
/// This is deliberately time/behavior only — it says nothing about *how* to mark a
/// request for caching (that's [`CacheMarkerStrategy`]). Warmth is a function of the
/// idle gap vs the provider's cache window, so the session router can reason about
/// stickiness without ever seeing a provider response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderCacheCapability {
    /// Sliding idle window: the cache stays warm as long as it is touched at least
    /// this often. Anthropic's default ephemeral cache is 5 minutes.
    pub idle_ttl: Duration,
    /// Absolute ceiling on how long a cache entry can live regardless of activity.
    /// Conservative default of 1h matches Anthropic's extended (1h) tier ceiling.
    pub hard_ttl: Duration,
    /// Whether the provider (as configured) actually retains caches out to the
    /// extended window. Off by default — extended retention is opt-in per provider.
    pub extended_retention: bool,
    /// The extended idle window when `extended_retention` is enabled (e.g. 1h).
    pub extended_ttl: Duration,
}

impl Default for ProviderCacheCapability {
    fn default() -> Self {
        // Conservative, provider-agnostic defaults: a 5-minute sliding window capped
        // at 1 hour, no extended retention. Anything unknown is treated as short-lived
        // so the router doesn't over-stick to a cache that has likely gone cold.
        ProviderCacheCapability {
            idle_ttl: Duration::from_secs(5 * 60),
            hard_ttl: Duration::from_secs(60 * 60),
            extended_retention: false,
            extended_ttl: Duration::from_secs(60 * 60),
        }
    }
}

/// Resolve the prompt-cache retention window for a gateway provider. Data-driven so
/// tuning a provider's window needs no code changes at the call sites — only this
/// table. Unknown providers fall back to the conservative [`ProviderCacheCapability::default`].
pub fn provider_cache_capability(provider: ProviderId) -> ProviderCacheCapability {
    match provider {
        // Anthropic-family caches (native or fronted): 5-minute sliding default,
        // 1-hour hard ceiling. Extended (1h) retention is opt-in and left off here.
        ProviderId::Anthropic
        | ProviderId::DigitalOcean
        | ProviderId::OpenRouter
        | ProviderId::Vercel => ProviderCacheCapability::default(),
        // OpenAI-family automatic prefix caching also lives on the order of minutes;
        // the conservative default holds.
        ProviderId::OpenAI
        | ProviderId::AzureOpenAI
        | ProviderId::ChatGPT
        | ProviderId::Groq
        | ProviderId::Deepseek
        | ProviderId::Gemini
        | ProviderId::Moonshotai
        | ProviderId::XAI => ProviderCacheCapability::default(),
        _ => ProviderCacheCapability::default(),
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
            ProviderId::Minimax => "minimax",
            ProviderId::Astraflow | ProviderId::AstraflowCN => return Vec::new(),
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
                | ProviderId::ChatGPT
                | ProviderId::Astraflow
                | ProviderId::AstraflowCN
                | ProviderId::Minimax,
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
                | ProviderId::ChatGPT
                | ProviderId::Astraflow
                | ProviderId::AstraflowCN
                | ProviderId::Minimax,
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
            ProviderId::Astraflow => write!(f, "astraflow"),
            ProviderId::AstraflowCN => write!(f, "astraflow_cn"),
            ProviderId::Minimax => write!(f, "minimax"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apis::{AnthropicApi, OpenAIApi};

    fn chat_completions() -> SupportedUpstreamAPIs {
        SupportedUpstreamAPIs::OpenAIChatCompletions(OpenAIApi::ChatCompletions)
    }

    fn anthropic_messages() -> SupportedUpstreamAPIs {
        SupportedUpstreamAPIs::AnthropicMessagesAPI(AnthropicApi::Messages)
    }

    #[test]
    fn digitalocean_anthropic_uses_openai_content_part_markers() {
        // DO fronts Anthropic over an OpenAI-compatible surface (dash-form model id).
        let strategy = cache_marker_strategy(
            ProviderId::DigitalOcean,
            "anthropic-claude-3-5-sonnet",
            &chat_completions(),
        );
        assert!(matches!(
            strategy,
            CacheMarkerStrategy::OpenAiContentPartCacheControl { .. }
        ));
    }

    #[test]
    fn openrouter_anthropic_uses_openai_content_part_markers() {
        // OpenRouter uses slash-form model ids after the gateway prefix.
        let strategy = cache_marker_strategy(
            ProviderId::OpenRouter,
            "anthropic/claude-3.5-sonnet",
            &chat_completions(),
        );
        assert!(matches!(
            strategy,
            CacheMarkerStrategy::OpenAiContentPartCacheControl { .. }
        ));
    }

    #[test]
    fn openai_family_over_chat_completions_is_automatic() {
        assert_eq!(
            cache_marker_strategy(
                ProviderId::DigitalOcean,
                "openai-gpt-4o",
                &chat_completions()
            ),
            CacheMarkerStrategy::Automatic
        );
        assert_eq!(
            cache_marker_strategy(ProviderId::OpenAI, "gpt-4o", &chat_completions()),
            CacheMarkerStrategy::Automatic
        );
    }

    #[test]
    fn native_anthropic_uses_messages_breakpoints() {
        let strategy = cache_marker_strategy(
            ProviderId::Anthropic,
            "claude-3-5-sonnet-20241022",
            &anthropic_messages(),
        );
        assert!(matches!(
            strategy,
            CacheMarkerStrategy::AnthropicMessagesBreakpoints { .. }
        ));
    }

    #[test]
    fn anthropic_family_without_content_part_support_is_none() {
        // An Anthropic-family model over chat completions on a gateway that does not
        // accept content-part cache_control has no honest marking path.
        assert_eq!(
            cache_marker_strategy(
                ProviderId::Vercel,
                "anthropic/claude-3.5",
                &chat_completions()
            ),
            CacheMarkerStrategy::None
        );
    }

    #[test]
    fn bedrock_is_honest_none() {
        assert_eq!(
            cache_marker_strategy(
                ProviderId::AmazonBedrock,
                "anthropic.claude-3-5-sonnet",
                &SupportedUpstreamAPIs::AmazonBedrockConverse(
                    crate::apis::AmazonBedrockApi::Converse
                )
            ),
            CacheMarkerStrategy::None
        );
    }

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
    fn test_minimax_parsing_and_models() {
        assert_eq!(ProviderId::try_from("minimax"), Ok(ProviderId::Minimax));
        assert_eq!(ProviderId::Minimax.to_string(), "minimax");

        let models = ProviderId::Minimax.models();
        assert!(
            models.iter().any(|m| m == "MiniMax-M3"),
            "minimax models should include MiniMax-M3"
        );
        for model in &models {
            assert!(
                !model.contains('/'),
                "Model name '{}' should not contain provider prefix",
                model
            );
        }
    }

    #[test]
    fn test_minimax_compatible_api() {
        use crate::clients::endpoints::{SupportedAPIsFromClient, SupportedUpstreamAPIs};

        let openai_client =
            SupportedAPIsFromClient::OpenAIChatCompletions(OpenAIApi::ChatCompletions);
        let upstream = ProviderId::Minimax.compatible_api_for_client(&openai_client, false);
        assert!(
            matches!(upstream, SupportedUpstreamAPIs::OpenAIChatCompletions(_)),
            "minimax should map OpenAI client to OpenAIChatCompletions upstream"
        );

        let anthropic_client =
            SupportedAPIsFromClient::AnthropicMessagesAPI(AnthropicApi::Messages);
        let upstream = ProviderId::Minimax.compatible_api_for_client(&anthropic_client, false);
        assert!(
            matches!(upstream, SupportedUpstreamAPIs::OpenAIChatCompletions(_)),
            "minimax should translate Anthropic client to OpenAIChatCompletions upstream"
        );
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
