//! Mock OTEL Collector for testing trace output
//!
//! This module provides a simple HTTP server that mimics an OTEL collector.
//! It exposes three endpoints:
//! - POST /v1/traces: Capture incoming OTLP JSON payloads
//! - GET /v1/traces: Return all captured payloads as JSON array
//! - DELETE /v1/traces: Clear all captured payloads
//!
//! Each test creates its own MockOtelCollector instance.

use axum::{
    extract::State,
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock;

type SharedTraces = Arc<RwLock<Vec<Value>>>;

/// POST /v1/traces - capture incoming OTLP payload
async fn post_traces(State(traces): State<SharedTraces>, Json(payload): Json<Value>) -> StatusCode {
    traces.write().await.push(payload);
    StatusCode::OK
}

/// GET /v1/traces - return all captured payloads
async fn get_traces(State(traces): State<SharedTraces>) -> Json<Vec<Value>> {
    Json(traces.read().await.clone())
}

/// DELETE /v1/traces - clear all captured payloads
async fn delete_traces(State(traces): State<SharedTraces>) -> StatusCode {
    traces.write().await.clear();
    StatusCode::NO_CONTENT
}

/// Mock OTEL collector server
pub struct MockOtelCollector {
    address: String,
    client: reqwest::Client,
    #[allow(dead_code)]
    server_handle: tokio::task::JoinHandle<()>,
}

impl MockOtelCollector {
    /// Create and start a new mock collector on a random port
    pub async fn start() -> Self {
        let traces = Arc::new(RwLock::new(Vec::new()));

        let app = Router::new()
            .route("/v1/traces", post(post_traces))
            .route("/v1/traces", get(get_traces))
            .route("/v1/traces", delete(delete_traces))
            .with_state(traces.clone());

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("Failed to bind to random port");

        let addr = listener.local_addr().expect("Failed to get local address");
        let address = format!("http://127.0.0.1:{}", addr.port());

        let server_handle = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("Server failed");
        });

        // Give server a moment to start
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        Self {
            address,
            client: reqwest::Client::new(),
            server_handle,
        }
    }

    /// Get the address of the collector
    pub fn address(&self) -> &str {
        &self.address
    }

    /// GET /v1/traces - fetch all captured payloads
    pub async fn get_traces(&self) -> Vec<Value> {
        self.client
            .get(format!("{}/v1/traces", self.address))
            .send()
            .await
            .expect("Failed to GET traces")
            .json()
            .await
            .expect("Failed to parse traces JSON")
    }
}
