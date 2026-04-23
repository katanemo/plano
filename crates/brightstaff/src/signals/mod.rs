//! Plano signals: behavioral quality indicators for agent interactions.
//!
//! This is a Rust port of the paper-aligned Python reference implementation at
//! `https://github.com/katanemo/signals` (or `/Users/shashmi/repos/signals`).
//!
//! Three layers of signals are detected from a conversation transcript:
//!
//! - **Interaction**: misalignment, stagnation, disengagement, satisfaction
//! - **Execution**: failure, loops
//! - **Environment**: exhaustion
//!
//! See `SignalType` for the full hierarchy.

pub mod analyzer;
pub mod environment;
pub mod execution;
pub mod interaction;
pub mod otel;
pub mod schemas;
pub mod text_processing;

pub use analyzer::{SignalAnalyzer, FLAG_MARKER};
pub use schemas::{
    EnvironmentSignals, ExecutionSignals, InteractionQuality, InteractionSignals, SignalGroup,
    SignalInstance, SignalLayer, SignalReport, SignalType, TurnMetrics,
};
