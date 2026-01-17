// Fetch latest provider models from OpenRouter and update provider_models.json
// Usage: OPENROUTER_API_KEY=xxx cargo run --bin fetch_models

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn main() {
    // Default to writing in the same directory as this source file
    let default_path = std::path::Path::new(file!())
        .parent()
        .unwrap()
        .join("provider_models.json");

    let output_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| default_path.to_string_lossy().to_string());

    println!("Fetching latest models from OpenRouter...");

    match fetch_openrouter_models() {
        Ok(models) => {
            let json = serde_json::to_string_pretty(&models).expect("Failed to serialize models");

            std::fs::write(&output_path, json).expect("Failed to write provider_models.json");

            println!(
                "✓ Successfully updated {} providers ({} models) to {}",
                models.metadata.total_providers, models.metadata.total_models, output_path
            );
        }
        Err(e) => {
            eprintln!("Error fetching models: {}", e);
            eprintln!("\nMake sure OPENROUTER_API_KEY is set:");
            eprintln!("  export OPENROUTER_API_KEY=your-key-here");
            eprintln!("  cargo run --bin fetch_models");
            std::process::exit(1);
        }
    }
}

#[derive(Debug, Deserialize)]
struct OpenRouterModel {
    id: String,
    architecture: Option<Architecture>,
}

#[derive(Debug, Deserialize)]
struct Architecture {
    modality: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterResponse {
    data: Vec<OpenRouterModel>,
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

fn fetch_openrouter_models() -> Result<ProviderModels, Box<dyn std::error::Error>> {
    let api_key = std::env::var("OPENROUTER_API_KEY")
        .map_err(|_| "OPENROUTER_API_KEY environment variable not set")?;

    let response_body = ureq::get("https://openrouter.ai/api/v1/models")
        .header("Authorization", &format!("Bearer {}", api_key))
        .call()?
        .body_mut()
        .read_to_string()?;

    let openrouter_response: OpenRouterResponse = serde_json::from_str(&response_body)?;

    // Supported providers to include
    let supported_providers = [
        "openai",
        "anthropic",
        "mistralai",
        "deepseek",
        "google",
        "x-ai",
        "moonshotai",
        "qwen",
        "amazon",
        "z-ai",
    ];

    let mut providers: HashMap<String, Vec<String>> = HashMap::new();
    let mut total_models = 0;
    let mut filtered_modality: Vec<(String, String)> = Vec::new();
    let mut filtered_provider: Vec<(String, Option<String>)> = Vec::new();

    for model in openrouter_response.data {
        let modality = model
            .architecture
            .as_ref()
            .and_then(|arch| arch.modality.clone());

        // Only include text->text and text+image->text models
        if let Some(ref mod_str) = modality {
            if mod_str != "text->text" && mod_str != "text" && mod_str != "text+image->text" {
                filtered_modality.push((model.id.clone(), mod_str.clone()));
                continue;
            }
        }

        // Extract provider from model ID (e.g., "openai/gpt-4" -> "openai")
        if let Some(provider_name) = model.id.split('/').next() {
            if supported_providers.contains(&provider_name) {
                providers
                    .entry(provider_name.to_string())
                    .or_default()
                    .push(model.id.clone());
                total_models += 1;
            } else {
                filtered_provider.push((model.id.clone(), modality));
            }
        }
    }

    println!("✅ Loaded models from {} providers:", providers.len());
    let mut sorted_providers: Vec<_> = providers.iter().collect();
    sorted_providers.sort_by_key(|(name, _)| *name);
    for (provider, models) in sorted_providers {
        println!("  • {}: {} models", provider, models.len());
    }

    // Group filtered providers to get counts
    let mut filtered_by_provider: HashMap<String, usize> = HashMap::new();
    for (model_id, _modality) in &filtered_provider {
        if let Some(provider_name) = model_id.split('/').next() {
            *filtered_by_provider
                .entry(provider_name.to_string())
                .or_insert(0) += 1;
        }
    }

    println!(
        "\n⏭️  Skipped {} providers ({} models total)",
        filtered_by_provider.len(),
        filtered_provider.len()
    );
    println!();

    let total_providers = providers.len();

    Ok(ProviderModels {
        version: "1.0".to_string(),
        source: "openrouter".to_string(),
        providers,
        metadata: Metadata {
            total_providers,
            total_models,
            last_updated: chrono::Utc::now().to_rfc3339(),
        },
    })
}
