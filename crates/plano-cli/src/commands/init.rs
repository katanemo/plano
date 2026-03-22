use std::path::Path;

use anyhow::{bail, Result};

const TEMPLATES: &[(&str, &str, &str)] = &[
    (
        "sub_agent_orchestration",
        "Sub-agent Orchestration",
        include_str!("../../templates/sub_agent_orchestration.yaml"),
    ),
    (
        "coding_agent_routing",
        "Coding Agent Routing",
        include_str!("../../templates/coding_agent_routing.yaml"),
    ),
    (
        "preference_aware_routing",
        "Preference-Aware Routing",
        include_str!("../../templates/preference_aware_routing.yaml"),
    ),
    (
        "filter_chain_guardrails",
        "Filter Chain Guardrails",
        include_str!("../../templates/filter_chain_guardrails.yaml"),
    ),
    (
        "conversational_state",
        "Conversational State",
        include_str!("../../templates/conversational_state.yaml"),
    ),
];

pub async fn run(
    template: Option<String>,
    clean: bool,
    output: Option<String>,
    force: bool,
    list_templates: bool,
) -> Result<()> {
    let bold = console::Style::new().bold();
    let dim = console::Style::new().dim();
    let green = console::Style::new().green();
    let cyan = console::Style::new().cyan();

    if list_templates {
        println!("\n{}:", bold.apply_to("Available templates"));
        for (id, name, _) in TEMPLATES {
            println!("  {} - {}", cyan.apply_to(id), name);
        }
        println!();
        return Ok(());
    }

    let output_path = output.unwrap_or_else(|| "plano_config.yaml".to_string());
    let output_path = Path::new(&output_path);

    if output_path.exists() && !force {
        bail!(
            "File {} already exists. Use --force to overwrite.",
            output_path.display()
        );
    }

    if clean {
        let content = "version: v0.3.0\nlisteners:\n  - type: model\n    name: egress_traffic\n    port: 12000\nmodel_providers: []\n";
        std::fs::write(output_path, content)?;
        println!(
            "{} Created clean config at {}",
            green.apply_to("✓"),
            output_path.display()
        );
        return Ok(());
    }

    if let Some(template_id) = template {
        let tmpl = TEMPLATES
            .iter()
            .find(|(id, _, _)| *id == template_id)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Unknown template '{}'. Use --list-templates to see available templates.",
                    template_id
                )
            })?;

        std::fs::write(output_path, tmpl.2)?;
        println!(
            "{} Created config from template '{}' at {}",
            green.apply_to("✓"),
            tmpl.1,
            output_path.display()
        );

        // Preview
        let lines: Vec<&str> = tmpl.2.lines().take(28).collect();
        println!("\n{}:", dim.apply_to("Preview"));
        for line in &lines {
            println!("  {}", dim.apply_to(line));
        }
        if tmpl.2.lines().count() > 28 {
            println!("  {}", dim.apply_to("..."));
        }

        return Ok(());
    }

    // Interactive mode using dialoguer
    if !atty::is(atty::Stream::Stdin) {
        bail!(
            "Interactive mode requires a TTY. Use --template or --clean for non-interactive mode."
        );
    }

    let selections: Vec<&str> = TEMPLATES.iter().map(|(_, name, _)| *name).collect();

    let selection = dialoguer::Select::new()
        .with_prompt("Choose a template")
        .items(&selections)
        .default(0)
        .interact()?;

    let tmpl = &TEMPLATES[selection];
    std::fs::write(output_path, tmpl.2)?;
    println!(
        "\n{} Created config from template '{}' at {}",
        green.apply_to("✓"),
        tmpl.1,
        output_path.display()
    );

    Ok(())
}
