use std::{collections::HashMap, sync::Arc};

use common::{
    configuration::TopLevelRoutingPreference,
    consts::{ARCH_PROVIDER_HINT_HEADER, REQUEST_ID_HEADER, TRACE_PARENT_HEADER},
};

use super::router_model::{ModelUsagePreference, RoutingPreference};
use hermesllm::apis::openai::Message;
use hyper::header;
use thiserror::Error;
use tracing::{debug, info};

use super::http::{self, post_and_extract_content};
use super::model_metrics::ModelMetricsService;
use super::router_model::RouterModel;

use crate::router::router_model_v1;

pub struct RouterService {
    router_url: String,
    client: reqwest::Client,
    router_model: Arc<dyn RouterModel>,
    routing_provider_name: String,
    top_level_preferences: HashMap<String, TopLevelRoutingPreference>,
    metrics_service: Option<Arc<ModelMetricsService>>,
}

#[derive(Debug, Error)]
pub enum RoutingError {
    #[error(transparent)]
    Http(#[from] http::HttpError),

    #[error("Router model error: {0}")]
    RouterModelError(#[from] super::router_model::RoutingModelError),
}

pub type Result<T> = std::result::Result<T, RoutingError>;

impl RouterService {
    pub fn new(
        top_level_prefs: Option<Vec<TopLevelRoutingPreference>>,
        metrics_service: Option<Arc<ModelMetricsService>>,
        router_url: String,
        routing_model_name: String,
        routing_provider_name: String,
    ) -> Self {
        let top_level_preferences: HashMap<String, TopLevelRoutingPreference> = top_level_prefs
            .map_or_else(HashMap::new, |prefs| {
                prefs.into_iter().map(|p| (p.name.clone(), p)).collect()
            });

        // Build sentinel routes for RouterModelV1: route_name → first model.
        // RouterModelV1 uses this to build its prompt; RouterService overrides
        // the model selection via rank_models() after the route is determined.
        let sentinel_routes: HashMap<String, Vec<RoutingPreference>> = top_level_preferences
            .iter()
            .filter_map(|(name, pref)| {
                pref.models.first().map(|first_model| {
                    (
                        first_model.clone(),
                        vec![RoutingPreference {
                            name: name.clone(),
                            description: pref.description.clone(),
                        }],
                    )
                })
            })
            .collect();

        let router_model = Arc::new(router_model_v1::RouterModelV1::new(
            sentinel_routes,
            routing_model_name,
            router_model_v1::MAX_TOKEN_LEN,
        ));

        RouterService {
            router_url,
            client: reqwest::Client::new(),
            router_model,
            routing_provider_name,
            top_level_preferences,
            metrics_service,
        }
    }

    pub async fn determine_route(
        &self,
        messages: &[Message],
        traceparent: &str,
        inline_routing_preferences: Option<Vec<TopLevelRoutingPreference>>,
        request_id: &str,
    ) -> Result<Option<(String, Vec<String>)>> {
        if messages.is_empty() {
            return Ok(None);
        }

        // Build inline top-level map from request if present (inline overrides config).
        let inline_top_map: Option<HashMap<String, TopLevelRoutingPreference>> =
            inline_routing_preferences
                .map(|prefs| prefs.into_iter().map(|p| (p.name.clone(), p)).collect());

        // No routing defined — skip the router call entirely.
        if inline_top_map.is_none() && self.top_level_preferences.is_empty() {
            return Ok(None);
        }

        // For inline overrides, build synthetic ModelUsagePreference list so RouterModelV1
        // generates the correct prompt (route name + description pairs).
        // For config-level prefs the sentinel routes are already baked into RouterModelV1.
        let effective_usage_preferences: Option<Vec<ModelUsagePreference>> =
            inline_top_map.as_ref().map(|inline_map| {
                inline_map
                    .values()
                    .map(|p| ModelUsagePreference {
                        model: p.models.first().cloned().unwrap_or_default(),
                        routing_preferences: vec![RoutingPreference {
                            name: p.name.clone(),
                            description: p.description.clone(),
                        }],
                    })
                    .collect()
            });

        let router_request = self
            .router_model
            .generate_request(messages, &effective_usage_preferences);

        debug!(
            model = %self.router_model.get_model_name(),
            endpoint = %self.router_url,
            "sending request to arch-router"
        );

        let body = serde_json::to_string(&router_request)
            .map_err(super::router_model::RoutingModelError::from)?;
        debug!(body = %body, "arch router request");

        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("application/json"),
        );
        if let Ok(val) = header::HeaderValue::from_str(&self.routing_provider_name) {
            headers.insert(
                header::HeaderName::from_static(ARCH_PROVIDER_HINT_HEADER),
                val,
            );
        }
        if let Ok(val) = header::HeaderValue::from_str(traceparent) {
            headers.insert(header::HeaderName::from_static(TRACE_PARENT_HEADER), val);
        }
        if let Ok(val) = header::HeaderValue::from_str(request_id) {
            headers.insert(header::HeaderName::from_static(REQUEST_ID_HEADER), val);
        }
        headers.insert(
            header::HeaderName::from_static("model"),
            header::HeaderValue::from_static("arch-router"),
        );

        let Some((content, elapsed)) =
            post_and_extract_content(&self.client, &self.router_url, headers, body).await?
        else {
            return Ok(None);
        };

        // Parse the route name from the router response.
        let parsed = self
            .router_model
            .parse_response(&content, &effective_usage_preferences)?;

        let result = if let Some((route_name, _sentinel)) = parsed {
            let top_pref = inline_top_map
                .as_ref()
                .and_then(|m| m.get(&route_name))
                .or_else(|| self.top_level_preferences.get(&route_name));

            if let Some(pref) = top_pref {
                let ranked = match &self.metrics_service {
                    Some(svc) => svc.rank_models(&pref.models, &pref.selection_policy).await,
                    None => pref.models.clone(),
                };
                Some((route_name, ranked))
            } else {
                None
            }
        } else {
            None
        };

        info!(
            content = %content.replace("\n", "\\n"),
            selected_model = ?result,
            response_time_ms = elapsed.as_millis(),
            "arch-router determined route"
        );

        Ok(result)
    }
}
