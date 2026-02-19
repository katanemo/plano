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

    #[error("Internal server error: {0}")]
    InternalServerError(String),

    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    #[error("{message}")]
    ForwardedError {
        status_code: StatusCode,
        message: String,
    },
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
