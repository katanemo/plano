use std::{collections::HashMap, sync::Arc};

use common::{
    configuration::{AgentUsagePreference, OrchestrationPreference},
    consts::{ARCH_PROVIDER_HINT_HEADER, PLANO_ORCHESTRATOR_MODEL_NAME, REQUEST_ID_HEADER},
};
use hermesllm::apis::openai::Message;
use hyper::header;
use opentelemetry::global;
use opentelemetry_http::HeaderInjector;
use thiserror::Error;
use tracing::{debug, info};

use super::http::{self, post_and_extract_content};
use super::orchestrator_model::OrchestratorModel;

use crate::router::orchestrator_model_v1;

pub struct OrchestratorService {
    orchestrator_url: String,
    client: reqwest::Client,
    orchestrator_model: Arc<dyn OrchestratorModel>,
}

#[derive(Debug, Error)]
pub enum OrchestrationError {
    #[error(transparent)]
    Http(#[from] http::HttpError),

    #[error("Orchestrator model error: {0}")]
    OrchestratorModelError(#[from] super::orchestrator_model::OrchestratorModelError),
}

pub type Result<T> = std::result::Result<T, OrchestrationError>;

impl OrchestratorService {
    pub fn new(orchestrator_url: String, orchestration_model_name: String) -> Self {
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

        let body = serde_json::to_string(&orchestrator_request).unwrap();
        debug!(body = %body, "arch orchestrator request");

        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("application/json"),
        );
        headers.insert(
            header::HeaderName::from_static(ARCH_PROVIDER_HINT_HEADER),
            header::HeaderValue::from_str(PLANO_ORCHESTRATOR_MODEL_NAME).unwrap(),
        );

        // Inject OpenTelemetry trace context from current span
        global::get_text_map_propagator(|propagator| {
            let cx =
                tracing_opentelemetry::OpenTelemetrySpanExt::context(&tracing::Span::current());
            propagator.inject_context(&cx, &mut HeaderInjector(&mut headers));
        });

        if let Some(ref request_id) = request_id {
            headers.insert(
                header::HeaderName::from_static(REQUEST_ID_HEADER),
                header::HeaderValue::from_str(request_id).unwrap(),
            );
        }

        headers.insert(
            header::HeaderName::from_static("model"),
            header::HeaderValue::from_static(PLANO_ORCHESTRATOR_MODEL_NAME),
        );

        let Some((content, elapsed)) =
            post_and_extract_content(&self.client, &self.orchestrator_url, headers, body).await?
        else {
            return Ok(None);
        };

        let parsed = self
            .orchestrator_model
            .parse_response(&content, &usage_preferences)?;

        info!(
            content = %content.replace("\n", "\\n"),
            selected_routes = ?parsed,
            response_time_ms = elapsed.as_millis(),
            "arch-orchestrator determined routes"
        );

        Ok(parsed)
    }
}
