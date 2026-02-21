use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Pricing info for a model
#[derive(Debug, Clone)]
pub struct PricingInfo {
    pub input_price_per_token: f64,  // cents per token
    pub output_price_per_token: f64, // cents per token
}

/// Thread-safe pricing registry
#[derive(Clone)]
pub struct PricingRegistry {
    prices: Arc<RwLock<HashMap<(String, String), PricingInfo>>>,
}

impl PricingRegistry {
    pub fn new() -> Self {
        Self {
            prices: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Load pricing from Portkey models JSON files
    pub async fn load_from_portkey_dir(
        &self,
        dir: &str,
    ) -> Result<usize, Box<dyn std::error::Error>> {
        let mut prices = HashMap::new();
        let entries = std::fs::read_dir(dir)?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }

            let content = std::fs::read_to_string(&path)?;
            match serde_json::from_str::<PortkeyProviderFile>(&content) {
                Ok(provider_file) => {
                    for (model_name, model_data) in &provider_file.models {
                        if let Some(ref pricing) = model_data.pricing_config {
                            if let Some(ref payg) = pricing.pay_as_you_go {
                                let input_price =
                                    payg.request_token.as_ref().map(|t| t.price).unwrap_or(0.0);
                                let output_price =
                                    payg.response_token.as_ref().map(|t| t.price).unwrap_or(0.0);

                                let provider_name = path
                                    .file_stem()
                                    .and_then(|s| s.to_str())
                                    .unwrap_or("unknown")
                                    .to_string();

                                prices.insert(
                                    (provider_name, model_name.clone()),
                                    PricingInfo {
                                        input_price_per_token: input_price,
                                        output_price_per_token: output_price,
                                    },
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!(file = ?path, error = %e, "failed to parse portkey pricing file");
                }
            }
        }

        let count = prices.len();
        *self.prices.write().await = prices;
        info!(models = count, "loaded pricing data");
        Ok(count)
    }

    /// Get pricing for a provider/model combination
    pub async fn get_pricing(&self, provider: &str, model: &str) -> Option<PricingInfo> {
        let prices = self.prices.read().await;
        prices
            .get(&(provider.to_string(), model.to_string()))
            .cloned()
    }

    /// Calculate cost in cents for a request (Portkey pricing only)
    pub async fn calculate_cost(
        &self,
        provider: &str,
        model: &str,
        input_tokens: i32,
        output_tokens: i32,
    ) -> f64 {
        match self.get_pricing(provider, model).await {
            Some(pricing) => {
                (input_tokens as f64 * pricing.input_price_per_token)
                    + (output_tokens as f64 * pricing.output_price_per_token)
            }
            None => {
                warn!(provider, model, "no pricing data found, cost = 0");
                0.0
            }
        }
    }

    /// Calculate cost using the full pricing chain:
    /// 1. Custom project pricing (per-million to per-token)
    /// 2. Global custom pricing
    /// 3. Portkey pricing
    pub async fn calculate_cost_with_custom(
        &self,
        pool: &crate::db::DbPool,
        project_id: uuid::Uuid,
        provider: &str,
        model: &str,
        input_tokens: i32,
        output_tokens: i32,
    ) -> f64 {
        // Try custom pricing (project -> global)
        if let Ok(client) = pool.get_client().await {
            if let Ok(Some(custom)) =
                crate::db::queries::get_custom_pricing(&client, project_id, provider, model).await
            {
                let input_cost =
                    input_tokens as f64 * (custom.input_price_per_million / 1_000_000.0);
                let output_cost =
                    output_tokens as f64 * (custom.output_price_per_million / 1_000_000.0);
                return input_cost + output_cost;
            }
        }

        // Fall back to Portkey pricing
        self.calculate_cost(provider, model, input_tokens, output_tokens)
            .await
    }
}

// Portkey JSON file structure
#[derive(Debug, Deserialize)]
struct PortkeyProviderFile {
    #[serde(flatten)]
    models: HashMap<String, PortkeyModelData>,
}

#[derive(Debug, Deserialize)]
struct PortkeyModelData {
    pricing_config: Option<PortkeyPricingConfig>,
}

#[derive(Debug, Deserialize)]
struct PortkeyPricingConfig {
    pay_as_you_go: Option<PayAsYouGo>,
}

#[derive(Debug, Deserialize)]
struct PayAsYouGo {
    request_token: Option<TokenPrice>,
    response_token: Option<TokenPrice>,
}

#[derive(Debug, Deserialize)]
struct TokenPrice {
    price: f64,
}
