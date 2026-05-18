//! Runtime helpers for handling Agent Skills selected by Plano-Orchestrator.
//!
//! These functions live in `common` (rather than `brightstaff` or a WASM
//! crate) so they can be unit-tested on the native target without dragging
//! in proxy-wasm host-call symbols or tokio runtime dependencies.

use std::collections::{HashMap, HashSet};

use crate::configuration::{SkillRef, TopLevelRoutingPreference};

/// Upper bound on the byte length of a single skill body the runtime will
/// inject into an upstream system prompt. SKILL.md files are typically a few
/// kilobytes; this guard keeps a single oversized or malicious skill from
/// blowing the downstream model's context window. Bodies longer than this
/// are tail-trimmed with a marker line. ~32 KiB ≈ 8K tokens at the
/// 4-bytes-per-token heuristic used elsewhere in brightstaff.
pub const MAX_SKILL_BODY_BYTES: usize = 32 * 1024;

const SKILL_BODY_TRUNCATION_MARKER: &str = "\n…[skill body truncated]\n";

/// Outcome of resolving a list of orchestrator-selected skill names against
/// a route's `skills:` allow-list and the runtime catalog. Callers should
/// emit `warn!` for each name in `dropped_not_allowed` / `dropped_unknown`
/// so misconfigured allow-lists and hallucinated picks are observable.
#[derive(Debug, Default)]
pub struct SkillResolution<'a> {
    /// Skills that survived both the allow-list and catalog filters, in
    /// orchestrator-selected order with duplicates removed.
    pub activated: Vec<&'a SkillRef>,
    /// Names the orchestrator selected that are NOT in the chosen route's
    /// `skills:` allow-list. Typically a cross-route mention — the model
    /// pulled a skill name from the global catalog that this route did not
    /// expose. Callers should `warn!`.
    pub dropped_not_allowed: Vec<String>,
    /// Names that ARE allow-listed for the route but are missing from the
    /// runtime catalog (skill removed / never installed / hallucinated).
    pub dropped_unknown: Vec<String>,
}

/// Build the orchestrator-visible skills catalog from the union of every
/// skill name referenced under `routing_preferences[].skills`. Skills not
/// referenced by any route are excluded — they would just clutter the
/// `<skills>` block with no way for the orchestrator to attach them. The
/// result preserves `catalog` order and is deduplicated by name.
pub fn referenced_skills_catalog(
    catalog: &[SkillRef],
    routes: &HashMap<String, TopLevelRoutingPreference>,
) -> Vec<SkillRef> {
    let mut referenced: HashSet<&str> = HashSet::new();
    for route in routes.values() {
        if let Some(names) = route.skills.as_ref() {
            for name in names {
                referenced.insert(name.as_str());
            }
        }
    }

    let mut out: Vec<SkillRef> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for skill in catalog {
        if referenced.contains(skill.name.as_str()) && seen.insert(skill.name.clone()) {
            out.push(skill.clone());
        }
    }
    out
}

/// Filter `selected` skill names to those that are both (a) allow-listed
/// for the chosen route via `route_allowlist` and (b) present in `catalog`,
/// preserving orchestrator order and deduplicating. Drops are reported on
/// the `SkillResolution` struct so callers can `warn!` and surface
/// misconfiguration without re-walking the lists.
pub fn resolve_for_route<'a>(
    catalog: &'a [SkillRef],
    route_allowlist: &[String],
    selected: &[String],
) -> SkillResolution<'a> {
    let allowed: HashSet<&str> = route_allowlist.iter().map(String::as_str).collect();
    let mut activated: Vec<&SkillRef> = Vec::with_capacity(selected.len());
    let mut taken: HashSet<&str> = HashSet::new();
    let mut dropped_not_allowed: Vec<String> = Vec::new();
    let mut dropped_unknown: Vec<String> = Vec::new();
    for name in selected {
        if !taken.insert(name.as_str()) {
            continue;
        }
        if !allowed.contains(name.as_str()) {
            dropped_not_allowed.push(name.clone());
            continue;
        }
        match catalog.iter().find(|s| &s.name == name) {
            Some(skill) => activated.push(skill),
            None => dropped_unknown.push(name.clone()),
        }
    }
    SkillResolution {
        activated,
        dropped_not_allowed,
        dropped_unknown,
    }
}

/// Resolve a list of orchestrator-selected skill names to their `SkillRef`s
/// directly against the catalog, without any per-route allow-list. Use this
/// for the "skills-only" path documented in `docs/source/resources/skills.rst`
/// where the orchestrator returns skills but no route — the catalog itself
/// (already pre-filtered to skills referenced by SOME route via
/// `referenced_skills_catalog`) is the effective allow-list. Unknown names
/// are dropped silently; results are deduplicated by name preserving order.
pub fn resolve_selected_skills<'a>(
    skills: &'a [SkillRef],
    selected_names: &[String],
) -> Vec<&'a SkillRef> {
    let mut out: Vec<&SkillRef> = Vec::with_capacity(selected_names.len());
    let mut seen: HashSet<&str> = HashSet::new();
    for name in selected_names {
        if !seen.insert(name.as_str()) {
            continue;
        }
        if let Some(skill) = skills.iter().find(|s| &s.name == name) {
            out.push(skill);
        }
    }
    out
}

/// Append the bodies of activated skills to a system prompt, wrapped in
/// `<skill_content name="..." [base_dir="..."]>…</skill_content>` tags so a
/// future context-management pass can recognize and recompact them.
///
/// Behavior contract (relied on by `brightstaff::handlers::llm::model_selection`):
///
/// * Returns `None` only when no base prompt was supplied **and** no skills
///   were activated. Otherwise always returns `Some`.
/// * The base prompt (if any) is kept verbatim and the skill block is
///   appended after a blank line.
/// * Each skill body is tail-trimmed at `MAX_SKILL_BODY_BYTES` bytes (UTF-8
///   boundary safe) with a truncation marker, so a single oversized
///   SKILL.md cannot blow the downstream context window.
/// * `name` and `base_dir` are XML-attribute-escaped (`&`, `<`, `>`, `"`)
///   so a maliciously named skill cannot break out of the wrapper. Skill
///   names are already validated upstream, but defense-in-depth matters
///   here because the wrapper is part of the LLM's system prompt.
pub fn augment_system_prompt_with_skills(
    base_system_prompt: Option<String>,
    activated_skills: &[&SkillRef],
) -> Option<String> {
    if activated_skills.is_empty() {
        return base_system_prompt;
    }
    let mut buf = String::new();
    if let Some(base) = base_system_prompt {
        if !base.is_empty() {
            buf.push_str(&base);
            buf.push('\n');
            buf.push('\n');
        }
    }
    buf.push_str(
        "The following Agent Skills have been activated for this request. \
         Follow their instructions when relevant; resolve relative paths \
         against each skill's base directory.\n\n",
    );
    for skill in activated_skills {
        buf.push_str(&format!(
            "<skill_content name=\"{}\"",
            xml_attr_escape(&skill.name)
        ));
        if let Some(base_dir) = skill.base_dir.as_deref() {
            buf.push_str(&format!(" base_dir=\"{}\"", xml_attr_escape(base_dir)));
        }
        buf.push_str(">\n");
        if let Some(body) = skill.body.as_deref() {
            buf.push_str(&truncate_skill_body(body));
            buf.push('\n');
        } else {
            buf.push_str(&format!(
                "(skill description) {}\n",
                xml_attr_escape(&skill.description)
            ));
        }
        buf.push_str("</skill_content>\n\n");
    }
    Some(buf.trim_end().to_string())
}

/// Escape a string for use inside an XML attribute value (double-quoted).
/// Quotes `&`, `<`, `>`, and `"`; leaves single quotes alone since the
/// wrapper always uses double quotes.
fn xml_attr_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Tail-trim `body` to at most `MAX_SKILL_BODY_BYTES` bytes, respecting
/// UTF-8 character boundaries. Appends a marker so the downstream model
/// can tell content was dropped. Pass-through for short bodies.
fn truncate_skill_body(body: &str) -> String {
    let trimmed = body.trim_end();
    if trimmed.len() <= MAX_SKILL_BODY_BYTES {
        return trimmed.to_string();
    }
    // Reserve room for the marker so the final length is still within the
    // budget even when the marker is added.
    let budget = MAX_SKILL_BODY_BYTES.saturating_sub(SKILL_BODY_TRUNCATION_MARKER.len());
    let mut end = budget;
    while end > 0 && !trimmed.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = String::with_capacity(end + SKILL_BODY_TRUNCATION_MARKER.len());
    out.push_str(&trimmed[..end]);
    out.push_str(SKILL_BODY_TRUNCATION_MARKER);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::SelectionPolicy;

    fn skill(name: &str, body: &str) -> SkillRef {
        SkillRef {
            name: name.to_string(),
            description: format!("desc for {}", name),
            path: Some(format!("/skills/{}/SKILL.md", name)),
            base_dir: Some(format!("/skills/{}", name)),
            body: Some(body.to_string()),
            scope: Some("project".to_string()),
            compatibility: None,
            license: None,
            metadata: None,
            allowed_tools: None,
        }
    }

    fn route(name: &str, skill_names: Option<Vec<&str>>) -> TopLevelRoutingPreference {
        TopLevelRoutingPreference {
            name: name.to_string(),
            description: format!("desc for {}", name),
            models: vec!["openai/gpt-4o".to_string()],
            skills: skill_names.map(|v| v.into_iter().map(String::from).collect()),
            selection_policy: SelectionPolicy::default(),
        }
    }

    fn routes_map(
        routes: Vec<TopLevelRoutingPreference>,
    ) -> HashMap<String, TopLevelRoutingPreference> {
        routes.into_iter().map(|r| (r.name.clone(), r)).collect()
    }

    // --- referenced_skills_catalog ---

    #[test]
    fn referenced_catalog_is_union_across_routes() {
        let catalog = vec![
            skill("pdf", "extract"),
            skill("code-review", "review"),
            skill("never-used", "x"),
        ];
        let routes = routes_map(vec![
            route("docs", Some(vec!["pdf"])),
            route("review", Some(vec!["code-review"])),
            route("other", None),
        ]);
        let out = referenced_skills_catalog(&catalog, &routes);
        let names: Vec<_> = out.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"pdf"));
        assert!(names.contains(&"code-review"));
        assert!(!names.contains(&"never-used"));
    }

    #[test]
    fn referenced_catalog_deduplicates_when_multiple_routes_share_a_skill() {
        let catalog = vec![skill("pdf", "extract")];
        let routes = routes_map(vec![
            route("a", Some(vec!["pdf"])),
            route("b", Some(vec!["pdf"])),
        ]);
        let out = referenced_skills_catalog(&catalog, &routes);
        assert_eq!(out.len(), 1);
    }

    // --- resolve_for_route ---

    #[test]
    fn resolve_for_route_keeps_allowlisted_skills_in_orchestrator_order() {
        let catalog = vec![skill("a", ""), skill("b", ""), skill("c", "")];
        let allow = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let selected = vec!["c".to_string(), "a".to_string()];
        let r = resolve_for_route(&catalog, &allow, &selected);
        let names: Vec<_> = r.activated.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["c", "a"]);
        assert!(r.dropped_not_allowed.is_empty());
        assert!(r.dropped_unknown.is_empty());
    }

    #[test]
    fn resolve_for_route_drops_cross_route_skill_into_not_allowed() {
        let catalog = vec![skill("pdf", ""), skill("payment", "")];
        let allow = vec!["pdf".to_string()]; // route only allows pdf
        let selected = vec!["pdf".to_string(), "payment".to_string()];
        let r = resolve_for_route(&catalog, &allow, &selected);
        assert_eq!(r.activated.len(), 1);
        assert_eq!(r.activated[0].name, "pdf");
        assert_eq!(r.dropped_not_allowed, vec!["payment".to_string()]);
        assert!(r.dropped_unknown.is_empty());
    }

    #[test]
    fn resolve_for_route_drops_hallucinated_skill_into_unknown() {
        let catalog = vec![skill("pdf", "")];
        let allow = vec!["pdf".to_string(), "imaginary".to_string()];
        let selected = vec!["pdf".to_string(), "imaginary".to_string()];
        let r = resolve_for_route(&catalog, &allow, &selected);
        assert_eq!(r.activated.len(), 1);
        assert_eq!(r.activated[0].name, "pdf");
        assert!(r.dropped_not_allowed.is_empty());
        assert_eq!(r.dropped_unknown, vec!["imaginary".to_string()]);
    }

    #[test]
    fn resolve_for_route_deduplicates_repeats() {
        let catalog = vec![skill("pdf", "")];
        let allow = vec!["pdf".to_string()];
        let selected = vec!["pdf".to_string(), "pdf".to_string(), "pdf".to_string()];
        let r = resolve_for_route(&catalog, &allow, &selected);
        assert_eq!(r.activated.len(), 1);
    }

    // --- resolve_selected_skills (skills-only path) ---

    #[test]
    fn resolve_selected_skills_drops_unknown_and_dedupes() {
        let catalog = vec![
            skill("pdf-processing", "extract"),
            skill("code-review", "review"),
        ];
        let names = vec![
            "code-review".to_string(),
            "ghost".to_string(),
            "code-review".to_string(),
            "pdf-processing".to_string(),
        ];
        let resolved = resolve_selected_skills(&catalog, &names);
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].name, "code-review");
        assert_eq!(resolved[1].name, "pdf-processing");
    }

    // --- augment_system_prompt_with_skills ---

    #[test]
    fn augment_passthrough_with_no_skills() {
        let augmented = augment_system_prompt_with_skills(Some("you are helpful".to_string()), &[]);
        assert_eq!(augmented.as_deref(), Some("you are helpful"));
    }

    #[test]
    fn augment_includes_skill_bodies() {
        let s = skill("pdf-processing", "extract text and tables");
        let augmented =
            augment_system_prompt_with_skills(Some("you are helpful".to_string()), &[&s])
                .expect("augmented");
        assert!(augmented.starts_with("you are helpful"));
        assert!(augmented.contains("<skill_content name=\"pdf-processing\""));
        assert!(augmented.contains("extract text and tables"));
        assert!(augmented.contains("base_dir=\"/skills/pdf-processing\""));
    }

    #[test]
    fn augment_without_base_prompt_still_works() {
        let s = skill("code-review", "look at diffs");
        let augmented = augment_system_prompt_with_skills(None, &[&s]).expect("augmented");
        assert!(augmented.contains("<skill_content name=\"code-review\""));
        assert!(augmented.contains("look at diffs"));
    }

    #[test]
    fn augment_xml_escapes_skill_name_and_base_dir() {
        let mut s = skill("safe-name", "body");
        s.name = "bad\"name".to_string();
        s.base_dir = Some("/path/with\"quote".to_string());
        let augmented = augment_system_prompt_with_skills(None, &[&s]).expect("augmented");
        // Raw double-quote must NOT appear inside the attribute value — only
        // its escaped form. Otherwise it would close the attribute and let a
        // skill name inject arbitrary attributes / break out of the wrapper.
        assert!(augmented.contains("name=\"bad&quot;name\""));
        assert!(augmented.contains("base_dir=\"/path/with&quot;quote\""));
    }

    #[test]
    fn augment_truncates_oversized_skill_body() {
        let big_body: String = "a".repeat(MAX_SKILL_BODY_BYTES * 2);
        let s = skill("huge", &big_body);
        let augmented = augment_system_prompt_with_skills(None, &[&s]).expect("augmented");
        // Truncation marker is present, so the body did NOT pass through verbatim.
        assert!(augmented.contains("[skill body truncated]"));
        // And the body slice cannot be longer than MAX_SKILL_BODY_BYTES + a
        // little wrapper overhead — definitely not 2× the cap.
        let body_section_end = augmented.find("</skill_content>").unwrap();
        let body_section_start = augmented.find(">\n").unwrap() + 2;
        let body_len = body_section_end - body_section_start;
        assert!(body_len <= MAX_SKILL_BODY_BYTES + 64);
    }
}
