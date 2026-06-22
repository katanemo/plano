//! OpenAI multimodal output APIs: image generation (`/v1/images/generations`)
//! and text-to-speech (`/v1/audio/speech`).
//!
//! These are OpenAI-native serde shapes. Image responses are JSON; audio/speech
//! responses are **binary** and are passed through untouched by the gateway
//! (see `llm_gateway::stream_context`), so there is no audio *response* struct.

use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::providers::request::ProviderRequestError;

/// Request body for `POST /v1/images/generations`.
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct ImagesRequest {
    #[serde(default)]
    pub model: String,
    pub prompt: String,
    /// Number of images to generate.
    pub n: Option<u32>,
    /// e.g. `1024x1024`, `1792x1024`.
    pub size: Option<String>,
    /// e.g. `standard`, `hd`, or model-specific quality levels.
    pub quality: Option<String>,
    /// `url` or `b64_json`.
    pub response_format: Option<String>,
    pub style: Option<String>,
    pub background: Option<String>,
    pub output_format: Option<String>,
    pub user: Option<String>,
}

impl ImagesRequest {
    pub fn try_from_bytes(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn set_model(&mut self, model: String) {
        self.model = model;
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, ProviderRequestError> {
        serde_json::to_vec(self).map_err(|e| ProviderRequestError {
            message: format!("failed to serialize ImagesRequest: {}", e),
            source: Some(Box::new(e)),
        })
    }
}

/// One generated image in an [`ImagesResponse`].
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct ImageData {
    pub b64_json: Option<String>,
    pub url: Option<String>,
    pub revised_prompt: Option<String>,
}

/// Response body for `POST /v1/images/generations`.
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct ImagesResponse {
    pub created: Option<u64>,
    #[serde(default)]
    pub data: Vec<ImageData>,
    /// Some providers report token/image usage here.
    pub usage: Option<ImagesUsage>,
}

/// Usage block for image generation (used for per-image cost accounting).
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct ImagesUsage {
    /// Number of images produced (the primary billable unit).
    pub images: Option<u32>,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub total_tokens: Option<u32>,
}

impl ImagesResponse {
    pub fn try_from_bytes(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }

    /// Number of images produced (billable unit), preferring an explicit usage
    /// count and falling back to the number of returned images.
    pub fn image_units(&self) -> usize {
        self.usage
            .as_ref()
            .and_then(|u| u.images)
            .map(|n| n as usize)
            .unwrap_or(self.data.len())
    }
}

/// Request body for `POST /v1/audio/speech` (text-to-speech). The response is
/// binary audio and is streamed/passed through without a typed response body.
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct AudioSpeechRequest {
    #[serde(default)]
    pub model: String,
    /// Text to synthesize.
    pub input: String,
    /// Voice id (e.g. `alloy`, `verse`).
    pub voice: String,
    /// Output container, e.g. `mp3`, `wav`, `opus`, `pcm`.
    pub response_format: Option<String>,
    pub speed: Option<f32>,
    pub instructions: Option<String>,
    /// Whether the client requested streamed audio chunks.
    pub stream: Option<bool>,
}

impl AudioSpeechRequest {
    pub fn try_from_bytes(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn set_model(&mut self, model: String) {
        self.model = model;
    }

    pub fn is_streaming(&self) -> bool {
        self.stream.unwrap_or(false)
    }

    /// Billable unit for TTS: number of input characters.
    pub fn audio_units(&self) -> usize {
        self.input.chars().count()
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, ProviderRequestError> {
        serde_json::to_vec(self).map_err(|e| ProviderRequestError {
            message: format!("failed to serialize AudioSpeechRequest: {}", e),
            source: Some(Box::new(e)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_request_roundtrips() {
        let raw = br#"{"model":"gpt-image-1","prompt":"a cat","n":2,"size":"1024x1024"}"#;
        let req = ImagesRequest::try_from_bytes(raw).unwrap();
        assert_eq!(req.model, "gpt-image-1");
        assert_eq!(req.n, Some(2));
        assert!(req.to_bytes().is_ok());
    }

    #[test]
    fn images_response_counts_units() {
        let raw = br#"{"created":1,"data":[{"b64_json":"aaa"},{"b64_json":"bbb"}]}"#;
        let resp = ImagesResponse::try_from_bytes(raw).unwrap();
        assert_eq!(resp.image_units(), 2);

        let raw2 = br#"{"created":1,"data":[{"url":"x"}],"usage":{"images":5}}"#;
        let resp2 = ImagesResponse::try_from_bytes(raw2).unwrap();
        assert_eq!(resp2.image_units(), 5);
    }

    #[test]
    fn audio_speech_request_units() {
        let raw = br#"{"model":"gpt-4o-mini-tts","input":"hello","voice":"alloy"}"#;
        let req = AudioSpeechRequest::try_from_bytes(raw).unwrap();
        assert_eq!(req.audio_units(), 5);
        assert_eq!(req.voice, "alloy");
        assert!(!req.is_streaming());
    }
}
