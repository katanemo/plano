use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use common::configuration::{
    CostProvider, LatencyProvider, MetricsSource, SelectionPolicy, SelectionPreference,
};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

const DO_PRICING_URL: &str = "https://api.digitalocean.com/v2/gen-ai/models/catalog";

/// Anthropic's standard cache-write surcharge (5-minute cache). Catalog sources
/// generally don't publish a write rate, so this multiplier is applied to the input
/// rate; override via `model_metrics_sources` cost config.
const DEFAULT_CACHE_CREATION_MULTIPLIER: f64 = 1.25;

/// Assumed per-turn fresh input/output tokens when projecting cache-adjusted cost.
/// The dominant term is the repeated prefix, so precision here barely matters.
const ASSUMED_NEW_INPUT_TOKENS: f64 = 500.0;
const ASSUMED_OUTPUT_TOKENS: f64 = 500.0;

/// Per-model pricing, in $ per million tokens.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelPricing {
    pub input_per_million: f64,
    pub output_per_million: f64,
    /// Cached-input (cache read) rate. `None` when the catalog doesn't publish one —
    /// the model is then priced with no cache discount.
    pub cached_input_per_million: Option<f64>,
}

/// Request-scoped context for ranking decisions.
#[derive(Debug, Clone, Default)]
pub struct RankContext {
    /// Model whose provider prompt cache may still be warm for this session
    /// (the current or recently-expired pin). Cache-aware ranking prices this model
    /// at its cached-input rate for the repeated prefix; all others pay full input
    /// price plus the cache-creation surcharge — the switch penalty.
    pub previous_model: Option<String>,
    /// Estimated token length of the repeated prompt prefix.
    pub prefix_tokens: Option<u64>,
}

pub struct ModelMetricsService {
    cost: Arc<RwLock<HashMap<String, f64>>>,
    pricing: Arc<RwLock<HashMap<String, ModelPricing>>>,
    latency: Arc<RwLock<HashMap<String, f64>>>,
    cache_creation_multiplier: f64,
}

impl ModelMetricsService {
    pub async fn new(sources: &[MetricsSource], client: reqwest::Client) -> Self {
        let cost_data = Arc::new(RwLock::new(HashMap::new()));
        let pricing_data = Arc::new(RwLock::new(HashMap::new()));
        let latency_data = Arc::new(RwLock::new(HashMap::new()));
        let mut cache_creation_multiplier = DEFAULT_CACHE_CREATION_MULTIPLIER;

        for source in sources {
            match source {
                MetricsSource::Cost(cfg) => match cfg.provider {
                    CostProvider::Digitalocean => {
                        if let Some(mult) = cfg.cache_creation_multiplier {
                            cache_creation_multiplier = mult;
                        }
                        let aliases = cfg.model_aliases.clone().unwrap_or_default();
                        let pricing = fetch_do_pricing(&client, &aliases).await;
                        info!(models = pricing.len(), "fetched digitalocean pricing");
                        *cost_data.write().await = summed_cost(&pricing);
                        *pricing_data.write().await = pricing;

                        if let Some(interval_secs) = cfg.refresh_interval {
                            let cost_clone = Arc::clone(&cost_data);
                            let pricing_clone = Arc::clone(&pricing_data);
                            let client_clone = client.clone();
                            let interval = Duration::from_secs(interval_secs);
                            tokio::spawn(async move {
                                loop {
                                    tokio::time::sleep(interval).await;
                                    let pricing = fetch_do_pricing(&client_clone, &aliases).await;
                                    info!(models = pricing.len(), "refreshed digitalocean pricing");
                                    *cost_clone.write().await = summed_cost(&pricing);
                                    *pricing_clone.write().await = pricing;
                                }
                            });
                        }
                    }
                },
                MetricsSource::Latency(cfg) => match cfg.provider {
                    LatencyProvider::Prometheus => {
                        let data = fetch_prometheus_metrics(&cfg.url, &cfg.query, &client).await;
                        info!(models = data.len(), url = %cfg.url, "fetched latency metrics");
                        *latency_data.write().await = data;

                        if let Some(interval_secs) = cfg.refresh_interval {
                            let latency_clone = Arc::clone(&latency_data);
                            let client_clone = client.clone();
                            let url = cfg.url.clone();
                            let query = cfg.query.clone();
                            let interval = Duration::from_secs(interval_secs);
                            tokio::spawn(async move {
                                loop {
                                    tokio::time::sleep(interval).await;
                                    let data =
                                        fetch_prometheus_metrics(&url, &query, &client_clone).await;
                                    info!(models = data.len(), url = %url, "refreshed latency metrics");
                                    *latency_clone.write().await = data;
                                }
                            });
                        }
                    }
                },
            }
        }

        ModelMetricsService {
            cost: cost_data,
            pricing: pricing_data,
            latency: latency_data,
            cache_creation_multiplier,
        }
    }

    /// Rank `models` by `policy`, returning them in preference order.
    /// Models with no metric data are appended at the end in their original order.
    pub async fn rank_models(
        &self,
        models: &[String],
        policy: &SelectionPolicy,
        ctx: &RankContext,
    ) -> Vec<String> {
        let cost_data = self.cost.read().await;
        let latency_data = self.latency.read().await;
        debug!(
            input_models = ?models,
            cost_data = ?cost_data.iter().collect::<Vec<_>>(),
            latency_data = ?latency_data.iter().collect::<Vec<_>>(),
            prefer = ?policy.prefer,
            previous_model = ?ctx.previous_model,
            prefix_tokens = ?ctx.prefix_tokens,
            "rank_models called"
        );

        match policy.prefer {
            SelectionPreference::Cheapest => {
                for m in models {
                    if !cost_data.contains_key(m.as_str()) {
                        warn!(model = %m, "no cost data for model — ranking last (prefer: cheapest)");
                    }
                }
                rank_by_ascending_metric(models, &cost_data)
            }
            SelectionPreference::Fastest => {
                for m in models {
                    if !latency_data.contains_key(m.as_str()) {
                        warn!(model = %m, "no latency data for model — ranking last (prefer: fastest)");
                    }
                }
                rank_by_ascending_metric(models, &latency_data)
            }
            SelectionPreference::CacheAware => {
                let pricing = self.pricing.read().await;
                let scores = self.cache_adjusted_scores(models, &pricing, ctx);
                for m in models {
                    if !scores.contains_key(m.as_str()) {
                        warn!(model = %m, "no pricing data for model — ranking last (prefer: cache_aware)");
                    }
                }
                rank_by_ascending_metric(models, &scores)
            }
            SelectionPreference::None => models.to_vec(),
        }
    }

    /// Projected next-turn cost per model, accounting for the warm provider cache.
    ///
    /// Staying on the previously-pinned model prices the repeated prefix at the
    /// cached-input rate; switching to any other model re-bills the full prefix at
    /// the input rate plus the cache-creation surcharge. This is the "price the
    /// miss, not the sticker rate" core of cache-aware routing.
    fn cache_adjusted_scores(
        &self,
        models: &[String],
        pricing: &HashMap<String, ModelPricing>,
        ctx: &RankContext,
    ) -> HashMap<String, f64> {
        let prefix_tokens = ctx.prefix_tokens.unwrap_or(0) as f64;
        models
            .iter()
            .filter_map(|m| {
                let p = pricing.get(m.as_str())?;
                let is_warm = ctx.previous_model.as_deref() == Some(m.as_str());
                let prefix_rate = if is_warm {
                    // Warm cache: prefix billed at cached-input rate when known.
                    p.cached_input_per_million.unwrap_or(p.input_per_million)
                } else {
                    // Cold model: full input price plus the one-time write surcharge.
                    p.input_per_million * self.cache_creation_multiplier
                };
                let score = prefix_tokens * prefix_rate
                    + ASSUMED_NEW_INPUT_TOKENS * p.input_per_million
                    + ASSUMED_OUTPUT_TOKENS * p.output_per_million;
                Some((m.clone(), score))
            })
            .collect()
    }

    /// Returns a snapshot of the current cost data. Used at startup to warn about unmatched models.
    pub async fn cost_snapshot(&self) -> HashMap<String, f64> {
        self.cost.read().await.clone()
    }

    /// Returns a snapshot of the current latency data. Used at startup to warn about unmatched models.
    pub async fn latency_snapshot(&self) -> HashMap<String, f64> {
        self.latency.read().await.clone()
    }

    /// Returns a snapshot of the full per-model pricing (input/output/cached-input).
    pub async fn pricing_snapshot(&self) -> HashMap<String, ModelPricing> {
        self.pricing.read().await.clone()
    }
}

/// Derive the legacy `input + output` cost map (used by `prefer: cheapest`).
fn summed_cost(pricing: &HashMap<String, ModelPricing>) -> HashMap<String, f64> {
    pricing
        .iter()
        .map(|(k, p)| (k.clone(), p.input_per_million + p.output_per_million))
        .collect()
}

fn rank_by_ascending_metric(models: &[String], data: &HashMap<String, f64>) -> Vec<String> {
    let mut with_data: Vec<(&String, f64)> = models
        .iter()
        .filter_map(|m| {
            let v = *data.get(m.as_str())?;
            if v.is_nan() {
                None
            } else {
                Some((m, v))
            }
        })
        .collect();
    with_data.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    let without_data: Vec<&String> = models
        .iter()
        .filter(|m| data.get(m.as_str()).is_none_or(|v| v.is_nan()))
        .collect();

    with_data
        .iter()
        .map(|(m, _)| (*m).clone())
        .chain(without_data.iter().map(|m| (*m).clone()))
        .collect()
}

#[derive(serde::Deserialize)]
struct DoModelList {
    data: Vec<DoModel>,
}

#[derive(serde::Deserialize)]
struct DoModel {
    model_id: String,
    pricing: Option<DoPricing>,
}

#[derive(serde::Deserialize)]
struct DoPricing {
    input_price_per_million: Option<f64>,
    output_price_per_million: Option<f64>,
    /// Cached-input (cache read) rate — e.g. $0.30/M vs $3.00/M input for Sonnet.
    cache_read_input_price_per_million: Option<f64>,
}

async fn fetch_do_pricing(
    client: &reqwest::Client,
    aliases: &HashMap<String, String>,
) -> HashMap<String, ModelPricing> {
    match client.get(DO_PRICING_URL).send().await {
        Ok(resp) => match resp.json::<DoModelList>().await {
            Ok(list) => list
                .data
                .into_iter()
                .filter_map(|m| {
                    let pricing = m.pricing?;
                    let raw_key = m.model_id.clone();
                    let key = aliases.get(&raw_key).cloned().unwrap_or(raw_key);
                    Some((
                        key,
                        ModelPricing {
                            input_per_million: pricing.input_price_per_million.unwrap_or(0.0),
                            output_per_million: pricing.output_price_per_million.unwrap_or(0.0),
                            cached_input_per_million: pricing.cache_read_input_price_per_million,
                        },
                    ))
                })
                .collect(),
            Err(err) => {
                warn!(error = %err, url = DO_PRICING_URL, "failed to parse digitalocean pricing response");
                HashMap::new()
            }
        },
        Err(err) => {
            warn!(error = %err, url = DO_PRICING_URL, "failed to fetch digitalocean pricing");
            HashMap::new()
        }
    }
}

#[derive(serde::Deserialize)]
struct PrometheusResponse {
    data: PrometheusData,
}

#[derive(serde::Deserialize)]
struct PrometheusData {
    result: Vec<PrometheusResult>,
}

#[derive(serde::Deserialize)]
struct PrometheusResult {
    metric: HashMap<String, String>,
    value: (f64, String), // (timestamp, value_str)
}

async fn fetch_prometheus_metrics(
    url: &str,
    query: &str,
    client: &reqwest::Client,
) -> HashMap<String, f64> {
    let query_url = format!("{}/api/v1/query", url.trim_end_matches('/'));
    match client
        .get(&query_url)
        .query(&[("query", query)])
        .send()
        .await
    {
        Ok(resp) => match resp.json::<PrometheusResponse>().await {
            Ok(prom) => prom
                .data
                .result
                .into_iter()
                .filter_map(|r| {
                    let model_name = r.metric.get("model_name")?.clone();
                    let value: f64 = r.value.1.parse().ok()?;
                    Some((model_name, value))
                })
                .collect(),
            Err(err) => {
                warn!(error = %err, url = %query_url, "failed to parse prometheus response");
                HashMap::new()
            }
        },
        Err(err) => {
            warn!(error = %err, url = %query_url, "failed to fetch prometheus metrics");
            HashMap::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::configuration::SelectionPreference;

    fn make_policy(prefer: SelectionPreference) -> SelectionPolicy {
        SelectionPolicy { prefer }
    }

    fn make_service(
        cost: HashMap<String, f64>,
        pricing: HashMap<String, ModelPricing>,
        latency: HashMap<String, f64>,
    ) -> ModelMetricsService {
        ModelMetricsService {
            cost: Arc::new(RwLock::new(cost)),
            pricing: Arc::new(RwLock::new(pricing)),
            latency: Arc::new(RwLock::new(latency)),
            cache_creation_multiplier: DEFAULT_CACHE_CREATION_MULTIPLIER,
        }
    }

    #[test]
    fn test_rank_by_ascending_metric_picks_lowest_first() {
        let models = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let mut data = HashMap::new();
        data.insert("a".to_string(), 0.01);
        data.insert("b".to_string(), 0.005);
        data.insert("c".to_string(), 0.02);
        assert_eq!(
            rank_by_ascending_metric(&models, &data),
            vec!["b", "a", "c"]
        );
    }

    #[test]
    fn test_rank_by_ascending_metric_no_data_preserves_order() {
        let models = vec!["x".to_string(), "y".to_string()];
        let data = HashMap::new();
        assert_eq!(rank_by_ascending_metric(&models, &data), vec!["x", "y"]);
    }

    #[test]
    fn test_rank_by_ascending_metric_partial_data() {
        let models = vec!["a".to_string(), "b".to_string()];
        let mut data = HashMap::new();
        data.insert("b".to_string(), 100.0);
        assert_eq!(rank_by_ascending_metric(&models, &data), vec!["b", "a"]);
    }

    #[tokio::test]
    async fn test_rank_models_cheapest() {
        let service = make_service(
            {
                let mut m = HashMap::new();
                m.insert("gpt-4o".to_string(), 0.005);
                m.insert("gpt-4o-mini".to_string(), 0.0001);
                m
            },
            HashMap::new(),
            HashMap::new(),
        );
        let models = vec!["gpt-4o".to_string(), "gpt-4o-mini".to_string()];
        let result = service
            .rank_models(
                &models,
                &make_policy(SelectionPreference::Cheapest),
                &RankContext::default(),
            )
            .await;
        assert_eq!(result, vec!["gpt-4o-mini", "gpt-4o"]);
    }

    #[tokio::test]
    async fn test_rank_models_fastest() {
        let service = make_service(HashMap::new(), HashMap::new(), {
            let mut m = HashMap::new();
            m.insert("gpt-4o".to_string(), 200.0);
            m.insert("claude-sonnet".to_string(), 120.0);
            m
        });
        let models = vec!["gpt-4o".to_string(), "claude-sonnet".to_string()];
        let result = service
            .rank_models(
                &models,
                &make_policy(SelectionPreference::Fastest),
                &RankContext::default(),
            )
            .await;
        assert_eq!(result, vec!["claude-sonnet", "gpt-4o"]);
    }

    #[tokio::test]
    async fn test_rank_models_fallback_no_metrics() {
        let service = make_service(HashMap::new(), HashMap::new(), HashMap::new());
        let models = vec!["model-a".to_string(), "model-b".to_string()];
        let result = service
            .rank_models(
                &models,
                &make_policy(SelectionPreference::Cheapest),
                &RankContext::default(),
            )
            .await;
        assert_eq!(result, vec!["model-a", "model-b"]);
    }

    #[tokio::test]
    async fn test_rank_models_partial_data_appended_last() {
        let service = make_service(
            {
                let mut m = HashMap::new();
                m.insert("gpt-4o".to_string(), 0.005);
                m
            },
            HashMap::new(),
            HashMap::new(),
        );
        let models = vec!["gpt-4o-mini".to_string(), "gpt-4o".to_string()];
        let result = service
            .rank_models(
                &models,
                &make_policy(SelectionPreference::Cheapest),
                &RankContext::default(),
            )
            .await;
        assert_eq!(result, vec!["gpt-4o", "gpt-4o-mini"]);
    }

    #[tokio::test]
    async fn test_rank_models_none_preserves_order() {
        let service = make_service(
            {
                let mut m = HashMap::new();
                m.insert("gpt-4o-mini".to_string(), 0.0001);
                m.insert("gpt-4o".to_string(), 0.005);
                m
            },
            HashMap::new(),
            HashMap::new(),
        );
        let models = vec!["gpt-4o".to_string(), "gpt-4o-mini".to_string()];
        let result = service
            .rank_models(
                &models,
                &make_policy(SelectionPreference::None),
                &RankContext::default(),
            )
            .await;
        // none → original order, despite gpt-4o-mini being cheaper
        assert_eq!(result, vec!["gpt-4o", "gpt-4o-mini"]);
    }

    /// Live DO catalog rates for `anthropic-claude-4.6-sonnet` vs a cheaper model,
    /// mirroring the worked cost model from the design doc.
    fn cache_test_pricing() -> HashMap<String, ModelPricing> {
        let mut m = HashMap::new();
        m.insert(
            "anthropic/claude-sonnet".to_string(),
            ModelPricing {
                input_per_million: 3.0,
                output_per_million: 15.0,
                cached_input_per_million: Some(0.3), // 90% off input
            },
        );
        m.insert(
            "openai/gpt-4o-mini".to_string(),
            ModelPricing {
                input_per_million: 0.15,
                output_per_million: 0.6,
                cached_input_per_million: Some(0.075),
            },
        );
        m
    }

    #[tokio::test]
    async fn test_cache_aware_keeps_warm_model_despite_cheaper_alternative() {
        // Turn N of a pinned session with a 32K-token warm prefix on Sonnet:
        // rereading the prefix at $0.30/M beats switching to a lower-sticker-price
        // model that must re-bill all 32K tokens at full input price plus the
        // cache-creation surcharge.
        let mut pricing = cache_test_pricing();
        pricing.insert(
            "openai/gpt-4o".to_string(),
            ModelPricing {
                input_per_million: 2.5,
                output_per_million: 10.0,
                cached_input_per_million: Some(1.25),
            },
        );
        let service = make_service(HashMap::new(), pricing, HashMap::new());

        let models = vec![
            "openai/gpt-4o".to_string(),
            "anthropic/claude-sonnet".to_string(),
        ];
        let ctx = RankContext {
            previous_model: Some("anthropic/claude-sonnet".to_string()),
            prefix_tokens: Some(32_000),
        };
        let result = service
            .rank_models(&models, &make_policy(SelectionPreference::CacheAware), &ctx)
            .await;
        // Warm Sonnet: 32K x $0.30/M + small = ~$0.0175 per million-scaled units.
        // Cold gpt-4o: 32K x $2.50/M x 1.25 + small = ~$0.106 — stays on Sonnet.
        assert_eq!(result[0], "anthropic/claude-sonnet");
    }

    #[tokio::test]
    async fn test_cache_aware_without_pin_ranks_by_projected_cost() {
        let service = make_service(HashMap::new(), cache_test_pricing(), HashMap::new());
        let models = vec![
            "anthropic/claude-sonnet".to_string(),
            "openai/gpt-4o-mini".to_string(),
        ];
        // No previous pin: everyone pays cold-start prices; cheaper model wins.
        let ctx = RankContext {
            previous_model: None,
            prefix_tokens: Some(32_000),
        };
        let result = service
            .rank_models(&models, &make_policy(SelectionPreference::CacheAware), &ctx)
            .await;
        assert_eq!(result[0], "openai/gpt-4o-mini");
    }

    #[tokio::test]
    async fn test_cache_aware_missing_pricing_ranked_last() {
        let service = make_service(HashMap::new(), cache_test_pricing(), HashMap::new());
        let models = vec![
            "unknown/model".to_string(),
            "anthropic/claude-sonnet".to_string(),
        ];
        let ctx = RankContext {
            previous_model: Some("anthropic/claude-sonnet".to_string()),
            prefix_tokens: Some(10_000),
        };
        let result = service
            .rank_models(&models, &make_policy(SelectionPreference::CacheAware), &ctx)
            .await;
        assert_eq!(result, vec!["anthropic/claude-sonnet", "unknown/model"]);
    }

    #[test]
    fn test_rank_by_ascending_metric_nan_treated_as_missing() {
        let models = vec![
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
            "d".to_string(),
        ];
        let mut data = HashMap::new();
        data.insert("a".to_string(), f64::NAN);
        data.insert("b".to_string(), 0.5);
        data.insert("c".to_string(), 0.1);
        // "d" has no entry at all
        let result = rank_by_ascending_metric(&models, &data);
        // c (0.1) < b (0.5), then NaN "a" and missing "d" appended in original order
        assert_eq!(result, vec!["c", "b", "a", "d"]);
    }
}
