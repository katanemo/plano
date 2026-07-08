//! Prompt-caching request handling for the LLM path.
//!
//! This module owns the "make the upstream provider's prompt cache work" concern:
//! resolving the correct cache-marking strategy for the `(gateway × model family ×
//! upstream API)` combination and injecting the markers into the outbound request.
//! It never influences routing — see [`super::session_stickiness`] for the pinning /
//! switch-cost concern.

use common::configuration::EffectivePromptCaching;
use hermesllm::clients::SupportedUpstreamAPIs;
use hermesllm::{cache_marker_strategy, CacheMarkerStrategy, ProviderId, ProviderRequestType};
use tracing::debug;

/// Auto-inject prompt-cache markers into `client_request`.
///
/// The strategy is resolved from `(gateway × model family × upstream API)` so that
/// Anthropic-family models cache whether they arrive over the native Messages API or
/// an OpenAI-compatible gateway (DigitalOcean, OpenRouter), while OpenAI-family models
/// (which cache automatically) and unimplemented backends are left untouched.
///
/// A no-op when caching is disabled, `inject_cache_control` is off, the request opted
/// out (`X-Plano-Cache: off`), or the provider caches automatically. Injection is
/// idempotent (client-supplied markers are respected) and threshold-guarded against
/// the provider's minimum cacheable prefix.
pub fn inject_cache_markers(
    client_request: &mut ProviderRequestType,
    provider_id: ProviderId,
    model_name_only: &str,
    upstream_api: &SupportedUpstreamAPIs,
    prompt_caching: &EffectivePromptCaching,
    cache_off_for_request: bool,
    alias_resolved_model: &str,
) {
    if !(prompt_caching.enabled && prompt_caching.inject_cache_control && !cache_off_for_request) {
        return;
    }

    match cache_marker_strategy(provider_id, model_name_only, upstream_api) {
        CacheMarkerStrategy::AnthropicMessagesBreakpoints {
            min_prefix_tokens, ..
        } => {
            if let ProviderRequestType::MessagesRequest(req) = client_request {
                let threshold = prompt_caching.min_prefix_tokens.max(min_prefix_tokens);
                if req.inject_cache_breakpoints(threshold) {
                    debug!(
                        model = %alias_resolved_model,
                        min_prefix_tokens = threshold,
                        "injected anthropic ephemeral cache breakpoints"
                    );
                }
            }
        }
        CacheMarkerStrategy::OpenAiContentPartCacheControl {
            min_prefix_tokens,
            ttl,
        } => {
            if let ProviderRequestType::ChatCompletionsRequest(req) = client_request {
                let threshold = prompt_caching.min_prefix_tokens.max(min_prefix_tokens);
                if req.inject_cache_control(ttl, threshold) {
                    debug!(
                        model = %alias_resolved_model,
                        min_prefix_tokens = threshold,
                        "injected openai content-part cache_control"
                    );
                }
            }
        }
        CacheMarkerStrategy::Automatic | CacheMarkerStrategy::None => {}
    }
}
