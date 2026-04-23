use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use hyper::{Response, StatusCode};

use super::full;

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
