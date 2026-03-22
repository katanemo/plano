use anyhow::Result;

pub async fn run(debug: bool, follow: bool, docker: bool) -> Result<()> {
    if !docker {
        crate::native::runner::native_logs(debug, follow)?;
    } else {
        crate::docker::stream_logs(debug, follow).await?;
    }
    Ok(())
}
