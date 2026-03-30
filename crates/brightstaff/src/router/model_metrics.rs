use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use common::configuration::{MetricsSource, SelectionPolicy, SelectionPreference};
use tokio::sync::RwLock;
use tracing::{info, warn};

const DO_PRICING_URL: &str = "https://api.digitalocean.com/v2/gen-ai/models/catalog";

pub struct ModelMetricsService {
    cost: Arc<RwLock<HashMap<String, f64>>>,
    latency: Arc<RwLock<HashMap<String, f64>>>,
}

impl ModelMetricsService {
    pub async fn new(sources: &[MetricsSource], client: reqwest::Client) -> Self {
        let cost_data = Arc::new(RwLock::new(HashMap::new()));
        let latency_data = Arc::new(RwLock::new(HashMap::new()));

        for source in sources {
            match source {
                MetricsSource::CostMetrics {
                    url,
                    refresh_interval,
                    auth,
                } => {
                    let data = fetch_cost_metrics(url, auth.as_ref(), &client).await;
                    info!(models = data.len(), url = %url, "fetched cost metrics");
                    *cost_data.write().await = data;

                    if let Some(interval_secs) = refresh_interval {
                        let cost_clone = Arc::clone(&cost_data);
                        let client_clone = client.clone();
                        let url = url.clone();
                        let auth = auth.clone();
                        let interval = Duration::from_secs(*interval_secs);
                        tokio::spawn(async move {
                            loop {
                                tokio::time::sleep(interval).await;
                                let data =
                                    fetch_cost_metrics(&url, auth.as_ref(), &client_clone).await;
                                info!(models = data.len(), url = %url, "refreshed cost metrics");
                                *cost_clone.write().await = data;
                            }
                        });
                    }
                }
                MetricsSource::PrometheusMetrics {
                    url,
                    query,
                    refresh_interval,
                } => {
                    let data = fetch_prometheus_metrics(url, query, &client).await;
                    info!(models = data.len(), url = %url, "fetched prometheus latency metrics");
                    *latency_data.write().await = data;

                    if let Some(interval_secs) = refresh_interval {
                        let latency_clone = Arc::clone(&latency_data);
                        let client_clone = client.clone();
                        let url = url.clone();
                        let query = query.clone();
                        let interval = Duration::from_secs(*interval_secs);
                        tokio::spawn(async move {
                            loop {
                                tokio::time::sleep(interval).await;
                                let data =
                                    fetch_prometheus_metrics(&url, &query, &client_clone).await;
                                info!(models = data.len(), url = %url, "refreshed prometheus latency metrics");
                                *latency_clone.write().await = data;
                            }
                        });
                    }
                }
                MetricsSource::DigitalOceanPricing {
                    refresh_interval,
                    model_aliases,
                } => {
                    let aliases = model_aliases.clone().unwrap_or_default();
                    let data = fetch_do_pricing(&client, &aliases).await;
                    info!(models = data.len(), "fetched digitalocean pricing");
                    *cost_data.write().await = data;

                    if let Some(interval_secs) = refresh_interval {
                        let cost_clone = Arc::clone(&cost_data);
                        let client_clone = client.clone();
                        let interval = Duration::from_secs(*interval_secs);
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
        match policy.prefer {
            SelectionPreference::Cheapest => {
                let data = self.cost.read().await;
                for m in models {
                    if !data.contains_key(m.as_str()) {
                        warn!(model = %m, "no cost data for model — ranking last (prefer: cheapest)");
                    }
                }
                rank_by_ascending_metric(models, &data)
            }
            SelectionPreference::Fastest => {
                let data = self.latency.read().await;
                for m in models {
                    if !data.contains_key(m.as_str()) {
                        warn!(model = %m, "no latency data for model — ranking last (prefer: fastest)");
                    }
                }
                rank_by_ascending_metric(models, &data)
            }
            SelectionPreference::None => models.to_vec(),
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
        .filter_map(|m| data.get(m.as_str()).map(|v| (m, *v)))
        .collect();
    with_data.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    let without_data: Vec<&String> = models
        .iter()
        .filter(|m| !data.contains_key(m.as_str()))
        .collect();

    with_data
        .iter()
        .map(|(m, _)| (*m).clone())
        .chain(without_data.iter().map(|m| (*m).clone()))
        .collect()
}

#[derive(serde::Deserialize)]
struct CostEntry {
    input_per_million: f64,
    output_per_million: f64,
}

async fn fetch_cost_metrics(
    url: &str,
    auth: Option<&common::configuration::MetricsAuth>,
    client: &reqwest::Client,
) -> HashMap<String, f64> {
    let mut req = client.get(url);
    if let Some(auth) = auth {
        if auth.auth_type == "bearer" {
            req = req.header("Authorization", format!("Bearer {}", auth.token));
        } else {
            warn!(auth_type = %auth.auth_type, "unsupported auth type for cost_metrics, skipping auth");
        }
    }
    match req.send().await {
        Ok(resp) => match resp.json::<HashMap<String, CostEntry>>().await {
            Ok(data) => data
                .into_iter()
                .map(|(k, v)| (k, v.input_per_million + v.output_per_million))
                .collect(),
            Err(err) => {
                warn!(error = %err, url = %url, "failed to parse cost metrics response");
                HashMap::new()
            }
        },
        Err(err) => {
            warn!(error = %err, url = %url, "failed to fetch cost metrics");
            HashMap::new()
        }
    }
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
}
