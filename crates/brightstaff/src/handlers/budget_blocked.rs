use crate::billing::budget_checker::BudgetChecker;
use bytes::Bytes;
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::{Response, StatusCode};

/// Handle GET /budget/blocked
/// Returns the list of project IDs that have exceeded their spending limits.
/// Polled by WASM filters to enforce soft spending limits.
pub async fn handle_budget_blocked(
    checker: &BudgetChecker,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let blocked = checker.get_blocked_projects();
    let blocked_strings: Vec<String> = blocked.iter().map(|id| id.to_string()).collect();

    let body = serde_json::json!({ "blocked": blocked_strings });
    let body_bytes = serde_json::to_vec(&body).unwrap_or_default();

    let mut response = Response::new(
        Full::new(Bytes::from(body_bytes))
            .map_err(|never| match never {})
            .boxed(),
    );
    *response.status_mut() = StatusCode::OK;
    response
        .headers_mut()
        .insert("content-type", "application/json".parse().unwrap());

    Ok(response)
}
