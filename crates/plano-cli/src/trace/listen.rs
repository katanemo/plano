use anyhow::Result;

use crate::trace::daemon;
use crate::trace::store::TraceStore;

/// Start the trace listener in the foreground.
pub async fn run(host: &str, port: u16) -> Result<()> {
    let green = console::Style::new().green();
    let cyan = console::Style::new().cyan();

    // Check if already running
    if let Some(pid) = daemon::get_listener_pid() {
        eprintln!(
            "{} Trace listener already running (PID {pid})",
            green.apply_to("✓")
        );
        return Ok(());
    }

    eprintln!(
        "{} Starting trace listener on {}",
        green.apply_to("✓"),
        cyan.apply_to(format!("{host}:{port}"))
    );

    // Start as a background task in this process
    start_background(port).await?;

    // Write PID
    daemon::write_listener_pid(std::process::id())?;

    // Wait forever (until ctrl+c)
    tokio::signal::ctrl_c().await?;

    daemon::remove_listener_pid()?;
    eprintln!("\nTrace listener stopped.");
    Ok(())
}

/// Start the trace listener in the background (within the current process).
pub async fn start_background(port: u16) -> Result<()> {
    let store = TraceStore::shared();

    // TODO: Implement gRPC OTLP listener using tonic
    // For now, spawn a placeholder task
    let _store = store.clone();
    tokio::spawn(async move {
        // The actual gRPC server will be implemented here
        // using tonic with the OTLP ExportTraceServiceRequest handler
        tracing::info!("Trace listener background task started on port {port}");
        // Keep running
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        }
    });

    Ok(())
}
