use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use common::configuration::{ModelMetricsSources, SelectionPolicy, SelectionPreference};
use serde::Deserialize;
use tokio::sync::RwLock;
use tracing::{info, warn};

#[derive(Deserialize)]
struct MetricsResponse {
    #[serde(default)]
    cost: HashMap<String, f64>,
    #[serde(default)]
    latency: HashMap<String, f64>,
}

pub struct ModelMetricsService {
    cost: Arc<RwLock<HashMap<String, f64>>>,
    latency: Arc<RwLock<HashMap<String, f64>>>,
}

impl ModelMetricsService {
    pub async fn new(sources: &ModelMetricsSources, client: reqwest::Client) -> Self {
        let cost_data = Arc::new(RwLock::new(HashMap::new()));
        let latency_data = Arc::new(RwLock::new(HashMap::new()));

        let metrics = fetch_metrics(&sources.url, &client).await;
        info!(
            cost_models = metrics.cost.len(),
            latency_models = metrics.latency.len(),
            url = %sources.url,
            "fetched model metrics"
        );
        *cost_data.write().await = metrics.cost;
        *latency_data.write().await = metrics.latency;

        if let Some(interval_secs) = sources.refresh_interval {
            let cost_clone = Arc::clone(&cost_data);
            let latency_clone = Arc::clone(&latency_data);
            let client_clone = client.clone();
            let url = sources.url.clone();
            tokio::spawn(async move {
                let interval = Duration::from_secs(interval_secs);
                loop {
                    tokio::time::sleep(interval).await;
                    let metrics = fetch_metrics(&url, &client_clone).await;
                    info!(
                        cost_models = metrics.cost.len(),
                        latency_models = metrics.latency.len(),
                        url = %url,
                        "refreshed model metrics"
                    );
                    *cost_clone.write().await = metrics.cost;
                    *latency_clone.write().await = metrics.latency;
                }
            });
        }

        ModelMetricsService {
            cost: cost_data,
            latency: latency_data,
        }
    }

    /// Select the best model from `models` according to `policy`.
    /// Falls back to `models[0]` if metric data is unavailable for all candidates.
    pub async fn select_model(&self, models: &[String], policy: &SelectionPolicy) -> String {
        match policy.prefer {
            SelectionPreference::Cheapest => {
                let data = self.cost.read().await;
                select_by_ascending_metric(models, &data)
            }
            SelectionPreference::Fastest => {
                let data = self.latency.read().await;
                select_by_ascending_metric(models, &data)
            }
            SelectionPreference::Random => {
                let idx = rand_index(models.len());
                models[idx].clone()
            }
        }
    }
}

fn select_by_ascending_metric(models: &[String], data: &HashMap<String, f64>) -> String {
    models
        .iter()
        .filter_map(|m| data.get(m.as_str()).map(|v| (m, *v)))
        .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(m, _)| m.clone())
        .unwrap_or_else(|| models[0].clone())
}

/// Simple non-crypto random index using system time nanoseconds.
fn rand_index(len: usize) -> usize {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as usize)
        .unwrap_or(0);
    nanos % len
}

async fn fetch_metrics(url: &str, client: &reqwest::Client) -> MetricsResponse {
    match client.get(url).send().await {
        Ok(resp) => match resp.json::<MetricsResponse>().await {
            Ok(data) => data,
            Err(err) => {
                warn!(error = %err, url = %url, "failed to parse metrics response");
                MetricsResponse {
                    cost: HashMap::new(),
                    latency: HashMap::new(),
                }
            }
        },
        Err(err) => {
            warn!(error = %err, url = %url, "failed to fetch metrics");
            MetricsResponse {
                cost: HashMap::new(),
                latency: HashMap::new(),
            }
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
    fn test_select_by_ascending_metric_picks_lowest() {
        let models = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let mut data = HashMap::new();
        data.insert("a".to_string(), 0.01);
        data.insert("b".to_string(), 0.005);
        data.insert("c".to_string(), 0.02);
        assert_eq!(select_by_ascending_metric(&models, &data), "b");
    }

    #[test]
    fn test_select_by_ascending_metric_fallback_to_first() {
        let models = vec!["x".to_string(), "y".to_string()];
        let data = HashMap::new();
        assert_eq!(select_by_ascending_metric(&models, &data), "x");
    }

    #[test]
    fn test_select_by_ascending_metric_partial_data() {
        let models = vec!["a".to_string(), "b".to_string()];
        let mut data = HashMap::new();
        data.insert("b".to_string(), 100.0);
        assert_eq!(select_by_ascending_metric(&models, &data), "b");
    }

    #[tokio::test]
    async fn test_select_model_cheapest() {
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
            .select_model(&models, &make_policy(SelectionPreference::Cheapest))
            .await;
        assert_eq!(result, "gpt-4o-mini");
    }

    #[tokio::test]
    async fn test_select_model_fastest() {
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
            .select_model(&models, &make_policy(SelectionPreference::Fastest))
            .await;
        assert_eq!(result, "claude-sonnet");
    }

    #[tokio::test]
    async fn test_select_model_fallback_no_metrics() {
        let service = ModelMetricsService {
            cost: Arc::new(RwLock::new(HashMap::new())),
            latency: Arc::new(RwLock::new(HashMap::new())),
        };
        let models = vec!["model-a".to_string(), "model-b".to_string()];
        let result = service
            .select_model(&models, &make_policy(SelectionPreference::Cheapest))
            .await;
        assert_eq!(result, "model-a");
    }
}
