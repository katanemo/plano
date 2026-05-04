// Fetch latest provider models from canonical provider APIs and merge into
// provider_models.yaml.
//
// Behavior is non-destructive: only providers we successfully fetch this run
// are replaced. Providers whose API key is missing, or whose fetch fails, are
// left untouched in the existing file. This means partial runs (e.g. without
// AWS or Google creds) can't accidentally wipe out provider entries you don't
// have keys for locally.
//
// Usage:
//   Optional: OPENAI_API_KEY, ANTHROPIC_API_KEY, MISTRAL_API_KEY,
//             DEEPSEEK_API_KEY, GROK_API_KEY, DASHSCOPE_API_KEY,
//             MOONSHOT_API_KEY, ZHIPU_API_KEY, MIMO_API_KEY, GOOGLE_API_KEY
//   Optional: AWS CLI configured for Amazon Bedrock models
//   cargo run --bin fetch_models --features model-fetch

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

fn main() {
    // Default to writing in the same directory as this source file
    let default_path = std::path::Path::new(file!())
        .parent()
        .unwrap()
        .join("provider_models.yaml");

    let output_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| default_path.to_string_lossy().to_string());

    println!("Loading existing {}...", output_path);
    let existing = match load_existing_models(&output_path) {
        Ok(map) => {
            if map.is_empty() {
                println!("  (none — starting fresh)");
            } else {
                println!("  loaded {} existing providers", map.len());
            }
            map
        }
        Err(e) => {
            eprintln!("Error loading existing {}: {}", output_path, e);
            eprintln!("Refusing to overwrite a file we can't parse. Fix or delete it and re-run.");
            std::process::exit(1);
        }
    };

    println!("\nFetching latest models from provider APIs...");

    match fetch_all_models(existing) {
        Ok(models) => {
            let yaml = serde_yaml::to_string(&models).expect("Failed to serialize models");

            std::fs::write(&output_path, yaml).expect("Failed to write provider_models.yaml");

            println!(
                "✓ Wrote {} providers ({} models) to {}",
                models.metadata.total_providers, models.metadata.total_models, output_path
            );
        }
        Err(e) => {
            eprintln!("Error fetching models: {}", e);
            eprintln!("\nMake sure required tools are set up:");
            eprintln!("  AWS CLI configured for Bedrock (for Amazon models)");
            eprintln!("  export OPENAI_API_KEY=your-key-here      # Optional");
            eprintln!("  export DEEPSEEK_API_KEY=your-key-here    # Optional");
            eprintln!("  cargo run --bin fetch_models");
            std::process::exit(1);
        }
    }
}

fn load_existing_models(
    path: &str,
) -> Result<BTreeMap<String, Vec<String>>, Box<dyn std::error::Error>> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(Box::new(e)),
    };
    let parsed: ProviderModels = serde_yaml::from_str(&content)?;
    Ok(parsed.providers)
}

// OpenAI-compatible API response (used by most providers)
#[derive(Debug, Deserialize)]
struct OpenAICompatibleModel {
    id: String,
}

#[derive(Debug, Deserialize)]
struct OpenAICompatibleResponse {
    data: Vec<OpenAICompatibleModel>,
}

// Google Gemini API response
#[derive(Debug, Deserialize)]
struct GoogleModel {
    name: String,
    #[serde(rename = "supportedGenerationMethods")]
    supported_generation_methods: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct GoogleResponse {
    models: Vec<GoogleModel>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ProviderModels {
    #[serde(default = "default_version")]
    version: String,
    #[serde(default = "default_source")]
    source: String,
    #[serde(default)]
    providers: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    metadata: Metadata,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Metadata {
    #[serde(default)]
    total_providers: usize,
    #[serde(default)]
    total_models: usize,
    #[serde(default)]
    last_updated: String,
}

fn default_version() -> String {
    "1.0".to_string()
}

fn default_source() -> String {
    "canonical-apis".to_string()
}

fn is_text_model(model_id: &str) -> bool {
    let id_lower = model_id.to_lowercase();

    // Filter out known non-text models
    let non_text_patterns = [
        "embedding",   // Embedding models
        "whisper",     // Audio transcription
        "-tts",        // Text-to-speech (with dash to avoid matching in middle of words)
        "tts-",        // Text-to-speech prefix
        "dall-e",      // Image generation
        "sora",        // Video generation
        "moderation",  // Moderation models
        "babbage",     // Legacy completion models
        "davinci-002", // Legacy completion models
        "transcribe",  // Audio transcription models
        "realtime",    // Realtime audio models
        "audio",       // Audio models (gpt-audio, gpt-audio-mini)
        "-image-",     // Image generation models (grok-2-image-1212)
        "-ocr-",       // OCR models
        "ocr-",        // OCR models prefix
        "voxtral",     // Audio/voice models
    ];

    // Additional pattern: models that are purely for image generation usually have "image" in the name
    // but we need to be careful not to filter vision models that can process images
    // Models like "gpt-image-1" or "chatgpt-image-latest" are image generators
    // Models like "grok-2-vision" or "gemini-vision" are vision models (text+image->text)

    if non_text_patterns
        .iter()
        .any(|pattern| id_lower.contains(pattern))
    {
        return false;
    }

    // Filter models starting with "gpt-image" (image generators)
    if id_lower.contains("/gpt-image") || id_lower.contains("/chatgpt-image") {
        return false;
    }

    true
}

fn fetch_openai_compatible_models(
    api_url: &str,
    api_key: &str,
    provider_prefix: &str,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let response_body = ureq::get(api_url)
        .header("Authorization", &format!("Bearer {}", api_key))
        .call()?
        .body_mut()
        .read_to_string()?;

    let response: OpenAICompatibleResponse = serde_json::from_str(&response_body)?;

    Ok(response
        .data
        .into_iter()
        .filter(|m| is_text_model(&m.id))
        .map(|m| format!("{}/{}", provider_prefix, m.id))
        .collect())
}

fn fetch_anthropic_models(api_key: &str) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let response_body = ureq::get("https://api.anthropic.com/v1/models")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .call()?
        .body_mut()
        .read_to_string()?;

    let response: OpenAICompatibleResponse = serde_json::from_str(&response_body)?;

    let dated_models: Vec<String> = response
        .data
        .into_iter()
        .filter(|m| is_text_model(&m.id))
        .map(|m| m.id)
        .collect();

    let mut models: Vec<String> = Vec::new();

    // Add both dated versions and their aliases (without the -YYYYMMDD suffix)
    for model_id in dated_models {
        // Add the full dated model ID
        models.push(format!("anthropic/{}", model_id));

        // Generate alias by removing trailing -YYYYMMDD pattern
        // Pattern: ends with -YYYYMMDD where YYYY is year, MM is month, DD is day
        if let Some(date_pos) = model_id.rfind('-') {
            let potential_date = &model_id[date_pos + 1..];
            // Check if it's an 8-digit date (YYYYMMDD)
            if potential_date.len() == 8 && potential_date.chars().all(|c| c.is_ascii_digit()) {
                let alias = &model_id[..date_pos];
                let alias_full = format!("anthropic/{}", alias);
                // Only add if not already present
                if !models.contains(&alias_full) {
                    models.push(alias_full);
                }
            }
        }
    }

    Ok(models)
}

fn fetch_google_models(api_key: &str) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let api_url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models?key={}",
        api_key
    );

    let response_body = ureq::get(&api_url).call()?.body_mut().read_to_string()?;

    let response: GoogleResponse = serde_json::from_str(&response_body)?;

    // Only include models that support generateContent
    Ok(response
        .models
        .into_iter()
        .filter(|m| {
            m.supported_generation_methods
                .as_ref()
                .is_some_and(|methods| methods.contains(&"generateContent".to_string()))
        })
        .map(|m| {
            // Convert "models/gemini-pro" to "google/gemini-pro"
            let model_id = m.name.strip_prefix("models/").unwrap_or(&m.name);
            format!("google/{}", model_id)
        })
        .collect())
}

fn fetch_bedrock_amazon_models() -> Result<Vec<String>, Box<dyn std::error::Error>> {
    // Use AWS CLI to fetch Amazon models from Bedrock
    let output = std::process::Command::new("aws")
        .args([
            "bedrock",
            "list-foundation-models",
            "--by-provider",
            "amazon",
            "--by-output-modality",
            "TEXT",
            "--no-cli-pager",
            "--output",
            "json",
        ])
        .output()?;

    if !output.status.success() {
        return Err(format!(
            "AWS CLI command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    let response_body = String::from_utf8(output.stdout)?;

    #[derive(Debug, Deserialize)]
    struct BedrockModelSummary {
        #[serde(rename = "modelId")]
        model_id: String,
    }

    #[derive(Debug, Deserialize)]
    struct BedrockResponse {
        #[serde(rename = "modelSummaries")]
        model_summaries: Vec<BedrockModelSummary>,
    }

    let bedrock_response: BedrockResponse = serde_json::from_str(&response_body)?;

    // Filter out embedding, image generation, and rerank models
    let amazon_models: Vec<String> = bedrock_response
        .model_summaries
        .into_iter()
        .filter(|model| {
            let id_lower = model.model_id.to_lowercase();
            !id_lower.contains("embed")
                && !id_lower.contains("image")
                && !id_lower.contains("rerank")
        })
        .map(|m| format!("amazon/{}", m.model_id))
        .collect();

    Ok(amazon_models)
}

fn fetch_all_models(
    existing: BTreeMap<String, Vec<String>>,
) -> Result<ProviderModels, Box<dyn std::error::Error>> {
    let mut providers = existing;
    let mut updated: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    // Configuration: provider name, env var, API URL, prefix for model IDs
    let provider_configs = vec![
        (
            "openai",
            "OPENAI_API_KEY",
            "https://api.openai.com/v1/models",
            "openai",
        ),
        (
            "mistralai",
            "MISTRAL_API_KEY",
            "https://api.mistral.ai/v1/models",
            "mistralai",
        ),
        (
            "deepseek",
            "DEEPSEEK_API_KEY",
            "https://api.deepseek.com/v1/models",
            "deepseek",
        ),
        ("x-ai", "GROK_API_KEY", "https://api.x.ai/v1/models", "x-ai"),
        (
            "moonshotai",
            "MOONSHOT_API_KEY",
            "https://api.moonshot.ai/v1/models",
            "moonshotai",
        ),
        (
            "qwen",
            "DASHSCOPE_API_KEY",
            "https://dashscope-intl.aliyuncs.com/compatible-mode/v1/models",
            "qwen",
        ),
        (
            "z-ai",
            "ZHIPU_API_KEY",
            "https://open.bigmodel.cn/api/paas/v4/models",
            "z-ai",
        ),
        (
            "xiaomi",
            "MIMO_API_KEY",
            "https://api.xiaomimimo.com/v1/models",
            "xiaomi",
        ),
    ];

    // Helper that records the outcome of a fetch attempt and only mutates
    // `providers` on success, so missing/failed providers keep their existing
    // entries (or stay absent if there were none).
    let mut record =
        |name: &str,
         env_var: Option<&str>,
         result: Option<Result<Vec<String>, Box<dyn std::error::Error>>>,
         providers: &mut BTreeMap<String, Vec<String>>| match result {
            Some(Ok(models)) => {
                println!("  ✓ {}: {} models", name, models.len());
                providers.insert(name.to_string(), models);
                updated.push(name.to_string());
            }
            Some(Err(e)) => {
                let kept = providers
                    .get(name)
                    .map(|v| format!(" (keeping existing {} models)", v.len()))
                    .unwrap_or_default();
                let err_msg = format!("  ✗ {}: {}{}", name, e, kept);
                eprintln!("{}", err_msg);
                errors.push(err_msg);
                failed.push(name.to_string());
            }
            None => {
                let kept = providers
                    .get(name)
                    .map(|v| format!(" (keeping existing {} models)", v.len()))
                    .unwrap_or_else(|| " (no existing entry)".to_string());
                let label = env_var
                    .map(|v| format!("{} not set", v))
                    .unwrap_or_else(|| "no credentials".to_string());
                println!("  ⊘ {}: {}{}", name, label, kept);
                skipped.push(name.to_string());
            }
        };

    // Fetch from OpenAI-compatible providers
    for (provider_name, env_var, api_url, prefix) in provider_configs {
        let result = std::env::var(env_var)
            .ok()
            .map(|api_key| fetch_openai_compatible_models(api_url, &api_key, prefix));
        record(provider_name, Some(env_var), result, &mut providers);
    }

    // Fetch Anthropic models (different authentication)
    let anthropic_result = std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .map(|key| fetch_anthropic_models(&key));
    record(
        "anthropic",
        Some("ANTHROPIC_API_KEY"),
        anthropic_result,
        &mut providers,
    );

    // Fetch Google models (different API format)
    let google_result = std::env::var("GOOGLE_API_KEY")
        .ok()
        .map(|key| fetch_google_models(&key));
    record(
        "google",
        Some("GOOGLE_API_KEY"),
        google_result,
        &mut providers,
    );

    // Fetch Amazon models from AWS Bedrock. Only attempt if the AWS CLI is on
    // PATH and any AWS credential is configured — otherwise treat as skipped
    // so we don't drop the existing amazon entry on machines / CI runs without
    // Bedrock access.
    let amazon_result = if aws_credentials_available() {
        Some(fetch_bedrock_amazon_models())
    } else {
        None
    };
    record(
        "amazon",
        Some("AWS credentials"),
        amazon_result,
        &mut providers,
    );

    if providers.is_empty() {
        return Err(
            "No existing data and no models fetched. Set at least one API key and re-run.".into(),
        );
    }

    let total_providers = providers.len();
    let total_models: usize = providers.values().map(|v| v.len()).sum();

    println!("\nSummary:");
    println!(
        "  updated: {} ({})",
        updated.len(),
        if updated.is_empty() {
            "none".to_string()
        } else {
            updated.join(", ")
        }
    );
    println!(
        "  skipped (kept existing): {} ({})",
        skipped.len(),
        if skipped.is_empty() {
            "none".to_string()
        } else {
            skipped.join(", ")
        }
    );
    if !failed.is_empty() {
        println!(
            "  failed (kept existing): {} ({})",
            failed.len(),
            failed.join(", ")
        );
    }
    println!(
        "✅ Final state: {} providers, {} models",
        total_providers, total_models
    );

    Ok(ProviderModels {
        version: default_version(),
        source: default_source(),
        providers,
        metadata: Metadata {
            total_providers,
            total_models,
            last_updated: chrono::Utc::now().to_rfc3339(),
        },
    })
}

fn aws_credentials_available() -> bool {
    std::env::var("AWS_ACCESS_KEY_ID").is_ok()
        || std::env::var("AWS_PROFILE").is_ok()
        || std::env::var("AWS_SESSION_TOKEN").is_ok()
        || std::env::var("AWS_WEB_IDENTITY_TOKEN_FILE").is_ok()
}
