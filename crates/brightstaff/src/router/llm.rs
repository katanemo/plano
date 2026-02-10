use std::{collections::HashMap, sync::Arc};

use common::{
    configuration::{LlmProvider, ModelUsagePreference, RoutingPreference},
    consts::{ARCH_PROVIDER_HINT_HEADER, REQUEST_ID_HEADER, TRACE_PARENT_HEADER},
};
use hermesllm::apis::openai::Message;
use hyper::header;
use thiserror::Error;
use tracing::{debug, info};

use super::http::{self, post_and_extract_content};
use super::router_model::RouterModel;

use crate::router::router_model_v1;

pub struct RouterService {
    router_url: String,
    client: reqwest::Client,
    router_model: Arc<dyn RouterModel>,
    #[allow(dead_code)]
    routing_provider_name: String,
    llm_usage_defined: bool,
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
        router_url: String,
        routing_model_name: String,
        routing_provider_name: String,
    ) -> Self {
        let providers_with_usage = providers
            .iter()
            .filter(|provider| provider.routing_preferences.is_some())
            .cloned()
            .collect::<Vec<LlmProvider>>();

        let llm_routes: HashMap<String, Vec<RoutingPreference>> = providers_with_usage
            .iter()
            .filter_map(|provider| {
                provider
                    .routing_preferences
                    .as_ref()
                    .map(|prefs| (provider.name.clone(), prefs.clone()))
            })
            .collect();

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
            llm_usage_defined: !providers_with_usage.is_empty(),
        }
    }

    pub async fn determine_route(
        &self,
        messages: &[Message],
        traceparent: &str,
        usage_preferences: Option<Vec<ModelUsagePreference>>,
        request_id: &str,
    ) -> Result<Option<(String, String)>> {
        if messages.is_empty() {
            return Ok(None);
        }

        if (usage_preferences.is_none() || usage_preferences.as_ref().unwrap().len() < 2)
            && !self.llm_usage_defined
        {
            return Ok(None);
        }

        let router_request = self
            .router_model
            .generate_request(messages, &usage_preferences);

        debug!(
            model = %self.router_model.get_model_name(),
            endpoint = %self.router_url,
            "sending request to arch-router"
        );

        let body = serde_json::to_string(&router_request).unwrap();
        debug!(body = %body, "arch router request");

        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("application/json"),
        );
        headers.insert(
            header::HeaderName::from_static(ARCH_PROVIDER_HINT_HEADER),
            header::HeaderValue::from_str(&self.routing_provider_name).unwrap(),
        );
        headers.insert(
            header::HeaderName::from_static(TRACE_PARENT_HEADER),
            header::HeaderValue::from_str(traceparent).unwrap(),
        );
        headers.insert(
            header::HeaderName::from_static(REQUEST_ID_HEADER),
            header::HeaderValue::from_str(request_id).unwrap(),
        );
        headers.insert(
            header::HeaderName::from_static("model"),
            header::HeaderValue::from_static("arch-router"),
        );

        let Some((content, elapsed)) =
            post_and_extract_content(&self.client, &self.router_url, headers, body).await?
        else {
            return Ok(None);
        };

        let parsed = self
            .router_model
            .parse_response(&content, &usage_preferences)?;

        info!(
            content = %content.replace("\n", "\\n"),
            selected_model = ?parsed,
            response_time_ms = elapsed.as_millis(),
            "arch-router determined route"
        );

        Ok(parsed)
    }
}
