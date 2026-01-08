use std::{collections::HashMap, sync::Arc};

use common::{
    configuration::{LlmProvider, ModelUsagePreference, RoutingPreference},
    consts::{ARCH_PROVIDER_HINT_HEADER, REQUEST_ID_HEADER, TRACE_PARENT_HEADER},
};
use hermesllm::apis::openai::{ChatCompletionsResponse, Message};
use hyper::header;
use thiserror::Error;
use tracing::{debug, info, warn};

use crate::router::router_model_v1::{self};

use super::router_model::RouterModel;

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
    #[error("Failed to send request: {0}")]
    RequestError(#[from] reqwest::Error),

    #[error("Failed to parse JSON: {0}, JSON: {1}")]
    JsonError(serde_json::Error, String),

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
            routing_model_name.clone(),
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
            "sending request to arch-router model: {}, endpoint: {}",
            self.router_model.get_model_name(),
            self.router_url
        );

        debug!(
            "arch request body: {}",
            &serde_json::to_string(&router_request).unwrap(),
        );

        let mut llm_route_request_headers = header::HeaderMap::new();
        llm_route_request_headers.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("application/json"),
        );

        llm_route_request_headers.insert(
            header::HeaderName::from_static(ARCH_PROVIDER_HINT_HEADER),
            header::HeaderValue::from_str(&self.routing_provider_name).unwrap(),
        );

        llm_route_request_headers.insert(
            header::HeaderName::from_static(TRACE_PARENT_HEADER),
            header::HeaderValue::from_str(traceparent).unwrap(),
        );

        llm_route_request_headers.insert(
            header::HeaderName::from_static(REQUEST_ID_HEADER),
            header::HeaderValue::from_str(request_id).unwrap(),
        );

        llm_route_request_headers.insert(
            header::HeaderName::from_static("model"),
            header::HeaderValue::from_static("arch-router"),
        );

        let start_time = std::time::Instant::now();
        let res = self
            .client
            .post(&self.router_url)
            .headers(llm_route_request_headers)
            .body(serde_json::to_string(&router_request).unwrap())
            .send()
            .await?;

        let body = res.text().await?;
        let router_response_time = start_time.elapsed();

        let chat_completion_response: ChatCompletionsResponse = match serde_json::from_str(&body) {
            Ok(response) => response,
            Err(err) => {
                warn!(
                    "Failed to parse JSON: {}. Body: {}",
                    err,
                    &serde_json::to_string(&body).unwrap()
                );
                return Err(RoutingError::JsonError(
                    err,
                    format!("Failed to parse JSON: {}", body),
                ));
            }
        };

        if chat_completion_response.choices.is_empty() {
            warn!("No choices in router response: {}", body);
            return Ok(None);
        }

        if let Some(content) = &chat_completion_response.choices[0].message.content {
            let parsed_response = self
                .router_model
                .parse_response(content, &usage_preferences)?;
            info!(
                "arch-router determined route: {}, selected_model: {:?}, response time: {}ms",
                content.replace("\n", "\\n"),
                parsed_response,
                router_response_time.as_millis()
            );

            if let Some(ref parsed_response) = parsed_response {
                return Ok(Some(parsed_response.clone()));
            }

            Ok(None)
        } else {
            Ok(None)
        }
    }
}
