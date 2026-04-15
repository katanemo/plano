use common::configuration::TopLevelRoutingPreference;
use hermesllm::clients::endpoints::SupportedUpstreamAPIs;
use hermesllm::ProviderRequestType;
use hyper::StatusCode;
use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::router::orchestrator::OrchestratorService;
use crate::streaming::truncate_message;
use crate::tracing::routing;

pub struct RoutingResult {
    /// Primary model to use (first in the ranked list).
    pub model_name: String,
    /// Full ranked list — use subsequent entries as fallbacks on 429/5xx.
    pub models: Vec<String>,
    pub route_name: Option<String>,
}

pub struct RoutingError {
    pub message: String,
    pub status_code: StatusCode,
}

impl RoutingError {
    pub fn internal_error(message: String) -> Self {
        Self {
            message,
            status_code: StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// Determines the routing decision if
///
/// # Returns
/// * `Ok(RoutingResult)` - Contains the selected model name and span ID
/// * `Err(RoutingError)` - Contains error details and optional span ID
pub async fn router_chat_get_upstream_model(
    orchestrator_service: Arc<OrchestratorService>,
    client_request: ProviderRequestType,
    request_path: &str,
    request_id: &str,
    inline_routing_preferences: Option<Vec<TopLevelRoutingPreference>>,
) -> Result<RoutingResult, RoutingError> {
    // Convert to ChatCompletionsRequest for routing (regardless of input type)
    let chat_request = match ProviderRequestType::try_from((
        client_request,
        &SupportedUpstreamAPIs::OpenAIChatCompletions(hermesllm::apis::OpenAIApi::ChatCompletions),
    )) {
        Ok(ProviderRequestType::ChatCompletionsRequest(req)) => req,
        Ok(
            ProviderRequestType::MessagesRequest(_)
            | ProviderRequestType::BedrockConverse(_)
            | ProviderRequestType::BedrockConverseStream(_)
            | ProviderRequestType::ResponsesAPIRequest(_),
        ) => {
            warn!("unexpected: got non-ChatCompletions request after converting to OpenAI format");
            return Err(RoutingError::internal_error(
                "Request conversion failed".to_string(),
            ));
        }
        Err(err) => {
            warn!(
                "failed to convert request to ChatCompletionsRequest: {}",
                err
            );
            return Err(RoutingError::internal_error(format!(
                "Failed to convert request: {}",
                err
            )));
        }
    };

    debug!(
        request = %serde_json::to_string(&chat_request).unwrap(),
        "router request"
    );

    // Prepare log message with latest message from chat request
    let latest_message_for_log = chat_request
        .messages
        .last()
        .map_or("None".to_string(), |msg| {
            msg.content
                .as_ref()
                .map_or("None".to_string(), |c| c.to_string().replace('\n', "\\n"))
        });

    let latest_message_for_log = truncate_message(&latest_message_for_log, 50);

    info!(
        path = %request_path,
        latest_message = %latest_message_for_log,
        "processing router request"
    );

    // Capture start time for routing span
    let routing_start_time = std::time::Instant::now();

    let routing_result = orchestrator_service
        .determine_route(
            &chat_request.messages,
            inline_routing_preferences,
            request_id,
        )
        .await;

    let determination_ms = routing_start_time.elapsed().as_millis() as i64;
    let current_span = tracing::Span::current();
    current_span.record(routing::ROUTE_DETERMINATION_MS, determination_ms);

    match routing_result {
        Ok(route) => match route {
            Some((route_name, ranked_models)) => {
                let model_name = ranked_models.first().cloned().unwrap_or_default();
                current_span.record("route.selected_model", model_name.as_str());
                Ok(RoutingResult {
                    model_name,
                    models: ranked_models,
                    route_name: Some(route_name),
                })
            }
            None => {
                // No route determined, return sentinel value "none"
                // This signals to llm.rs to use the original validated request model
                current_span.record("route.selected_model", "none");
                info!("no route determined, using default model");

                Ok(RoutingResult {
                    model_name: "none".to_string(),
                    models: vec!["none".to_string()],
                    route_name: None,
                })
            }
        },
        Err(err) => {
            current_span.record("route.selected_model", "unknown");
            Err(RoutingError::internal_error(format!(
                "Failed to determine route: {}",
                err
            )))
        }
    }
}
