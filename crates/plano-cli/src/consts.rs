use std::path::PathBuf;

pub const PLANO_COLOR: &str = "#969FF4";
pub const SERVICE_NAME: &str = "plano";
pub const PLANO_DOCKER_NAME: &str = "plano";
pub const PLANO_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const DEFAULT_OTEL_TRACING_GRPC_ENDPOINT: &str = "http://localhost:4317";
pub const DEFAULT_NATIVE_OTEL_TRACING_GRPC_ENDPOINT: &str = "http://localhost:4317";

pub const ENVOY_VERSION: &str = "v1.37.0";
pub const PLANO_GITHUB_REPO: &str = "katanemo/archgw";

pub fn plano_docker_image() -> String {
    std::env::var("PLANO_DOCKER_IMAGE")
        .unwrap_or_else(|_| format!("katanemo/plano:{PLANO_VERSION}"))
}

pub fn plano_home() -> PathBuf {
    dirs::home_dir()
        .expect("could not determine home directory")
        .join(".plano")
}

pub fn plano_run_dir() -> PathBuf {
    plano_home().join("run")
}

pub fn plano_bin_dir() -> PathBuf {
    plano_home().join("bin")
}

pub fn plano_plugins_dir() -> PathBuf {
    plano_home().join("plugins")
}

pub fn native_pid_file() -> PathBuf {
    plano_run_dir().join("plano.pid")
}

pub fn plano_release_base_url() -> String {
    format!("https://github.com/{PLANO_GITHUB_REPO}/releases/download")
}
