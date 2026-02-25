use crate::{api::open_ai::ChatCompletionChunkResponseError, ratelimit};
use bytes::Bytes;
use hermesllm::apis::openai::OpenAIError;
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::{Error as HyperError, Response, StatusCode};
use proxy_wasm::types::Status;
use serde_json::json;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ClientError {
    #[error("Error dispatching HTTP call to `{upstream_name}/{path}`, error: {internal_status:?}")]
    DispatchError {
        upstream_name: String,
        path: String,
        internal_status: Status,
    },
}

#[derive(Error, Debug)]
pub enum ServerError {
    #[error(transparent)]
    HttpDispatch(ClientError),
    #[error(transparent)]
    Deserialization(serde_json::Error),
    #[error(transparent)]
    Serialization(serde_json::Error),
    #[error("{0}")]
    LogicError(String),
    #[error("upstream application error host={host}, path={path}, status={status}, body={body}")]
    Upstream {
        host: String,
        path: String,
        status: String,
        body: String,
    },
    #[error("jailbreak detected: {0}")]
    Jailbreak(String),
    #[error("{why}")]
    NoMessagesFound { why: String },
    #[error(transparent)]
    ExceededRatelimit(ratelimit::Error),
    #[error("{why}")]
    BadRequest { why: String },
    #[error("error in streaming response")]
    Streaming(#[from] ChatCompletionChunkResponseError),
    #[error("error parsing openai message: {0}")]
    OpenAIPError(#[from] OpenAIError),
}
// -----------------------------------------------------------------------------
// BrightStaff Errors (Standardized)
// -----------------------------------------------------------------------------
#[derive(Debug, Error)]
pub enum BrightStaffError {
    #[error("The requested model '{0}' does not exist")]
    ModelNotFound(String),

    #[error("No model specified in request and no default provider configured")]
    NoModelSpecified,

    #[error("Conversation state not found for previous_response_id: {0}")]
    ConversationStateNotFound(String),

    #[error("Internal server error")]
    InternalServerError(String),

    #[error("Invalid request")]
    InvalidRequest(String),

    #[error("{message}")]
    ForwardedError {
        status_code: StatusCode,
        message: String,
    },

    #[error("Stream error: {0}")]
    StreamError(String),

    #[error("Failed to create response: {0}")]
    ResponseCreationFailed(#[from] hyper::http::Error),
}

impl BrightStaffError {
    pub fn into_response(self) -> Response<BoxBody<Bytes, HyperError>> {
        let (status, code, details) = match &self {
            BrightStaffError::ModelNotFound(model_name) => (
                StatusCode::NOT_FOUND,
                "ModelNotFound",
                json!({ "rejected_model_id": model_name }),
            ),

            BrightStaffError::NoModelSpecified => {
                (StatusCode::BAD_REQUEST, "NoModelSpecified", json!({}))
            }

            BrightStaffError::ConversationStateNotFound(prev_resp_id) => (
                StatusCode::CONFLICT,
                "ConversationStateNotFound",
                json!({ "previous_response_id": prev_resp_id }),
            ),

            BrightStaffError::InternalServerError(reason) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "InternalServerError",
                // Passing the reason into details for easier debugging
                json!({ "reason": reason }),
            ),

            BrightStaffError::InvalidRequest(reason) => (
                StatusCode::BAD_REQUEST,
                "InvalidRequest",
                json!({ "reason": reason }),
            ),

            BrightStaffError::ForwardedError {
                status_code,
                message,
            } => (*status_code, "ForwardedError", json!({ "reason": message })),

            BrightStaffError::StreamError(reason) => (
                StatusCode::BAD_REQUEST,
                "StreamError",
                json!({ "reason": reason }),
            ),

            BrightStaffError::ResponseCreationFailed(reason) => (
                StatusCode::BAD_REQUEST,
                "ResponseCreationFailed",
                json!({ "reason": reason.to_string() }),
            ),
        };

        let body_json = json!({
            "error": {
                "code": code,
                "message": self.to_string(),
                "details": details
            }
        });

        // 1. Create the concrete body
        let full_body = Full::new(Bytes::from(body_json.to_string()));

        // 2. Convert it to BoxBody
        // We map_err because Full never fails, but BoxBody expects a HyperError
        let boxed_body = full_body
            .map_err(|never| match never {}) // This handles the "Infallible" error type
            .boxed();

        Response::builder()
            .status(status)
            .header("content-type", "application/json")
            .body(boxed_body)
            .unwrap_or_else(|_| {
                Response::new(
                    Full::new(Bytes::from("Internal Error"))
                        .map_err(|never| match never {})
                        .boxed(),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt; // For .collect().await

    #[tokio::test]
    async fn test_model_not_found_format() {
        let err = BrightStaffError::ModelNotFound("gpt-5-secret".to_string());
        let response = err.into_response();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        // Helper to extract body as JSON
        let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

        assert_eq!(body["error"]["code"], "ModelNotFound");
        assert_eq!(
            body["error"]["details"]["rejected_model_id"],
            "gpt-5-secret"
        );
        assert!(body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("gpt-5-secret"));
    }

    #[tokio::test]
    async fn test_forwarded_error_preserves_status() {
        let err = BrightStaffError::ForwardedError {
            status_code: StatusCode::TOO_MANY_REQUESTS,
            message: "Rate limit exceeded on agent side".to_string(),
        };

        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);

        let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

        assert_eq!(body["error"]["code"], "ForwardedError");
    }

    #[tokio::test]
    async fn test_hyper_error_wrapping() {
        // Manually trigger a hyper error by creating an invalid URI/Header
        let hyper_err = hyper::http::Response::builder()
            .status(1000) // Invalid status
            .body(())
            .unwrap_err();

        let err = BrightStaffError::ResponseCreationFailed(hyper_err);
        let response = err.into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
