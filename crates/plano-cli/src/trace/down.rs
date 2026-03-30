use anyhow::Result;

use crate::trace::daemon;

pub async fn run() -> Result<()> {
    let green = console::Style::new().green();

    if daemon::get_listener_pid().is_none() {
        eprintln!("No trace listener running.");
        return Ok(());
    }

    daemon::stop_listener_process()?;
    eprintln!("{} Trace listener stopped.", green.apply_to("✓"));
    Ok(())
}
