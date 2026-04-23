use hermesllm::apis::openai::ChatCompletionsResponse;
use hyper::header;
use serde::Deserialize;
use thiserror::Error;
use tracing::warn;

/// Max bytes of raw upstream body we include in a log message or error text
/// when the body is not a recognizable error envelope. Keeps logs from being
/// flooded by huge HTML error pages.
const RAW_BODY_LOG_LIMIT: usize = 512;

#[derive(Debug, Error)]
pub enum HttpError {
    #[error("Failed to send request: {0}")]
    Request(#[from] reqwest::Error),

    #[error("Failed to parse JSON response: {0}")]
    Json(serde_json::Error, String),

    #[error("Upstream returned {status}: {message}")]
    Upstream { status: u16, message: String },
}

/// Shape of an OpenAI-style error response body, e.g.
/// `{"error": {"message": "...", "type": "...", "param": "...", "code": ...}}`.
#[derive(Debug, Deserialize)]
struct UpstreamErrorEnvelope {
    error: UpstreamErrorBody,
}

#[derive(Debug, Deserialize)]
struct UpstreamErrorBody {
    message: String,
    #[serde(default, rename = "type")]
    err_type: Option<String>,
    #[serde(default)]
    param: Option<String>,
}

/// Extract a human-readable error message from an upstream response body.
/// Tries to parse an OpenAI-style `{"error": {"message": ...}}` envelope; if
/// that fails, falls back to the first `RAW_BODY_LOG_LIMIT` bytes of the raw
/// body (UTF-8 safe).
fn extract_upstream_error_message(body: &str) -> String {
    if let Ok(env) = serde_json::from_str::<UpstreamErrorEnvelope>(body) {
        let mut msg = env.error.message;
        if let Some(param) = env.error.param {
            msg.push_str(&format!(" (param={param})"));
        }
        if let Some(err_type) = env.error.err_type {
            msg.push_str(&format!(" [type={err_type}]"));
        }
        return msg;
    }
    truncate_for_log(body).to_string()
}

fn truncate_for_log(s: &str) -> &str {
    if s.len() <= RAW_BODY_LOG_LIMIT {
        return s;
    }
    let mut end = RAW_BODY_LOG_LIMIT;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Sends a POST request to the given URL and extracts the text content
/// from the first choice of the `ChatCompletionsResponse`.
///
/// Returns `Some((content, elapsed))` on success, `None` if the response
/// had no choices or the first choice had no content. Returns
/// `HttpError::Upstream` for any non-2xx status, carrying a message
/// extracted from the OpenAI-style error envelope (or a truncated raw body
/// if the body is not in that shape).
pub async fn post_and_extract_content(
    client: &reqwest::Client,
    url: &str,
    headers: header::HeaderMap,
    body: String,
) -> Result<Option<(String, std::time::Duration)>, HttpError> {
    let start_time = std::time::Instant::now();

    let res = client.post(url).headers(headers).body(body).send().await?;
    let status = res.status();

    let body = res.text().await?;
    let elapsed = start_time.elapsed();

    if !status.is_success() {
        let message = extract_upstream_error_message(&body);
        warn!(
            status = status.as_u16(),
            message = %message,
            body_size = body.len(),
            "upstream returned error response"
        );
        return Err(HttpError::Upstream {
            status: status.as_u16(),
            message,
        });
    }

    let response: ChatCompletionsResponse = serde_json::from_str(&body).map_err(|err| {
        warn!(
            error = %err,
            body = %truncate_for_log(&body),
            "failed to parse json response",
        );
        HttpError::Json(err, format!("Failed to parse JSON: {}", body))
    })?;

    if response.choices.is_empty() {
        warn!(body = %truncate_for_log(&body), "no choices in response");
        return Ok(None);
    }

    Ok(response.choices[0]
        .message
        .content
        .as_ref()
        .map(|c| (c.clone(), elapsed)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_message_from_openai_style_error_envelope() {
        let body = r#"{"error":{"code":400,"message":"This model's maximum context length is 32768 tokens. However, you requested 0 output tokens and your prompt contains at least 32769 input tokens, for a total of at least 32769 tokens.","param":"input_tokens","type":"BadRequestError"}}"#;
        let msg = extract_upstream_error_message(body);
        assert!(
            msg.starts_with("This model's maximum context length is 32768 tokens."),
            "unexpected message: {msg}"
        );
        assert!(msg.contains("(param=input_tokens)"));
        assert!(msg.contains("[type=BadRequestError]"));
    }

    #[test]
    fn extracts_message_without_optional_fields() {
        let body = r#"{"error":{"message":"something broke"}}"#;
        let msg = extract_upstream_error_message(body);
        assert_eq!(msg, "something broke");
    }

    #[test]
    fn falls_back_to_raw_body_when_not_error_envelope() {
        let body = "<html><body>502 Bad Gateway</body></html>";
        let msg = extract_upstream_error_message(body);
        assert_eq!(msg, body);
    }

    #[test]
    fn truncates_non_envelope_bodies_in_logs() {
        let body = "x".repeat(RAW_BODY_LOG_LIMIT * 3);
        let msg = extract_upstream_error_message(&body);
        assert_eq!(msg.len(), RAW_BODY_LOG_LIMIT);
    }

    #[test]
    fn truncate_for_log_respects_utf8_boundaries() {
        // 2-byte characters; picking a length that would split mid-char.
        let body = "é".repeat(RAW_BODY_LOG_LIMIT);
        let out = truncate_for_log(&body);
        // Should be a valid &str (implicit — would panic if we returned
        // a non-boundary slice) and at most RAW_BODY_LOG_LIMIT bytes.
        assert!(out.len() <= RAW_BODY_LOG_LIMIT);
        assert!(out.chars().all(|c| c == 'é'));
    }
}
