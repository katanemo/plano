use anyhow::{bail, Result};

pub async fn run(trace_id: &str, verbose: bool) -> Result<()> {
    // TODO: Connect to trace listener via gRPC and fetch trace
    // For now, print a placeholder
    println!("Showing trace: {trace_id}");
    if verbose {
        println!("(verbose mode)");
    }

    // The full implementation will:
    // 1. Connect to the gRPC trace query service
    // 2. Fetch the trace by ID
    // 3. Build a span tree
    // 4. Render it using console styling

    bail!("Trace show is not yet fully implemented. The gRPC trace query service needs to be running.")
}
