use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use hyper::{Response, StatusCode};
use std::sync::Arc;

use super::full;
use crate::app_state::AppState;

#[derive(serde::Serialize)]
struct MemStats {
    allocated_bytes: usize,
    resident_bytes: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Returns jemalloc memory statistics as JSON.
/// Falls back to a stub when the jemalloc feature is disabled.
pub async fn memstats() -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let stats = get_jemalloc_stats();
    let json = serde_json::to_string(&stats).unwrap();
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(full(json))
        .unwrap())
}

#[cfg(feature = "jemalloc")]
fn get_jemalloc_stats() -> MemStats {
    use tikv_jemalloc_ctl::{epoch, stats};

    // Advance the jemalloc stats epoch so numbers are fresh.
    if let Err(e) = epoch::advance() {
        return MemStats {
            allocated_bytes: 0,
            resident_bytes: 0,
            error: Some(format!("failed to advance jemalloc epoch: {e}")),
        };
    }

    MemStats {
        allocated_bytes: stats::allocated::read().unwrap_or(0),
        resident_bytes: stats::resident::read().unwrap_or(0),
        error: None,
    }
}

#[cfg(not(feature = "jemalloc"))]
fn get_jemalloc_stats() -> MemStats {
    MemStats {
        allocated_bytes: 0,
        resident_bytes: 0,
        error: Some("jemalloc feature not enabled".to_string()),
    }
}

#[derive(serde::Serialize)]
struct StateSize {
    entry_count: usize,
    estimated_bytes: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Returns the number of entries and estimated byte size in the conversation state store.
pub async fn state_size(
    state: Arc<AppState>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let result = match &state.state_storage {
        Some(storage) => match storage.entry_stats().await {
            Ok((count, bytes)) => StateSize {
                entry_count: count,
                estimated_bytes: bytes,
                error: None,
            },
            Err(e) => StateSize {
                entry_count: 0,
                estimated_bytes: 0,
                error: Some(format!("{e}")),
            },
        },
        None => StateSize {
            entry_count: 0,
            estimated_bytes: 0,
            error: Some("no state_storage configured".to_string()),
        },
    };

    let json = serde_json::to_string(&result).unwrap();
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(full(json))
        .unwrap())
}
