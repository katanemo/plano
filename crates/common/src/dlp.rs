use regex::Regex;
use serde::{Deserialize, Serialize};

/// Action to take when a DLP pattern matches
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DlpAction {
    Block,
    Redact,
}

/// Configuration for a single DLP pattern
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DlpPatternConfig {
    pub name: String,
    pub pattern: String,
    pub action: DlpAction,
}

/// Top-level DLP configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DlpConfig {
    pub enabled: bool,
    #[serde(default)]
    pub scan_responses: bool,
    #[serde(default = "default_patterns")]
    pub patterns: Vec<DlpPatternConfig>,
}

fn default_patterns() -> Vec<DlpPatternConfig> {
    vec![
        DlpPatternConfig {
            name: "ssn".to_string(),
            pattern: r"\b\d{3}-\d{2}-\d{4}\b".to_string(),
            action: DlpAction::Block,
        },
        DlpPatternConfig {
            name: "email".to_string(),
            pattern: r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b".to_string(),
            action: DlpAction::Redact,
        },
        DlpPatternConfig {
            name: "credit_card".to_string(),
            pattern: r"\b4[0-9]{12}(?:[0-9]{3})?\b".to_string(),
            action: DlpAction::Redact,
        },
        DlpPatternConfig {
            name: "api_key".to_string(),
            pattern: r"sk-[a-zA-Z0-9]{20,}".to_string(),
            action: DlpAction::Redact,
        },
    ]
}

/// A compiled DLP pattern ready for scanning
struct DlpPattern {
    name: String,
    regex: Regex,
    action: DlpAction,
}

/// Result of a DLP scan
pub struct DlpScanResult {
    /// Names of patterns that triggered a block
    pub blocked: Vec<String>,
    /// The body with redactions applied (if any)
    pub redacted_body: String,
    /// Whether any redactions were made
    pub was_redacted: bool,
}

/// DLP scanner that checks text against configured patterns
pub struct DlpScanner {
    patterns: Vec<DlpPattern>,
    enabled: bool,
    pub scan_responses: bool,
}

impl DlpScanner {
    /// Create a new DLP scanner from configuration.
    /// Compiles all regex patterns once at construction time.
    pub fn new(config: &DlpConfig) -> Self {
        let patterns = config
            .patterns
            .iter()
            .filter_map(|p| match Regex::new(&p.pattern) {
                Ok(regex) => Some(DlpPattern {
                    name: p.name.clone(),
                    regex,
                    action: p.action.clone(),
                }),
                Err(e) => {
                    log::warn!("DLP pattern '{}' failed to compile: {}", p.name, e);
                    None
                }
            })
            .collect();

        Self {
            patterns,
            enabled: config.enabled,
            scan_responses: config.scan_responses,
        }
    }

    /// Scan text and return results.
    /// - Block patterns: if any match, the request should be rejected
    /// - Redact patterns: matches are replaced with `[REDACTED:<name>]`
    pub fn scan_and_redact(&self, body: &str) -> DlpScanResult {
        if !self.enabled {
            return DlpScanResult {
                blocked: vec![],
                redacted_body: body.to_string(),
                was_redacted: false,
            };
        }

        let mut blocked = Vec::new();
        let mut redacted = body.to_string();
        let mut was_redacted = false;

        for pattern in &self.patterns {
            if pattern.regex.is_match(&redacted) {
                match pattern.action {
                    DlpAction::Block => {
                        blocked.push(pattern.name.clone());
                    }
                    DlpAction::Redact => {
                        let replacement = format!("[REDACTED:{}]", pattern.name);
                        let new_text = pattern.regex.replace_all(&redacted, &replacement);
                        if new_text != redacted {
                            was_redacted = true;
                            redacted = new_text.into_owned();
                        }
                    }
                }
            }
        }

        DlpScanResult {
            blocked,
            redacted_body: redacted,
            was_redacted,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> DlpConfig {
        DlpConfig {
            enabled: true,
            scan_responses: false,
            patterns: default_patterns(),
        }
    }

    #[test]
    fn test_ssn_blocked() {
        let scanner = DlpScanner::new(&test_config());
        let result = scanner.scan_and_redact("My SSN is 123-45-6789");
        assert_eq!(result.blocked, vec!["ssn"]);
    }

    #[test]
    fn test_email_redacted() {
        let scanner = DlpScanner::new(&test_config());
        let result = scanner.scan_and_redact("Contact me at user@example.com please");
        assert!(result.was_redacted);
        assert!(result.redacted_body.contains("[REDACTED:email]"));
        assert!(!result.redacted_body.contains("user@example.com"));
    }

    #[test]
    fn test_credit_card_redacted() {
        let scanner = DlpScanner::new(&test_config());
        let result = scanner.scan_and_redact("Card: 4111111111111111");
        assert!(result.was_redacted);
        assert!(result.redacted_body.contains("[REDACTED:credit_card]"));
    }

    #[test]
    fn test_api_key_redacted() {
        let scanner = DlpScanner::new(&test_config());
        let result = scanner.scan_and_redact("Key: sk-abcdefghijklmnopqrstuvwxyz");
        assert!(result.was_redacted);
        assert!(result.redacted_body.contains("[REDACTED:api_key]"));
    }

    #[test]
    fn test_clean_text() {
        let scanner = DlpScanner::new(&test_config());
        let result = scanner.scan_and_redact("Hello, how are you?");
        assert!(result.blocked.is_empty());
        assert!(!result.was_redacted);
        assert_eq!(result.redacted_body, "Hello, how are you?");
    }

    #[test]
    fn test_disabled_scanner() {
        let mut config = test_config();
        config.enabled = false;
        let scanner = DlpScanner::new(&config);
        let result = scanner.scan_and_redact("My SSN is 123-45-6789");
        assert!(result.blocked.is_empty());
        assert!(!result.was_redacted);
    }
}
