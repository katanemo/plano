use std::{collections::HashMap, sync::Arc};

use common::{
    configuration::{AgentUsagePreference, OrchestrationPreference},
    consts::{ARCH_PROVIDER_HINT_HEADER, PLANO_ORCHESTRATOR_MODEL_NAME, REQUEST_ID_HEADER},
};
use hermesllm::apis::openai::{ChatCompletionsResponse, Message};
use hyper::header;
use opentelemetry::global;
use opentelemetry_http::HeaderInjector;
use thiserror::Error;
use tracing::{debug, info, warn};

use crate::router::orchestrator_model_v1::{self};

use super::orchestrator_model::OrchestratorModel;

pub struct OrchestratorService {
    orchestrator_url: String,
    client: reqwest::Client,
    orchestrator_model: Arc<dyn OrchestratorModel>,
}

#[derive(Debug, Error)]
pub enum OrchestrationError {
    #[error("Failed to send request: {0}")]
    RequestError(#[from] reqwest::Error),

    #[error("Failed to parse JSON: {0}, JSON: {1}")]
    JsonError(serde_json::Error, String),

    #[error("Orchestrator model error: {0}")]
    OrchestratorModelError(#[from] super::orchestrator_model::OrchestratorModelError),
}

pub type Result<T> = std::result::Result<T, OrchestrationError>;

impl OrchestratorService {
    pub fn new(orchestrator_url: String, orchestration_model_name: String) -> Self {
        // Empty agent orchestrations - will be provided via usage_preferences in requests
        let agent_orchestrations: HashMap<String, Vec<OrchestrationPreference>> = HashMap::new();

        let orchestrator_model = Arc::new(orchestrator_model_v1::OrchestratorModelV1::new(
            agent_orchestrations,
            orchestration_model_name.clone(),
            orchestrator_model_v1::MAX_TOKEN_LEN,
        ));

        OrchestratorService {
            orchestrator_url,
            client: reqwest::Client::new(),
            orchestrator_model,
        }
    }

    pub async fn determine_orchestration(
        &self,
        messages: &[Message],
        usage_preferences: Option<Vec<AgentUsagePreference>>,
        request_id: Option<String>,
    ) -> Result<Option<Vec<(String, String)>>> {
        if messages.is_empty() {
            return Ok(None);
        }

        // Require usage_preferences to be provided
        if usage_preferences.is_none() || usage_preferences.as_ref().unwrap().is_empty() {
            return Ok(None);
        }

        let orchestrator_request = self
            .orchestrator_model
            .generate_request(messages, &usage_preferences);

        debug!(
            model = %self.orchestrator_model.get_model_name(),
            endpoint = %self.orchestrator_url,
            "sending request to arch-orchestrator"
        );

        debug!(
            body = %serde_json::to_string(&orchestrator_request).unwrap(),
            "arch orchestrator request"
        );

        let mut orchestration_request_headers = header::HeaderMap::new();
        orchestration_request_headers.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("application/json"),
        );

        orchestration_request_headers.insert(
            header::HeaderName::from_static(ARCH_PROVIDER_HINT_HEADER),
            header::HeaderValue::from_str(PLANO_ORCHESTRATOR_MODEL_NAME).unwrap(),
        );

        // Inject OpenTelemetry trace context from current span
        global::get_text_map_propagator(|propagator| {
            let cx =
                tracing_opentelemetry::OpenTelemetrySpanExt::context(&tracing::Span::current());
            propagator.inject_context(&cx, &mut HeaderInjector(&mut orchestration_request_headers));
        });

        if let Some(request_id) = request_id {
            orchestration_request_headers.insert(
                header::HeaderName::from_static(REQUEST_ID_HEADER),
                header::HeaderValue::from_str(&request_id).unwrap(),
            );
        }

        orchestration_request_headers.insert(
            header::HeaderName::from_static("model"),
            header::HeaderValue::from_static(PLANO_ORCHESTRATOR_MODEL_NAME),
        );

        let start_time = std::time::Instant::now();
        let res = self
            .client
            .post(&self.orchestrator_url)
            .headers(orchestration_request_headers)
            .body(serde_json::to_string(&orchestrator_request).unwrap())
            .send()
            .await?;

        let body = res.text().await?;
        let orchestrator_response_time = start_time.elapsed();

        let chat_completion_response: ChatCompletionsResponse = match serde_json::from_str(&body) {
            Ok(response) => response,
            Err(err) => {
                warn!(
                    error = %err,
                    body = %serde_json::to_string(&body).unwrap(),
                    "failed to parse json response"
                );
                return Err(OrchestrationError::JsonError(
                    err,
                    format!("Failed to parse JSON: {}", body),
                ));
            }
        };

        if chat_completion_response.choices.is_empty() {
            warn!(body = %body, "no choices in orchestrator response");
            return Ok(None);
        }

        if let Some(content) = &chat_completion_response.choices[0].message.content {
            let parsed_response = self
                .orchestrator_model
                .parse_response(content, &usage_preferences)?;
            info!(
                content = %content.replace("\n", "\\n"),
                selected_routes = ?parsed_response,
                response_time_ms = orchestrator_response_time.as_millis(),
                "arch-orchestrator determined routes"
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
