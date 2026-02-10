use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use hyper::Response;
use serde_json::json;
use tracing::{info, warn};

use super::response::ResponseHandler;

/// Build a JSON error response from an `AgentFilterChainError`, logging the
/// full error chain along the way.
///
/// Returns `Ok(Response)` so it can be used directly as a handler return value.
pub fn build_error_chain_response<E: std::error::Error>(
    err: &E,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let mut error_chain = Vec::new();
    let mut current: &dyn std::error::Error = err;
    loop {
        error_chain.push(current.to_string());
        match current.source() {
            Some(source) => current = source,
            None => break,
        }
    }

    warn!(error_chain = ?error_chain, "agent chat error chain");
    warn!(root_error = ?err, "root error");

    let error_json = json!({
        "error": {
            "type": "AgentFilterChainError",
            "message": err.to_string(),
            "error_chain": error_chain,
            "debug_info": format!("{:?}", err)
        }
    });

    info!(error = %error_json, "structured error info");

    Ok(ResponseHandler::create_json_error_response(&error_json))
}
