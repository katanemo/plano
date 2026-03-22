use std::fs;
use std::path::PathBuf;

use anyhow::Result;
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;

use crate::consts::plano_run_dir;

pub fn pid_file_path() -> PathBuf {
    plano_run_dir().join("trace_listener.pid")
}

pub fn log_file_path() -> PathBuf {
    plano_run_dir().join("trace_listener.log")
}

pub fn write_listener_pid(pid: u32) -> Result<()> {
    let run_dir = plano_run_dir();
    fs::create_dir_all(&run_dir)?;
    fs::write(pid_file_path(), pid.to_string())?;
    Ok(())
}

pub fn remove_listener_pid() -> Result<()> {
    let path = pid_file_path();
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

pub fn get_listener_pid() -> Option<u32> {
    let content = fs::read_to_string(pid_file_path()).ok()?;
    let pid: u32 = content.trim().parse().ok()?;
    // Check if alive
    if kill(Pid::from_raw(pid as i32), None).is_ok() {
        Some(pid)
    } else {
        None
    }
}

pub fn stop_listener_process() -> Result<()> {
    if let Some(pid) = get_listener_pid() {
        let nix_pid = Pid::from_raw(pid as i32);
        let _ = kill(nix_pid, Signal::SIGTERM);

        // Brief wait
        std::thread::sleep(std::time::Duration::from_millis(500));

        // Force kill if still alive
        if kill(nix_pid, None).is_ok() {
            let _ = kill(nix_pid, Signal::SIGKILL);
        }
    }

    remove_listener_pid()?;
    Ok(())
}
