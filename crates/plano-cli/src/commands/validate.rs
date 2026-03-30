use anyhow::Result;

use crate::native::runner::validate_config;
use crate::utils::find_config_file;

pub async fn run(file: Option<String>, path: &str) -> Result<()> {
    let green = console::Style::new().green();
    let red = console::Style::new().red();

    let config_path = find_config_file(path, file.as_deref());
    if !config_path.exists() {
        eprintln!(
            "{} Config file not found: {}",
            red.apply_to("✗"),
            config_path.display()
        );
        std::process::exit(1);
    }

    match validate_config(&config_path) {
        Ok(()) => {
            eprintln!("{} Configuration valid", green.apply_to("✓"));
            Ok(())
        }
        Err(e) => {
            eprintln!("{} Validation failed", red.apply_to("✗"));
            eprintln!("  {e:#}");
            std::process::exit(1);
        }
    }
}
