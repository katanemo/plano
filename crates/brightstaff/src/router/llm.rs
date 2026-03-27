use std::{collections::HashMap, sync::Arc};

use common::{
    configuration::{
        LlmProvider, ModelUsagePreference, RoutingPreference, TopLevelRoutingPreference,
    },
    consts::{ARCH_PROVIDER_HINT_HEADER, REQUEST_ID_HEADER, TRACE_PARENT_HEADER},
};
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
    llm_usage_defined: bool,
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
        providers: Vec<LlmProvider>,
        top_level_prefs: Option<Vec<TopLevelRoutingPreference>>,
        metrics_service: Option<Arc<ModelMetricsService>>,
        router_url: String,
        routing_model_name: String,
        routing_provider_name: String,
    ) -> Self {
        // Build top-level preference map and sentinel llm_routes when v0.4.0 format is used.
        let (top_level_preferences, llm_routes, llm_usage_defined) =
            if let Some(top_prefs) = top_level_prefs {
                let top_level_map: HashMap<String, TopLevelRoutingPreference> =
                    top_prefs.into_iter().map(|p| (p.name.clone(), p)).collect();
                // Build sentinel routes: route_name → first model (RouterModelV1 needs a model
                // mapping, but RouterService overrides the selection via metrics_service).
                let sentinel_routes: HashMap<String, Vec<RoutingPreference>> = top_level_map
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
                let defined = !top_level_map.is_empty();
                (top_level_map, sentinel_routes, defined)
            } else {
                // Legacy per-provider format.
                let providers_with_usage = providers
                    .iter()
                    .filter(|provider| provider.routing_preferences.is_some())
                    .cloned()
                    .collect::<Vec<LlmProvider>>();

                let routes: HashMap<String, Vec<RoutingPreference>> = providers_with_usage
                    .iter()
                    .filter_map(|provider| {
                        provider
                            .routing_preferences
                            .as_ref()
                            .map(|prefs| (provider.name.clone(), prefs.clone()))
                    })
                    .collect();

                let defined = !providers_with_usage.is_empty();
                (HashMap::new(), routes, defined)
            };

        let router_model = Arc::new(router_model_v1::RouterModelV1::new(
            llm_routes,
            routing_model_name,
            router_model_v1::MAX_TOKEN_LEN,
        ));

        RouterService {
            router_url,
            client: reqwest::Client::new(),
            router_model,
            routing_provider_name,
            llm_usage_defined,
            top_level_preferences,
            metrics_service,
        }
    }

    pub async fn determine_route(
        &self,
        messages: &[Message],
        traceparent: &str,
        usage_preferences: Option<Vec<ModelUsagePreference>>,
        inline_routing_preferences: Option<Vec<TopLevelRoutingPreference>>,
        request_id: &str,
    ) -> Result<Option<(String, String)>> {
        if messages.is_empty() {
            return Ok(None);
        }

        // Build inline top-level map from request if present (inline overrides config).
        let inline_top_map: Option<HashMap<String, TopLevelRoutingPreference>> =
            inline_routing_preferences
                .map(|prefs| prefs.into_iter().map(|p| (p.name.clone(), p)).collect());

        // Determine whether any routing is defined.
        let has_top_level = inline_top_map.is_some() || !self.top_level_preferences.is_empty();

        if usage_preferences
            .as_ref()
            .is_none_or(|prefs| prefs.len() < 2)
            && !self.llm_usage_defined
            && !has_top_level
        {
            return Ok(None);
        }

        // For top-level format, build a synthetic ModelUsagePreference list so RouterModelV1
        // generates the correct prompt (route name + description pairs).
        let effective_usage_preferences: Option<Vec<ModelUsagePreference>> =
            if let Some(ref inline_map) = inline_top_map {
                Some(
                    inline_map
                        .values()
                        .map(|p| ModelUsagePreference {
                            model: p.models.first().cloned().unwrap_or_default(),
                            routing_preferences: vec![RoutingPreference {
                                name: p.name.clone(),
                                description: p.description.clone(),
                            }],
                        })
                        .collect(),
                )
            } else if !self.top_level_preferences.is_empty() {
                // Config top-level prefs: already encoded as sentinel routes in RouterModelV1,
                // pass None so it uses the pre-built llm_route_json_str.
                None
            } else {
                usage_preferences.clone()
            };

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

        let result = if let Some((route_name, _sentinel_model)) = parsed {
            // Check if this route belongs to the top-level preference format.
            let top_pref = inline_top_map
                .as_ref()
                .and_then(|m| m.get(&route_name))
                .or_else(|| self.top_level_preferences.get(&route_name));

            if let Some(pref) = top_pref {
                let selected_model = match &self.metrics_service {
                    Some(svc) => svc.select_model(&pref.models, &pref.selection_policy).await,
                    None => pref.models.first().cloned().unwrap_or_default(),
                };
                Some((route_name, selected_model))
            } else {
                Some((route_name, _sentinel_model))
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
