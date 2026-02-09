use common::configuration::ModelUsagePreference;
use hermesllm::clients::endpoints::SupportedUpstreamAPIs;
use hermesllm::{ProviderRequest, ProviderRequestType};
use hyper::StatusCode;
use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::router::llm_router::RouterService;
use crate::tracing::routing;

pub struct RoutingResult {
    pub model_name: String,
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
    router_service: Arc<RouterService>,
    client_request: ProviderRequestType,
    traceparent: &str,
    request_path: &str,
    request_id: &str,
) -> Result<RoutingResult, RoutingError> {
    // Clone metadata for routing before converting (which consumes client_request)
    let routing_metadata = client_request.metadata().clone();

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

    // Extract usage preferences from metadata
    let usage_preferences_str: Option<String> = routing_metadata.as_ref().and_then(|metadata| {
        metadata
            .get("archgw_preference_config")
            .map(|value| value.to_string())
    });

    let usage_preferences: Option<Vec<ModelUsagePreference>> = usage_preferences_str
        .as_ref()
        .and_then(|s| serde_yaml::from_str(s).ok());

    // Prepare log message with latest message from chat request
    let latest_message_for_log = chat_request
        .messages
        .last()
        .map_or("None".to_string(), |msg| {
            msg.content
                .as_ref()
                .map_or("None".to_string(), |c| c.to_string().replace('\n', "\\n"))
        });

    const MAX_MESSAGE_LENGTH: usize = 50;
    let latest_message_for_log = if latest_message_for_log.chars().count() > MAX_MESSAGE_LENGTH {
        let truncated: String = latest_message_for_log
            .chars()
            .take(MAX_MESSAGE_LENGTH)
            .collect();
        format!("{}...", truncated)
    } else {
        latest_message_for_log
    };

    info!(
        has_usage_preferences = usage_preferences.is_some(),
        path = %request_path,
        latest_message = %latest_message_for_log,
        "processing router request"
    );

    // Capture start time for routing span
    let routing_start_time = std::time::Instant::now();

    // Attempt to determine route using the router service
    let routing_result = router_service
        .determine_route(
            &chat_request.messages,
            traceparent,
            usage_preferences,
            request_id,
        )
        .await;

    let determination_ms = routing_start_time.elapsed().as_millis() as i64;
    let current_span = tracing::Span::current();
    current_span.record(routing::ROUTE_DETERMINATION_MS, determination_ms);

    match routing_result {
        Ok(route) => match route {
            Some((_, model_name)) => {
                current_span.record("route.selected_model", model_name.as_str());
                Ok(RoutingResult { model_name })
            }
            None => {
                // No route determined, return sentinel value "none"
                // This signals to llm.rs to use the original validated request model
                current_span.record("route.selected_model", "none");
                info!("no route determined, using default model");

                Ok(RoutingResult {
                    model_name: "none".to_string(),
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
