//! Provider implementations for different LLM APIs
//!
//! This module contains provider-specific implementations that handle
//! request/response conversion for different LLM service APIs.
//!
pub mod capabilities;
pub mod id;
pub mod long_context_quality;
pub mod request;
pub mod response;
pub mod streaming_response;

pub use capabilities::{
    CapabilitiesCatalog, CapabilitiesSnapshot, ModelCapabilities, RequiredCapabilities,
};
pub use id::ProviderId;
pub use long_context_quality::{
    score_for as long_context_quality_score, LongContextQualityDataset,
};
pub use request::{ProviderRequest, ProviderRequestError, ProviderRequestType};
pub use response::{ProviderResponse, ProviderResponseType, TokenUsage};
pub use streaming_response::{ProviderStreamResponse, ProviderStreamResponseType};
