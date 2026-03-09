use bytes::Bytes;
use common::configuration::SpanAttributes;
use common::consts::{REQUEST_ID_HEADER, TRACE_PARENT_HEADER};
use common::errors::BrightStaffError;
use hermesllm::clients::SupportedAPIsFromClient;
use hermesllm::ProviderRequestType;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::{Request, Response, StatusCode};
use std::sync::Arc;
use tracing::{debug, info, info_span, warn, Instrument};

use crate::handlers::router_chat::router_chat_get_upstream_model;
use crate::router::llm_router::RouterService;
use crate::tracing::{collect_custom_trace_attributes, operation_component, set_service_name};

#[derive(serde::Serialize)]
struct RoutingDecisionResponse {
    model: String,
    route: Option<String>,
    trace_id: String,
}

pub async fn routing_decision(
    request: Request<hyper::body::Incoming>,
    router_service: Arc<RouterService>,
    request_path: String,
    span_attributes: Arc<Option<SpanAttributes>>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let request_headers = request.headers().clone();
    let request_id: String = request_headers
        .get(REQUEST_ID_HEADER)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let custom_attrs =
        collect_custom_trace_attributes(&request_headers, span_attributes.as_ref().as_ref());

    let request_span = info_span!(
        "routing_decision",
        component = "routing",
        request_id = %request_id,
        http.method = %request.method(),
        http.path = %request_path,
    );

    routing_decision_inner(
        request,
        router_service,
        request_id,
        request_path,
        request_headers,
        custom_attrs,
    )
    .instrument(request_span)
    .await
}

async fn routing_decision_inner(
    request: Request<hyper::body::Incoming>,
    router_service: Arc<RouterService>,
    request_id: String,
    request_path: String,
    request_headers: hyper::HeaderMap,
    custom_attrs: std::collections::HashMap<String, String>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    set_service_name(operation_component::ROUTING);
    opentelemetry::trace::get_active_span(|span| {
        for (key, value) in &custom_attrs {
            span.set_attribute(opentelemetry::KeyValue::new(key.clone(), value.clone()));
        }
    });

    // Extract or generate traceparent
    let traceparent: String = match request_headers
        .get(TRACE_PARENT_HEADER)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string())
    {
        Some(tp) => tp,
        None => {
            let trace_id = uuid::Uuid::new_v4().to_string().replace("-", "");
            let generated_tp = format!("00-{}-0000000000000000-01", trace_id);
            warn!(
                generated_traceparent = %generated_tp,
                "TRACE_PARENT header missing, generated new traceparent"
            );
            generated_tp
        }
    };

    // Extract trace_id from traceparent (format: 00-{trace_id}-{span_id}-{flags})
    let trace_id = traceparent
        .split('-')
        .nth(1)
        .unwrap_or("unknown")
        .to_string();

    // Parse request body
    let chat_request_bytes = request.collect().await?.to_bytes();

    debug!(
        body = %String::from_utf8_lossy(&chat_request_bytes),
        "routing decision request body received"
    );

    let client_request = match ProviderRequestType::try_from((
        &chat_request_bytes[..],
        &SupportedAPIsFromClient::from_endpoint(request_path.as_str()).unwrap(),
    )) {
        Ok(request) => request,
        Err(err) => {
            warn!(error = %err, "failed to parse request for routing decision");
            return Ok(BrightStaffError::InvalidRequest(format!(
                "Failed to parse request: {}",
                err
            ))
            .into_response());
        }
    };

    // Call the existing routing logic
    let routing_result = router_chat_get_upstream_model(
        router_service,
        client_request,
        &traceparent,
        &request_path,
        &request_id,
    )
    .await;

    match routing_result {
        Ok(result) => {
            let response = RoutingDecisionResponse {
                model: result.model_name,
                route: result.route_name,
                trace_id,
            };

            info!(
                model = %response.model,
                route = ?response.route,
                "routing decision completed"
            );

            let json = serde_json::to_string(&response).unwrap();
            let body = Full::new(Bytes::from(json))
                .map_err(|never| match never {})
                .boxed();

            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json")
                .body(body)
                .unwrap())
        }
        Err(err) => {
            warn!(error = %err.message, "routing decision failed");
            Ok(BrightStaffError::InternalServerError(err.message).into_response())
        }
    }
}
