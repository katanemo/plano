use hermesllm::apis::openai::ChatCompletionsResponse;
use hyper::header;
use thiserror::Error;
use tracing::warn;

#[derive(Debug, Error)]
pub enum HttpError {
    #[error("Failed to send request: {0}")]
    Request(#[from] reqwest::Error),

    #[error("Failed to parse JSON response: {0}")]
    Json(serde_json::Error, String),
}

/// Sends a POST request to the given URL and extracts the text content
/// from the first choice of the `ChatCompletionsResponse`.
///
/// Returns `Some((content, elapsed))` on success, or `None` if the response
/// had no choices or the first choice had no content.
pub async fn post_and_extract_content(
    client: &reqwest::Client,
    url: &str,
    headers: header::HeaderMap,
    body: String,
) -> Result<Option<(String, std::time::Duration)>, HttpError> {
    let start_time = std::time::Instant::now();

    let res = client.post(url).headers(headers).body(body).send().await?;

    let body = res.text().await?;
    let elapsed = start_time.elapsed();

    let response: ChatCompletionsResponse = serde_json::from_str(&body).map_err(|err| {
        warn!(error = %err, body = %body, "failed to parse json response");
        HttpError::Json(err, format!("Failed to parse JSON: {}", body))
    })?;

    if response.choices.is_empty() {
        warn!(body = %body, "no choices in response");
        return Ok(None);
    }

    Ok(response.choices[0]
        .message
        .content
        .as_ref()
        .map(|c| (c.clone(), elapsed)))
}
