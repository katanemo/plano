use bytes::Bytes;
use common::llm_providers::LlmProviders;
use http_body_util::combinators::BoxBody;
use hyper::{Response, StatusCode};
use std::sync::Arc;

use super::full;

pub async fn list_models(
    llm_providers: Arc<tokio::sync::RwLock<LlmProviders>>,
) -> Response<BoxBody<Bytes, hyper::Error>> {
    let prov = llm_providers.read().await;
    let models = prov.to_models();

    match serde_json::to_string(&models) {
        Ok(json) => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(full(json))
            .unwrap(),
        Err(_) => Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .header("Content-Type", "application/json")
            .body(full("{\"error\":\"Failed to serialize models\"}"))
            .unwrap(),
    }
}
