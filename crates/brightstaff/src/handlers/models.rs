use bytes::Bytes;
use common::llm_providers::LlmProviders;
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::{Response, StatusCode};
use serde_json;
use std::sync::Arc;

pub async fn list_models(
    llm_providers: Arc<tokio::sync::RwLock<LlmProviders>>,
) -> Response<BoxBody<Bytes, hyper::Error>> {
    let prov = llm_providers.read().await;
    let models = prov.to_models();

    match serde_json::to_string(&models) {
        Ok(json) => {
            let body = Full::new(Bytes::from(json))
                .map_err(|never| match never {})
                .boxed();
            Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json")
                .body(body)
                .unwrap()
        }
        Err(_) => {
            let body = Full::new(Bytes::from_static(
                b"{\"error\":\"Failed to serialize models\"}",
            ))
            .map_err(|never| match never {})
            .boxed();
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header("Content-Type", "application/json")
                .body(body)
                .unwrap()
        }
    }
}
