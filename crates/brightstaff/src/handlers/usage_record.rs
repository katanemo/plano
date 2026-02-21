use crate::billing::counters::SpendingCounters;
use crate::billing::flusher::UsageEvent;
use crate::pricing::PricingRegistry;
use bytes::Bytes;
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::body::Incoming;
use hyper::{Request, Response, StatusCode};
use serde::Deserialize;
use tokio::sync::mpsc;
use tracing::{debug, warn};
use uuid::Uuid;

/// Legacy usage record request (managed proxy mode - has pipe_id, user_id)
#[derive(Debug, Deserialize)]
pub struct UsageRecordRequest {
    pub user_id: Option<Uuid>,
    pub project_id: Uuid,
    pub pipe_id: Option<Uuid>,
    pub token_id: Option<Uuid>,
    pub provider: String,
    pub model: String,
    pub input_tokens: i32,
    pub output_tokens: i32,
    pub is_streaming: bool,
    pub status_code: Option<i32>,
    pub request_id: Option<String>,
    // Firewall mode: if true, skip pricing on hot path (will be priced async)
    #[serde(default)]
    pub firewall_mode: bool,
    pub api_key_hash: Option<String>,
}

/// Handle POST /usage/record
/// Accepts usage data from WASM callout.
/// - Managed proxy mode: calculates cost, enqueues with cost for batch write
/// - Firewall mode: enqueues with is_priced=false, cost calculated by background PriceCalculator
pub async fn handle_usage_record(
    req: Request<Incoming>,
    pricing: &PricingRegistry,
    counters: &SpendingCounters,
    usage_tx: &mpsc::Sender<UsageEvent>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let body_bytes = req
        .collect()
        .await
        .map(|b| b.to_bytes())
        .unwrap_or_default();

    let record: UsageRecordRequest = match serde_json::from_slice(&body_bytes) {
        Ok(r) => r,
        Err(e) => {
            warn!(error = %e, "invalid usage record request");
            let body = serde_json::json!({ "error": format!("invalid request: {}", e) });
            let mut response = Response::new(full_body_bytes(
                serde_json::to_vec(&body).unwrap_or_default(),
            ));
            *response.status_mut() = StatusCode::BAD_REQUEST;
            return Ok(response);
        }
    };

    let (cost_cents, is_priced) = if record.firewall_mode {
        // Firewall mode: no pricing on hot path
        (0.0, false)
    } else {
        // Managed proxy mode: calculate cost now
        let cost = pricing
            .calculate_cost(
                &record.provider,
                &record.model,
                record.input_tokens,
                record.output_tokens,
            )
            .await;

        // Update in-memory spending counters
        let cost_micro_cents = (cost * 10_000.0) as i64;
        if let Some(user_id) = record.user_id {
            counters.record_usage("user", user_id, "daily", cost_micro_cents);
            counters.record_usage("user", user_id, "monthly", cost_micro_cents);
        }
        counters.record_usage("project", record.project_id, "daily", cost_micro_cents);
        counters.record_usage("project", record.project_id, "monthly", cost_micro_cents);

        (cost, true)
    };

    // Enqueue usage event for batch writing
    let event = UsageEvent {
        user_id: record.user_id,
        project_id: record.project_id,
        pipe_id: record.pipe_id,
        token_id: record.token_id,
        provider: record.provider.clone(),
        model: record.model.clone(),
        input_tokens: record.input_tokens,
        output_tokens: record.output_tokens,
        cost_cents,
        is_streaming: record.is_streaming,
        status_code: record.status_code,
        request_id: record.request_id.clone(),
        is_priced,
    };

    if let Err(e) = usage_tx.send(event).await {
        warn!(error = %e, "failed to enqueue usage event");
    }

    debug!(
        provider = %record.provider,
        model = %record.model,
        input_tokens = record.input_tokens,
        output_tokens = record.output_tokens,
        cost_cents = cost_cents,
        firewall_mode = record.firewall_mode,
        "usage recorded"
    );

    let body = serde_json::json!({ "status": "ok", "cost_cents": cost_cents });
    let mut response = Response::new(full_body_bytes(
        serde_json::to_vec(&body).unwrap_or_default(),
    ));
    *response.status_mut() = StatusCode::OK;
    response
        .headers_mut()
        .insert("content-type", "application/json".parse().unwrap());
    Ok(response)
}

fn full_body_bytes(bytes: Vec<u8>) -> BoxBody<Bytes, hyper::Error> {
    Full::new(Bytes::from(bytes))
        .map_err(|never| match never {})
        .boxed()
}
