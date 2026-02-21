use super::token_resolver::AuthContext;
use uuid::Uuid;

/// A pipe selected for a specific model request
#[derive(Debug, Clone)]
pub struct SelectedPipe {
    pub pipe_id: Uuid,
    pub provider: String,
    pub api_key_decrypted: String,
    pub model: String,
}

/// Infer the provider from a model name when no explicit "provider/" prefix is given.
fn infer_provider(model: &str) -> Option<&'static str> {
    let m = model.to_lowercase();
    if m.starts_with("gpt-")
        || m.starts_with("o1")
        || m.starts_with("o3")
        || m.starts_with("o4")
        || m.starts_with("chatgpt")
        || m.starts_with("dall-e")
        || m.starts_with("text-embedding")
        || m.starts_with("whisper")
        || m.starts_with("tts")
    {
        Some("openai")
    } else if m.starts_with("claude") {
        Some("anthropic")
    } else if m.starts_with("gemini") || m.starts_with("gemma") {
        Some("google")
    } else if m.starts_with("mistral")
        || m.starts_with("mixtral")
        || m.starts_with("ministral")
        || m.starts_with("codestral")
        || m.starts_with("pixtral")
    {
        Some("mistral")
    } else if m.starts_with("llama") || m.starts_with("meta-llama") {
        Some("meta")
    } else if m.starts_with("deepseek") {
        Some("deepseek")
    } else if m.starts_with("command") || m.starts_with("embed-") || m.starts_with("rerank-") {
        Some("cohere")
    } else {
        None
    }
}

/// Select the appropriate pipe for the requested model.
///
/// Logic:
/// 1. Determine the provider from the model name (e.g., "openai/gpt-4o" -> "openai")
///    or infer it from well-known model prefixes (e.g., "gpt-4o" -> "openai")
/// 2. Find a pipe whose provider matches
/// 3. Check if the pipe's model_filter allows this model (NULL = all models)
/// 4. Return the pipe's API key (currently stored as plaintext, future: decrypt)
pub fn select_pipe(
    auth_ctx: &AuthContext,
    model: &str,
) -> Result<SelectedPipe, PipeSelectionError> {
    // Extract provider from model name "provider/model_id" or infer from model prefix
    let (provider, model_id) = if model.contains('/') {
        let parts: Vec<&str> = model.splitn(2, '/').collect();
        (parts[0].to_lowercase(), parts[1].to_string())
    } else if let Some(inferred) = infer_provider(model) {
        (inferred.to_string(), model.to_string())
    } else {
        (model.to_lowercase(), model.to_string())
    };

    for pipe in &auth_ctx.pipes {
        let pipe_provider = pipe.provider.to_lowercase();
        if pipe_provider != provider {
            continue;
        }

        // Check model filter
        if let Some(ref filter) = pipe.model_filter {
            let allowed: Vec<&str> = filter.split(',').map(|s| s.trim()).collect();
            let is_match = allowed
                .iter()
                .any(|&f| f == "*" || f == model || f == model_id);
            if !is_match {
                continue;
            }
        }

        // Found a matching pipe
        // For now, api_key_encrypted is treated as plaintext
        // TODO: implement proper encryption/decryption
        return Ok(SelectedPipe {
            pipe_id: pipe.id,
            provider: pipe.provider.clone(),
            api_key_decrypted: pipe.api_key_encrypted.clone(),
            model: model.to_string(),
        });
    }

    Err(PipeSelectionError::NoPipeFound {
        provider,
        model: model.to_string(),
    })
}

#[derive(Debug, thiserror::Error)]
pub enum PipeSelectionError {
    #[error("no pipe found for provider '{provider}' model '{model}'")]
    NoPipeFound { provider: String, model: String },
}
