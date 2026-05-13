//! Runtime helpers for handling Agent Skills selected by Plano-Orchestrator.
//!
//! These functions live in `common` (rather than `brightstaff` or a WASM
//! crate) so they can be unit-tested on the native target without dragging
//! in proxy-wasm host-call symbols or tokio runtime dependencies.

use crate::configuration::{SkillRef, TopLevelRoutingPreference};

/// Filter `skills` down to the subset attached to `route_name` via
/// `routing_preferences[].skills`. When the selected route has no `skills:`
/// list, returns an empty vector — skills are scoped to routes, not global.
///
/// `routing_preferences` is the source of truth for which skills are even
/// eligible for the orchestrator to activate on a given route.
pub fn skills_for_route<'a>(
    skills: &'a [SkillRef],
    routing_preferences: &[TopLevelRoutingPreference],
    route_name: &str,
) -> Vec<&'a SkillRef> {
    let Some(route) = routing_preferences.iter().find(|p| p.name == route_name) else {
        return Vec::new();
    };
    let Some(allow) = route.skills.as_ref() else {
        return Vec::new();
    };
    let mut out: Vec<&SkillRef> = Vec::with_capacity(allow.len());
    for name in allow {
        if let Some(skill) = skills.iter().find(|s| &s.name == name) {
            out.push(skill);
        }
    }
    out
}

/// Resolve a list of orchestrator-selected skill names to their `SkillRef`s.
/// Unknown names are dropped silently — the orchestrator can hallucinate.
/// Results are deduplicated by name, preserving the order Plano-Orchestrator
/// returned.
pub fn resolve_selected_skills<'a>(
    skills: &'a [SkillRef],
    selected_names: &[String],
) -> Vec<&'a SkillRef> {
    let mut out: Vec<&SkillRef> = Vec::with_capacity(selected_names.len());
    for name in selected_names {
        if out.iter().any(|s| &s.name == name) {
            continue;
        }
        if let Some(skill) = skills.iter().find(|s| &s.name == name) {
            out.push(skill);
        }
    }
    out
}

/// Append the bodies of activated skills to a system prompt, wrapped in
/// `<skill_content name="...">` tags so a future context-management pass can
/// recognize and recompact them.
///
/// Returns `None` only if no base system prompt was supplied and no skills
/// were activated. When skills are present the wrapper text always appears so
/// the downstream model receives a clear, well-structured instruction block.
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
        buf.push_str(&format!("<skill_content name=\"{}\"", skill.name));
        if let Some(base_dir) = skill.base_dir.as_deref() {
            buf.push_str(&format!(" base_dir=\"{}\"", base_dir));
        }
        buf.push_str(">\n");
        if let Some(body) = skill.body.as_deref() {
            buf.push_str(body.trim_end());
            buf.push('\n');
        } else {
            buf.push_str(&format!("(skill description) {}\n", skill.description));
        }
        buf.push_str("</skill_content>\n\n");
    }
    Some(buf.trim_end().to_string())
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

    #[test]
    fn skills_for_route_returns_attached_skills() {
        let catalog = vec![
            skill("pdf-processing", "extract"),
            skill("code-review", "review"),
        ];
        let routes = vec![
            route("code review", Some(vec!["code-review"])),
            route("doc work", Some(vec!["pdf-processing"])),
        ];
        let resolved = skills_for_route(&catalog, &routes, "code review");
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].name, "code-review");
    }

    #[test]
    fn skills_for_route_empty_when_route_has_no_skills_list() {
        let catalog = vec![skill("pdf-processing", "extract")];
        let routes = vec![route("code review", None)];
        let resolved = skills_for_route(&catalog, &routes, "code review");
        assert!(resolved.is_empty());
    }

    #[test]
    fn skills_for_route_empty_when_route_missing() {
        let catalog = vec![skill("pdf-processing", "extract")];
        let routes = vec![route("code review", Some(vec!["pdf-processing"]))];
        let resolved = skills_for_route(&catalog, &routes, "no-such-route");
        assert!(resolved.is_empty());
    }

    #[test]
    fn skills_for_route_drops_unknown_skill_names() {
        let catalog = vec![skill("pdf-processing", "extract")];
        let routes = vec![route(
            "code review",
            Some(vec!["pdf-processing", "ghost-skill"]),
        )];
        let resolved = skills_for_route(&catalog, &routes, "code review");
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].name, "pdf-processing");
    }

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
}
