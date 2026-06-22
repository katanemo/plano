//! Model capability metadata (Tier 1 routing).
//!
//! Capabilities are objective, stable properties of a model: which modalities it
//! accepts/produces and how large a context window it supports. They are sourced
//! at runtime from [models.dev](https://models.dev) (fetched by brightstaff's
//! `ModelCapabilitiesService`, mirroring how DigitalOcean pricing is fetched) and
//! can be overridden per-model by user config. This module owns only the snapshot
//! parsing and the canonical lookup — no data is vendored into the binary.
//!
//! Precedence (applied by the caller, e.g. brightstaff's `ModelCapabilitiesService`):
//! `user config capabilities > models.dev > conservative default`.

use crate::providers::id::ProviderId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Objective, stable per-model capabilities. All fields are optional so that a
/// user-config override can be merged field-by-field over the models.dev default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCapabilities {
    /// Maximum input context window (tokens).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u32>,
    /// Maximum output tokens the model will emit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    /// Accepts image input (vision).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_vision: Option<bool>,
    /// Produces images (`/v1/images/generations`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_image_generation: Option<bool>,
    /// Produces audio (`/v1/audio/speech`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_audio_out: Option<bool>,
}

impl ModelCapabilities {
    /// Resolve vision support, defaulting to text-only (`false`) when unknown.
    pub fn vision(&self) -> bool {
        self.supports_vision.unwrap_or(false)
    }

    /// Resolve image-generation support, defaulting to `false` when unknown.
    pub fn image_generation(&self) -> bool {
        self.supports_image_generation.unwrap_or(false)
    }

    /// Resolve audio-out support, defaulting to `false` when unknown.
    pub fn audio_out(&self) -> bool {
        self.supports_audio_out.unwrap_or(false)
    }

    /// Known context window, treating `0`/absent as "unknown" (no constraint).
    pub fn window(&self) -> Option<u32> {
        self.context_window.filter(|&w| w > 0)
    }

    /// Fill any `None` field on `self` from `fallback`. Used to apply precedence:
    /// `user.fill_from(models_dev)` keeps user-set fields and backfills the rest.
    pub fn fill_from(&self, fallback: &ModelCapabilities) -> ModelCapabilities {
        ModelCapabilities {
            context_window: self.context_window.or(fallback.context_window),
            max_output_tokens: self.max_output_tokens.or(fallback.max_output_tokens),
            supports_vision: self.supports_vision.or(fallback.supports_vision),
            supports_image_generation: self
                .supports_image_generation
                .or(fallback.supports_image_generation),
            supports_audio_out: self.supports_audio_out.or(fallback.supports_audio_out),
        }
    }
}

/// The capability requirements implied by a single request's shape (Tier 1).
/// Computed from the endpoint + request content; checked against each candidate
/// model's [`ModelCapabilities`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RequiredCapabilities {
    /// Request carries image input (vision).
    pub vision: bool,
    /// Request targets image generation (`/v1/images/generations`).
    pub image_out: bool,
    /// Request targets audio/TTS output (`/v1/audio/speech`).
    pub audio_out: bool,
    /// Minimum input context window required (estimated token count).
    pub min_context_tokens: usize,
}

impl RequiredCapabilities {
    /// Derive the modality requirements implied by an endpoint path. Vision and
    /// context-token requirements are request-content-derived and set separately.
    pub fn for_endpoint(path: &str) -> Self {
        RequiredCapabilities {
            image_out: path.contains("/images/generations"),
            audio_out: path.contains("/audio/speech"),
            ..Default::default()
        }
    }

    /// True when this request imposes no capability constraints (plain text chat
    /// that fits any window) — lets callers skip filtering entirely.
    pub fn is_unconstrained(&self) -> bool {
        !self.vision && !self.image_out && !self.audio_out && self.min_context_tokens == 0
    }

    /// Whether a model with the given capabilities can serve this request.
    /// Unknown context windows are treated permissively (conservative default):
    /// we only eliminate a model when we can *prove* the window is too small.
    pub fn satisfied_by(&self, caps: &ModelCapabilities) -> bool {
        if self.vision && !caps.vision() {
            return false;
        }
        if self.image_out && !caps.image_generation() {
            return false;
        }
        if self.audio_out && !caps.audio_out() {
            return false;
        }
        if self.min_context_tokens > 0 {
            if let Some(window) = caps.window() {
                if self.min_context_tokens as u64 > window as u64 {
                    return false;
                }
            }
        }
        true
    }

    /// Human-readable description of the unmet requirement(s) for error messages.
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        if self.vision {
            parts.push("vision input".to_string());
        }
        if self.image_out {
            parts.push("image generation".to_string());
        }
        if self.audio_out {
            parts.push("audio output".to_string());
        }
        if self.min_context_tokens > 0 {
            parts.push(format!(
                "context window >= {} tokens",
                self.min_context_tokens
            ));
        }
        if parts.is_empty() {
            "no special capabilities".to_string()
        } else {
            parts.join(", ")
        }
    }
}

/// On-disk shape of the vendored snapshot / models.dev refresh output.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CapabilitiesSnapshot {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub generated: String,
    /// Keyed by canonical `"<provider>/<model_id>"` (see [`ProviderId::canonical_key`]).
    #[serde(default)]
    pub models: HashMap<String, ModelCapabilities>,
}

impl CapabilitiesSnapshot {
    /// Parse a pre-built (canonical-keyed) snapshot from JSON bytes, i.e. the
    /// `{ "models": { "<provider>/<id>": { ... } } }` shape this module emits.
    pub fn from_json_slice(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }

    /// Build a snapshot directly from a raw `models.dev` `api.json` payload,
    /// applying the provider-key alias map and modality/limit mapping. Used by
    /// the brightstaff runtime fetch/refresh.
    pub fn from_models_dev_json(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        let providers: HashMap<String, ModelsDevProvider> = serde_json::from_slice(bytes)?;
        let mut models = HashMap::new();
        for (pkey, provider) in providers {
            let Some(canon) = models_dev_provider_to_canonical(&pkey) else {
                continue; // unmapped / aggregator provider
            };
            for (mid, m) in provider.models {
                models.insert(format!("{}/{}", canon, mid), m.into_capabilities());
            }
        }
        Ok(CapabilitiesSnapshot {
            version: "1.0".to_string(),
            source: "models.dev".to_string(),
            generated: String::new(),
            models,
        })
    }
}

/// models.dev provider key -> Plano canonical provider token (matches
/// [`ProviderId::canonical_key`]). Aggregator/unmapped keys return `None`.
pub fn models_dev_provider_to_canonical(provider_key: &str) -> Option<&'static str> {
    Some(match provider_key {
        "openai" => "openai",
        "anthropic" => "anthropic",
        "google" => "gemini",
        "mistral" => "mistral",
        "groq" => "groq",
        "xai" => "xai",
        "deepseek" => "deepseek",
        "moonshotai" => "moonshotai",
        "zhipuai" => "zhipu",
        "xiaomi" => "xiaomi",
        "togetherai" => "together_ai",
        "amazon-bedrock" => "amazon_bedrock",
        "digitalocean" => "digitalocean",
        "openrouter" => "openrouter",
        "vercel" => "vercel",
        "github-models" => "github",
        "alibaba" => "qwen",
        _ => return None,
    })
}

/// Raw models.dev per-provider shape (only the fields we map).
#[derive(Debug, Deserialize)]
struct ModelsDevProvider {
    #[serde(default)]
    models: HashMap<String, ModelsDevModel>,
}

#[derive(Debug, Deserialize)]
struct ModelsDevModel {
    #[serde(default)]
    modalities: ModelsDevModalities,
    #[serde(default)]
    limit: ModelsDevLimit,
}

#[derive(Debug, Default, Deserialize)]
struct ModelsDevModalities {
    #[serde(default)]
    input: Vec<String>,
    #[serde(default)]
    output: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ModelsDevLimit {
    #[serde(default)]
    context: Option<u32>,
    #[serde(default)]
    output: Option<u32>,
}

impl ModelsDevModel {
    fn into_capabilities(self) -> ModelCapabilities {
        ModelCapabilities {
            context_window: self.limit.context,
            max_output_tokens: self.limit.output,
            supports_vision: Some(self.modalities.input.iter().any(|s| s == "image")),
            supports_image_generation: Some(self.modalities.output.iter().any(|s| s == "image")),
            supports_audio_out: Some(self.modalities.output.iter().any(|s| s == "audio")),
        }
    }
}

/// In-memory capability catalog keyed by canonical `"<provider>/<model_id>"`.
#[derive(Debug, Clone, Default)]
pub struct CapabilitiesCatalog {
    models: HashMap<String, ModelCapabilities>,
}

impl CapabilitiesCatalog {
    pub fn new(models: HashMap<String, ModelCapabilities>) -> Self {
        Self { models }
    }

    pub fn from_snapshot(snapshot: CapabilitiesSnapshot) -> Self {
        Self::new(snapshot.models)
    }

    /// Number of models in the catalog.
    pub fn len(&self) -> usize {
        self.models.len()
    }

    pub fn is_empty(&self) -> bool {
        self.models.is_empty()
    }

    /// Look up capabilities for a `"<provider>/<model_id>"` string, normalizing
    /// the provider token to its canonical form. Returns `None` when the provider
    /// is unknown or the model is absent from the catalog.
    pub fn get(&self, model: &str) -> Option<&ModelCapabilities> {
        let key = canonical_model_key(model)?;
        self.models.get(&key)
    }
}

/// Normalize a `"<provider>/<model_id>"` string into the canonical catalog key
/// `"<canonical_provider>/<model_id>"`. Splits on the first `/` so model ids that
/// themselves contain `/` (e.g. `meta-llama/Llama-3.3-70B`) are preserved.
pub fn canonical_model_key(model: &str) -> Option<String> {
    let (provider, model_id) = model.split_once('/')?;
    let canonical = ProviderId::try_from(provider).ok()?.canonical_key();
    Some(format!("{}/{}", canonical, model_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_key_normalizes_provider_aliases() {
        // google -> gemini, x-ai handled via try_from
        assert_eq!(
            canonical_model_key("google/gemini-2.5-pro").as_deref(),
            Some("gemini/gemini-2.5-pro")
        );
        assert_eq!(
            canonical_model_key("openai/gpt-4o").as_deref(),
            Some("openai/gpt-4o")
        );
        // model id containing a slash is preserved
        assert_eq!(
            canonical_model_key("together_ai/meta-llama/Llama-3.3-70B").as_deref(),
            Some("together_ai/meta-llama/Llama-3.3-70B")
        );
        // unknown provider -> None
        assert!(canonical_model_key("notaprovider/foo").is_none());
        // no provider prefix -> None
        assert!(canonical_model_key("gpt-4o").is_none());
    }

    #[test]
    fn catalog_resolves_known_model_and_defaults_unknown() {
        // A catalog built from a models.dev payload resolves known models by
        // canonical key; absent models fall back to the conservative default.
        let raw = br#"{
            "openai": { "models": {
                "gpt-4o": {
                    "modalities": { "input": ["text","image"], "output": ["text"] },
                    "limit": { "context": 128000, "output": 16384 }
                }
            }}
        }"#;
        let snapshot = CapabilitiesSnapshot::from_models_dev_json(raw).unwrap();
        let catalog = CapabilitiesCatalog::from_snapshot(snapshot);

        let caps = catalog.get("openai/gpt-4o").cloned().unwrap_or_default();
        assert!(caps.vision());
        assert!(!caps.image_generation());
        assert_eq!(caps.window(), Some(128000));

        // Unknown model -> conservative default (text-only, unknown window).
        let missing = catalog
            .get("openai/totally-made-up-model")
            .cloned()
            .unwrap_or_default();
        assert!(!missing.vision());
        assert!(!missing.audio_out());
        assert_eq!(missing.window(), None);
    }

    #[test]
    fn models_dev_raw_json_maps_to_canonical_capabilities() {
        let raw = br#"{
            "google": { "models": {
                "gemini-2.5-pro": {
                    "modalities": { "input": ["text","image"], "output": ["text"] },
                    "limit": { "context": 1048576, "output": 65536 }
                }
            }},
            "requesty": { "models": {
                "openai/gpt-4o": { "modalities": { "input": ["text"], "output": ["text"] } }
            }}
        }"#;
        let snapshot = CapabilitiesSnapshot::from_models_dev_json(raw).unwrap();
        // google -> gemini canonical key
        let caps = snapshot.models.get("gemini/gemini-2.5-pro").unwrap();
        assert_eq!(caps.supports_vision, Some(true));
        assert_eq!(caps.context_window, Some(1048576));
        assert_eq!(caps.max_output_tokens, Some(65536));
        // aggregator provider "requesty" is skipped
        assert!(snapshot.models.keys().all(|k| !k.contains("requesty")));
    }

    #[test]
    fn provider_alias_map_matches_canonical_keys() {
        assert_eq!(models_dev_provider_to_canonical("google"), Some("gemini"));
        assert_eq!(
            models_dev_provider_to_canonical("amazon-bedrock"),
            Some("amazon_bedrock")
        );
        assert_eq!(models_dev_provider_to_canonical("requesty"), None);
        // Alias targets must be valid canonical provider tokens (round-trip).
        for key in ["google", "amazon-bedrock", "togetherai", "github-models"] {
            let canon = models_dev_provider_to_canonical(key).unwrap();
            let provider = ProviderId::try_from(canon).expect("canonical token must parse");
            assert_eq!(provider.canonical_key(), canon);
        }
    }

    #[test]
    fn fill_from_applies_precedence() {
        let user = ModelCapabilities {
            context_window: Some(128000),
            ..Default::default()
        };
        let models_dev = ModelCapabilities {
            context_window: Some(200000),
            supports_vision: Some(true),
            ..Default::default()
        };
        let resolved = user.fill_from(&models_dev);
        // user override wins for context_window
        assert_eq!(resolved.context_window, Some(128000));
        // models.dev backfills vision
        assert_eq!(resolved.supports_vision, Some(true));
    }
}
