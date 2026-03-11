use common::configuration::ModelUsagePreference;
use hermesllm::clients::endpoints::SupportedUpstreamAPIs;
use hermesllm::{ProviderRequest, ProviderRequestType};
use hyper::StatusCode;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::handlers::policy_provider::PolicyProviderClient;
use crate::router::llm_router::RouterService;
use crate::tracing::routing;

pub struct RoutingResult {
    pub model_name: String,
    pub route_name: Option<String>,
}

#[derive(Debug)]
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

    pub fn bad_request(message: String) -> Self {
        Self {
            message,
            status_code: StatusCode::BAD_REQUEST,
        }
    }
}

async fn resolve_usage_preferences(
    inline_usage_preferences: Option<Vec<ModelUsagePreference>>,
    policy_id: Option<&str>,
    policy_provider: Option<&PolicyProviderClient>,
    routing_metadata: Option<&HashMap<String, Value>>,
) -> Result<Option<Vec<ModelUsagePreference>>, RoutingError> {
    if let Some(inline_preferences) = inline_usage_preferences {
        info!("using inline routing_policy from request body");
        return Ok(Some(inline_preferences));
    }

    if let (Some(policy_id), Some(policy_provider_client)) = (policy_id, policy_provider) {
        match policy_provider_client.fetch_policy(policy_id).await {
            Ok(preferences) => {
                info!(
                    policy_id,
                    "using routing policy from external policy provider"
                );
                return Ok(Some(preferences));
            }
            Err(err) if err.is_transient() => {
                warn!(
                    policy_id,
                    error = %err.message(),
                    "policy provider fetch failed, falling back to metadata/config routing preferences"
                );
            }
            Err(err) => {
                return Err(RoutingError::bad_request(format!(
                    "Failed to load routing policy for policy_id '{}': {}",
                    policy_id,
                    err.message()
                )));
            }
        }
    }

    let usage_preferences_str: Option<String> = routing_metadata.and_then(|metadata| {
        metadata
            .get("plano_preference_config")
            .map(|value| value.to_string())
    });
    Ok(usage_preferences_str
        .as_ref()
        .and_then(|s| serde_yaml::from_str(s).ok()))
}

/// Determines the routing decision if
///
/// # Returns
/// * `Ok(RoutingResult)` - Contains the selected model name and span ID
/// * `Err(RoutingError)` - Contains error details and optional span ID
#[allow(clippy::too_many_arguments)]
pub async fn router_chat_get_upstream_model(
    router_service: Arc<RouterService>,
    client_request: ProviderRequestType,
    traceparent: &str,
    request_path: &str,
    request_id: &str,
    inline_usage_preferences: Option<Vec<ModelUsagePreference>>,
    policy_id: Option<String>,
    policy_provider: Option<Arc<PolicyProviderClient>>,
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

    let usage_preferences = resolve_usage_preferences(
        inline_usage_preferences,
        policy_id.as_deref(),
        policy_provider.as_deref(),
        routing_metadata.as_ref(),
    )
    .await?;

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
            Some((route_name, model_name)) => {
                current_span.record("route.selected_model", model_name.as_str());
                Ok(RoutingResult {
                    model_name,
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

#[cfg(test)]
mod tests {
    use super::resolve_usage_preferences;
    use crate::handlers::policy_provider::PolicyProviderClient;
    use crate::state::policy_cache::PolicyCache;
    use common::configuration::{ModelUsagePreference, RoutingPolicyProvider, RoutingPreference};
    use mockito::{Matcher, Server};
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn inline_policy(name: &str) -> Vec<ModelUsagePreference> {
        vec![ModelUsagePreference {
            model: "openai/gpt-4o".to_string(),
            routing_preferences: vec![RoutingPreference {
                name: name.to_string(),
                description: "desc".to_string(),
            }],
        }]
    }

    #[tokio::test]
    async fn resolve_usage_preferences_prioritizes_inline_policy() {
        let inline = inline_policy("inline");
        let mut metadata = HashMap::new();
        metadata.insert(
            "plano_preference_config".to_string(),
            json!(
                [{"model":"openai/gpt-4o-mini","routing_preferences":[{"name":"metadata","description":"desc"}]}]
            ),
        );

        let result = resolve_usage_preferences(
            Some(inline.clone()),
            Some("policy-a"),
            None,
            Some(&metadata),
        )
        .await
        .unwrap();
        assert_eq!(result.unwrap()[0].routing_preferences[0].name, "inline");
    }

    #[tokio::test]
    async fn resolve_usage_preferences_falls_back_to_metadata_on_transient_policy_error() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/policy")
            .match_query(Matcher::Any)
            .with_status(500)
            .create_async()
            .await;

        let provider = PolicyProviderClient::new(
            RoutingPolicyProvider {
                url: format!("{}/policy", server.url()),
                headers: None,
                ttl_seconds: Some(60),
            },
            Arc::new(PolicyCache::new()),
        );
        let mut metadata = HashMap::new();
        metadata.insert(
            "plano_preference_config".to_string(),
            json!(
                [{"model":"openai/gpt-4o-mini","routing_preferences":[{"name":"metadata","description":"desc"}]}]
            ),
        );

        let result =
            resolve_usage_preferences(None, Some("customer-a"), Some(&provider), Some(&metadata))
                .await
                .unwrap()
                .unwrap();

        assert_eq!(result[0].routing_preferences[0].name, "metadata");
    }

    #[tokio::test]
    async fn resolve_usage_preferences_returns_bad_request_on_policy_mismatch() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/policy")
            .match_query(Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"policy_id":"different","routing_preferences":[]}"#)
            .create_async()
            .await;

        let provider = PolicyProviderClient::new(
            RoutingPolicyProvider {
                url: format!("{}/policy", server.url()),
                headers: None,
                ttl_seconds: Some(60),
            },
            Arc::new(PolicyCache::new()),
        );

        let err = resolve_usage_preferences(None, Some("expected"), Some(&provider), None)
            .await
            .unwrap_err();
        assert_eq!(err.status_code, hyper::StatusCode::BAD_REQUEST);
    }
}
