use proxy_wasm::types::Status;

use crate::{api::open_ai::ChatCompletionChunkResponseError, ratelimit};
use hermesllm::apis::openai::OpenAIError;

#[derive(thiserror::Error, Debug)]
pub enum ClientError {
    #[error("Error dispatching HTTP call to `{upstream_name}/{path}`, error: {internal_status:?}")]
    DispatchError {
        upstream_name: String,
        path: String,
        internal_status: Status,
    },
}

#[derive(thiserror::Error, Debug)]
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

#[derive(thiserror::Error, Debug)]
pub enum AwsError {
    #[error("Failed to get credentials from environment: {0}")]
    CredentialError(String),
    #[error("STS AssumeRole failed: {0}")]
    StsError(String),
    #[error("AWS Signature V4 signing failed: {0}")]
    SigningError(String),
    #[error("Invalid AWS configuration: {0}")]
    ConfigError(String),
}
