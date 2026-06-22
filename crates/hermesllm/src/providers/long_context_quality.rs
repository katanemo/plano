//! Internal long-context-quality (LCQ) dataset — the v1 Tier 2 routing signal.
//!
//! LCQ scores estimate how well a model stays coherent at long context lengths.
//! They are **Plano-internal** data seeded from public benchmarks (RULER, HELMET,
//! LongBench v2, NoLiMa), loaded from a vendored YAML file. They are intentionally
//! kept separate from the capability catalog: capabilities are stable, LCQ drifts.
//!
//! Used by brightstaff's `rank_models` when `selection_policy.prefer = long_context_quality`.

use crate::providers::capabilities::canonical_model_key;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;

static LONG_CONTEXT_QUALITY_YAML: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/bin/long_context_quality.yaml"
));

/// One model's LCQ entry, carrying provenance.
#[derive(Debug, Clone, Deserialize)]
pub struct LcqEntry {
    pub score: f64,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub dated: String,
}

#[derive(Debug, Clone, Deserialize)]
struct LcqFile {
    #[serde(default)]
    version: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    dated: String,
    #[serde(default)]
    models: HashMap<String, LcqEntry>,
}

/// The internal LCQ dataset, parsed once.
#[derive(Debug, Clone)]
pub struct LongContextQualityDataset {
    pub version: String,
    pub source: String,
    /// Provenance date of the dataset (used for staleness telemetry).
    pub dated: String,
    models: HashMap<String, LcqEntry>,
}

impl LongContextQualityDataset {
    fn parse(yaml: &str) -> Self {
        let file: LcqFile = serde_yaml::from_str(yaml).expect("parse long_context_quality.yaml");
        LongContextQualityDataset {
            version: file.version,
            source: file.source,
            dated: file.dated,
            models: file.models,
        }
    }

    /// Score for a `"<provider>/<model_id>"` string, normalizing the provider
    /// token so config aliases resolve. Returns `None` when not benchmarked.
    pub fn score_for(&self, model: &str) -> Option<f64> {
        // Try the canonical key first, then the raw string as a fallback.
        if let Some(key) = canonical_model_key(model) {
            if let Some(entry) = self.models.get(&key) {
                return Some(entry.score);
            }
        }
        self.models.get(model).map(|e| e.score)
    }

    pub fn len(&self) -> usize {
        self.models.len()
    }

    pub fn is_empty(&self) -> bool {
        self.models.is_empty()
    }
}

/// The vendored LCQ dataset, parsed once and shared.
pub fn dataset() -> &'static LongContextQualityDataset {
    static DATA: OnceLock<LongContextQualityDataset> = OnceLock::new();
    DATA.get_or_init(|| LongContextQualityDataset::parse(LONG_CONTEXT_QUALITY_YAML))
}

/// Convenience: LCQ score for a model from the vendored dataset.
pub fn score_for(model: &str) -> Option<f64> {
    dataset().score_for(model)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dataset_loads_with_provenance() {
        let ds = dataset();
        assert!(!ds.is_empty());
        assert!(
            !ds.dated.is_empty(),
            "dataset should carry a provenance date"
        );
        assert!(!ds.source.is_empty());
    }

    #[test]
    fn scores_resolve_with_alias_normalization() {
        // google/ alias normalizes to canonical gemini/
        let via_alias = score_for("google/gemini-2.5-pro");
        let via_canonical = score_for("gemini/gemini-2.5-pro");
        assert!(via_alias.is_some());
        assert_eq!(via_alias, via_canonical);
    }

    #[test]
    fn unknown_model_has_no_score() {
        assert!(score_for("openai/no-such-model").is_none());
    }
}
