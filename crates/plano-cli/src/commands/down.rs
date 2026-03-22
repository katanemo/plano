use anyhow::Result;

use crate::utils::print_cli_header;

pub async fn run(docker: bool) -> Result<()> {
    print_cli_header();

    if !docker {
        crate::native::runner::stop_native()?;
    } else {
        crate::docker::stop_container().await?;
    }

    Ok(())
}
