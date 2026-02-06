// Fetch latest provider models from canonical provider APIs and update provider_models.yaml
// Usage:
//   Optional: OPENAI_API_KEY, ANTHROPIC_API_KEY, DEEPSEEK_API_KEY, GROK_API_KEY,
//             DASHSCOPE_API_KEY, MOONSHOT_API_KEY, ZHIPU_API_KEY, GOOGLE_API_KEY
//   Required: AWS CLI configured for Amazon Bedrock models
//   cargo run --bin fetch_models

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn main() {
    // Default to writing in the same directory as this source file
    let default_path = std::path::Path::new(file!())
        .parent()
        .unwrap()
        .join("provider_models.yaml");

    let output_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| default_path.to_string_lossy().to_string());

    println!("Fetching latest models from provider APIs...");

    match fetch_all_models() {
        Ok(models) => {
            let yaml = serde_yaml::to_string(&models).expect("Failed to serialize models");

            std::fs::write(&output_path, yaml).expect("Failed to write provider_models.yaml");

            println!(
                "✓ Successfully updated {} providers ({} models) to {}",
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

#[derive(Debug, Serialize)]
struct ProviderModels {
    version: String,
    source: String,
    providers: HashMap<String, Vec<String>>,
    metadata: Metadata,
}

#[derive(Debug, Serialize)]
struct Metadata {
    total_providers: usize,
    total_models: usize,
    last_updated: String,
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

fn fetch_all_models() -> Result<ProviderModels, Box<dyn std::error::Error>> {
    let mut providers: HashMap<String, Vec<String>> = HashMap::new();
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
    ];

    // Fetch from OpenAI-compatible providers
    for (provider_name, env_var, api_url, prefix) in provider_configs {
        if let Ok(api_key) = std::env::var(env_var) {
            match fetch_openai_compatible_models(api_url, &api_key, prefix) {
                Ok(models) => {
                    println!("  ✓ {}: {} models", provider_name, models.len());
                    providers.insert(provider_name.to_string(), models);
                }
                Err(e) => {
                    let err_msg = format!("  ✗ {}: {}", provider_name, e);
                    eprintln!("{}", err_msg);
                    errors.push(err_msg);
                }
            }
        } else {
            println!("  ⊘ {}: {} not set (skipped)", provider_name, env_var);
        }
    }

    // Fetch Anthropic models (different authentication)
    if let Ok(api_key) = std::env::var("ANTHROPIC_API_KEY") {
        match fetch_anthropic_models(&api_key) {
            Ok(models) => {
                println!("  ✓ anthropic: {} models", models.len());
                providers.insert("anthropic".to_string(), models);
            }
            Err(e) => {
                let err_msg = format!("  ✗ anthropic: {}", e);
                eprintln!("{}", err_msg);
                errors.push(err_msg);
            }
        }
    } else {
        println!("  ⊘ anthropic: ANTHROPIC_API_KEY not set (skipped)");
    }

    // Fetch Google models (different API format)
    if let Ok(api_key) = std::env::var("GOOGLE_API_KEY") {
        match fetch_google_models(&api_key) {
            Ok(models) => {
                println!("  ✓ google: {} models", models.len());
                providers.insert("google".to_string(), models);
            }
            Err(e) => {
                let err_msg = format!("  ✗ google: {}", e);
                eprintln!("{}", err_msg);
                errors.push(err_msg);
            }
        }
    } else {
        println!("  ⊘ google: GOOGLE_API_KEY not set (skipped)");
    }

    // Fetch Amazon models from AWS Bedrock
    match fetch_bedrock_amazon_models() {
        Ok(models) => {
            println!("  ✓ amazon: {} models (via AWS Bedrock)", models.len());
            providers.insert("amazon".to_string(), models);
        }
        Err(e) => {
            let err_msg = format!("  ✗ amazon: {} (AWS Bedrock required)", e);
            eprintln!("{}", err_msg);
            errors.push(err_msg);
        }
    }

    if providers.is_empty() {
        return Err("No models fetched from any provider. Check API keys.".into());
    }

    let total_providers = providers.len();
    let total_models: usize = providers.values().map(|v| v.len()).sum();

    println!(
        "\n✅ Successfully fetched models from {} providers",
        total_providers
    );
    if !errors.is_empty() {
        println!("⚠️  {} providers failed", errors.len());
    }

    Ok(ProviderModels {
        version: "1.0".to_string(),
        source: "canonical-apis".to_string(),
        providers,
        metadata: Metadata {
            total_providers,
            total_models,
            last_updated: chrono::Utc::now().to_rfc3339(),
        },
    })
}
