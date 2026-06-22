use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use common::configuration::{
    CostProvider, LatencyProvider, MetricsSource, SelectionPolicy, SelectionPreference,
};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

const DO_PRICING_URL: &str = "https://api.digitalocean.com/v2/gen-ai/models/catalog";

/// Routing modality used to scope the per-modality `fastest` latency signal
/// (WS10). Latency is keyed by `(model, modality)`: text keeps its existing
/// TTFT behavior; image/audio carry their own per-modality timing definitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modality {
    Text,
    Vision,
    ImageOut,
    AudioOut,
}

impl Modality {
    pub fn as_str(&self) -> &'static str {
        match self {
            Modality::Text => "text",
            Modality::Vision => "vision",
            Modality::ImageOut => "image_out",
            Modality::AudioOut => "audio_out",
        }
    }
}

/// Composite latency-map key scoping a model's latency to a modality. Text uses
/// the bare model id (back-compat with the existing flat Prometheus map); other
/// modalities are namespaced so they never collide with the text signal.
pub fn modality_latency_key(model: &str, modality: Modality) -> String {
    match modality {
        Modality::Text => model.to_string(),
        other => format!("{}\u{1}{}", other.as_str(), model),
    }
}

pub struct ModelMetricsService {
    cost: Arc<RwLock<HashMap<String, f64>>>,
    /// Latency map keyed by either a bare model id (text) or a
    /// `modality\u{1}model` composite (see [`modality_latency_key`]).
    latency: Arc<RwLock<HashMap<String, f64>>>,
}

impl ModelMetricsService {
    pub async fn new(sources: &[MetricsSource], client: reqwest::Client) -> Self {
        let cost_data = Arc::new(RwLock::new(HashMap::new()));
        let latency_data = Arc::new(RwLock::new(HashMap::new()));

        for source in sources {
            match source {
                MetricsSource::Cost(cfg) => match cfg.provider {
                    CostProvider::Digitalocean => {
                        let aliases = cfg.model_aliases.clone().unwrap_or_default();
                        let data = fetch_do_pricing(&client, &aliases).await;
                        info!(models = data.len(), "fetched digitalocean pricing");
                        *cost_data.write().await = data;

                        if let Some(interval_secs) = cfg.refresh_interval {
                            let cost_clone = Arc::clone(&cost_data);
                            let client_clone = client.clone();
                            let interval = Duration::from_secs(interval_secs);
                            tokio::spawn(async move {
                                loop {
                                    tokio::time::sleep(interval).await;
                                    let data = fetch_do_pricing(&client_clone, &aliases).await;
                                    info!(models = data.len(), "refreshed digitalocean pricing");
                                    *cost_clone.write().await = data;
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
            latency: latency_data,
        }
    }

    /// Rank `models` by `policy`, returning them in preference order.
    /// Models with no metric data are appended at the end in their original order.
    pub async fn rank_models(&self, models: &[String], policy: &SelectionPolicy) -> Vec<String> {
        let cost_data = self.cost.read().await;
        let latency_data = self.latency.read().await;
        debug!(
            input_models = ?models,
            cost_data = ?cost_data.iter().collect::<Vec<_>>(),
            latency_data = ?latency_data.iter().collect::<Vec<_>>(),
            prefer = ?policy.prefer,
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
            SelectionPreference::LongContextQuality => {
                // Tier 2: rank by Plano's internal long-context-quality dataset.
                // Higher score is better, so rank descending; unscored models last.
                let scores: HashMap<String, f64> = models
                    .iter()
                    .filter_map(|m| {
                        match hermesllm::long_context_quality_score(m) {
                            Some(s) => Some((m.clone(), s)),
                            None => {
                                warn!(model = %m, "no long-context-quality score — ranking last (prefer: long_context_quality)");
                                None
                            }
                        }
                    })
                    .collect();
                rank_by_descending_metric(models, &scores)
            }
            SelectionPreference::None => models.to_vec(),
        }
    }

    /// Rank `models` by latency for a specific `modality` (WS10 groundwork).
    /// Looks up the `(model, modality)` composite key first, then falls back to
    /// the bare model latency, then ranks last. For `Modality::Text` this is
    /// identical to `prefer: fastest` today.
    pub async fn rank_models_by_modality_latency(
        &self,
        models: &[String],
        modality: Modality,
    ) -> Vec<String> {
        let latency_data = self.latency.read().await;
        let resolved: HashMap<String, f64> = models
            .iter()
            .filter_map(|m| {
                let composite = modality_latency_key(m, modality);
                let v = latency_data
                    .get(&composite)
                    .or_else(|| latency_data.get(m.as_str()))
                    .copied();
                match v {
                    Some(v) if !v.is_nan() => Some((m.clone(), v)),
                    _ => {
                        warn!(model = %m, modality = modality.as_str(), "no per-modality latency — ranking last (prefer: fastest)");
                        None
                    }
                }
            })
            .collect();
        rank_by_ascending_metric(models, &resolved)
    }

    /// Seed the latency map for cold start (WS10): at launch there is no live
    /// traffic, so `prefer: fastest` on the new modalities must be primed from
    /// benchmark data. Entries should use [`modality_latency_key`] for non-text
    /// modalities. Existing keys are overwritten; live data takes over on the
    /// next Prometheus refresh.
    pub async fn seed_latency(&self, seed: HashMap<String, f64>) {
        let mut latency = self.latency.write().await;
        for (k, v) in seed {
            latency.insert(k, v);
        }
    }

    /// Returns a snapshot of the current cost data. Used at startup to warn about unmatched models.
    pub async fn cost_snapshot(&self) -> HashMap<String, f64> {
        self.cost.read().await.clone()
    }

    /// Returns a snapshot of the current latency data. Used at startup to warn about unmatched models.
    pub async fn latency_snapshot(&self) -> HashMap<String, f64> {
        self.latency.read().await.clone()
    }
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

/// Rank `models` by a metric where **higher is better** (e.g. quality scores).
/// Models with no data are appended at the end in their original order.
fn rank_by_descending_metric(models: &[String], data: &HashMap<String, f64>) -> Vec<String> {
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
    with_data.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

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
}

async fn fetch_do_pricing(
    client: &reqwest::Client,
    aliases: &HashMap<String, String>,
) -> HashMap<String, f64> {
    match client.get(DO_PRICING_URL).send().await {
        Ok(resp) => match resp.json::<DoModelList>().await {
            Ok(list) => list
                .data
                .into_iter()
                .filter_map(|m| {
                    let pricing = m.pricing?;
                    let raw_key = m.model_id.clone();
                    let key = aliases.get(&raw_key).cloned().unwrap_or(raw_key);
                    let cost = pricing.input_price_per_million.unwrap_or(0.0)
                        + pricing.output_price_per_million.unwrap_or(0.0);
                    Some((key, cost))
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
                    // If the series carries a `modality` label, namespace the key
                    // so per-modality latency (WS10) never collides with text.
                    let key = match r.metric.get("modality").map(String::as_str) {
                        Some("text") | None => model_name,
                        Some("vision") => modality_latency_key(&model_name, Modality::Vision),
                        Some("image_out") => modality_latency_key(&model_name, Modality::ImageOut),
                        Some("audio_out") => modality_latency_key(&model_name, Modality::AudioOut),
                        Some(_) => model_name,
                    };
                    Some((key, value))
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
        let service = ModelMetricsService {
            cost: Arc::new(RwLock::new({
                let mut m = HashMap::new();
                m.insert("gpt-4o".to_string(), 0.005);
                m.insert("gpt-4o-mini".to_string(), 0.0001);
                m
            })),
            latency: Arc::new(RwLock::new(HashMap::new())),
        };
        let models = vec!["gpt-4o".to_string(), "gpt-4o-mini".to_string()];
        let result = service
            .rank_models(&models, &make_policy(SelectionPreference::Cheapest))
            .await;
        assert_eq!(result, vec!["gpt-4o-mini", "gpt-4o"]);
    }

    #[tokio::test]
    async fn test_rank_models_fastest() {
        let service = ModelMetricsService {
            cost: Arc::new(RwLock::new(HashMap::new())),
            latency: Arc::new(RwLock::new({
                let mut m = HashMap::new();
                m.insert("gpt-4o".to_string(), 200.0);
                m.insert("claude-sonnet".to_string(), 120.0);
                m
            })),
        };
        let models = vec!["gpt-4o".to_string(), "claude-sonnet".to_string()];
        let result = service
            .rank_models(&models, &make_policy(SelectionPreference::Fastest))
            .await;
        assert_eq!(result, vec!["claude-sonnet", "gpt-4o"]);
    }

    #[test]
    fn test_rank_by_descending_metric_picks_highest_first() {
        let models = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let mut data = HashMap::new();
        data.insert("a".to_string(), 0.80);
        data.insert("b".to_string(), 0.95);
        data.insert("c".to_string(), 0.70);
        // unscored models would sort last; here all scored, highest first
        assert_eq!(
            rank_by_descending_metric(&models, &data),
            vec!["b", "a", "c"]
        );
    }

    #[tokio::test]
    async fn test_rank_models_long_context_quality() {
        // Uses the vendored internal LCQ dataset. gemini-2.5-pro outranks gpt-4o,
        // and an unscored model sorts last.
        let service = ModelMetricsService {
            cost: Arc::new(RwLock::new(HashMap::new())),
            latency: Arc::new(RwLock::new(HashMap::new())),
        };
        let models = vec![
            "openai/gpt-4o".to_string(),
            "openai/no-lcq-model".to_string(),
            "gemini/gemini-2.5-pro".to_string(),
        ];
        let result = service
            .rank_models(
                &models,
                &make_policy(SelectionPreference::LongContextQuality),
            )
            .await;
        assert_eq!(
            result,
            vec![
                "gemini/gemini-2.5-pro",
                "openai/gpt-4o",
                "openai/no-lcq-model"
            ]
        );
    }

    #[tokio::test]
    async fn test_rank_models_fallback_no_metrics() {
        let service = ModelMetricsService {
            cost: Arc::new(RwLock::new(HashMap::new())),
            latency: Arc::new(RwLock::new(HashMap::new())),
        };
        let models = vec!["model-a".to_string(), "model-b".to_string()];
        let result = service
            .rank_models(&models, &make_policy(SelectionPreference::Cheapest))
            .await;
        assert_eq!(result, vec!["model-a", "model-b"]);
    }

    #[tokio::test]
    async fn test_rank_models_partial_data_appended_last() {
        let service = ModelMetricsService {
            cost: Arc::new(RwLock::new({
                let mut m = HashMap::new();
                m.insert("gpt-4o".to_string(), 0.005);
                m
            })),
            latency: Arc::new(RwLock::new(HashMap::new())),
        };
        let models = vec!["gpt-4o-mini".to_string(), "gpt-4o".to_string()];
        let result = service
            .rank_models(&models, &make_policy(SelectionPreference::Cheapest))
            .await;
        assert_eq!(result, vec!["gpt-4o", "gpt-4o-mini"]);
    }

    #[tokio::test]
    async fn test_rank_models_none_preserves_order() {
        let service = ModelMetricsService {
            cost: Arc::new(RwLock::new({
                let mut m = HashMap::new();
                m.insert("gpt-4o-mini".to_string(), 0.0001);
                m.insert("gpt-4o".to_string(), 0.005);
                m
            })),
            latency: Arc::new(RwLock::new(HashMap::new())),
        };
        let models = vec!["gpt-4o".to_string(), "gpt-4o-mini".to_string()];
        let result = service
            .rank_models(&models, &make_policy(SelectionPreference::None))
            .await;
        // none → original order, despite gpt-4o-mini being cheaper
        assert_eq!(result, vec!["gpt-4o", "gpt-4o-mini"]);
    }

    #[test]
    fn test_modality_latency_key_namespaces_non_text() {
        assert_eq!(
            modality_latency_key("openai/gpt-4o", Modality::Text),
            "openai/gpt-4o"
        );
        assert_eq!(
            modality_latency_key("openai/gpt-image-1", Modality::ImageOut),
            "image_out\u{1}openai/gpt-image-1"
        );
        assert_ne!(
            modality_latency_key("m", Modality::Vision),
            modality_latency_key("m", Modality::AudioOut)
        );
    }

    #[tokio::test]
    async fn test_rank_models_by_modality_latency_prefers_composite_key() {
        let service = ModelMetricsService {
            cost: Arc::new(RwLock::new(HashMap::new())),
            latency: Arc::new(RwLock::new(HashMap::new())),
        };
        // Seed per-modality (audio) latency for two TTS models.
        let mut seed = HashMap::new();
        seed.insert(
            modality_latency_key("openai/tts-slow", Modality::AudioOut),
            500.0,
        );
        seed.insert(
            modality_latency_key("openai/tts-fast", Modality::AudioOut),
            100.0,
        );
        service.seed_latency(seed).await;

        let models = vec!["openai/tts-slow".to_string(), "openai/tts-fast".to_string()];
        let result = service
            .rank_models_by_modality_latency(&models, Modality::AudioOut)
            .await;
        assert_eq!(result, vec!["openai/tts-fast", "openai/tts-slow"]);
    }

    #[tokio::test]
    async fn test_rank_models_by_modality_latency_falls_back_to_bare_model() {
        let service = ModelMetricsService {
            cost: Arc::new(RwLock::new(HashMap::new())),
            latency: Arc::new(RwLock::new({
                let mut m = HashMap::new();
                m.insert("text-model".to_string(), 42.0);
                m
            })),
        };
        let models = vec!["text-model".to_string(), "no-data".to_string()];
        let result = service
            .rank_models_by_modality_latency(&models, Modality::Vision)
            .await;
        // bare-model fallback ranks first; the unseen model sorts last.
        assert_eq!(result, vec!["text-model", "no-data"]);
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
