pub mod build;
pub mod cli_agent;
pub mod down;
pub mod init;
pub mod logs;
pub mod self_update;
pub mod up;
pub mod validate;

use clap::{Parser, Subcommand};

use crate::consts::PLANO_VERSION;

const LOGO: &str = r#"
 ______ _
 | ___ \ |
 | |_/ / | __ _ _ __   ___
 |  __/| |/ _` | '_ \ / _ \
 | |   | | (_| | | | | (_) |
 \_|   |_|\__,_|_| |_|\___/
"#;

#[derive(Parser)]
#[command(
    name = "planoai",
    about = "The Delivery Infrastructure for Agentic Apps"
)]
#[command(version = PLANO_VERSION)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Start Plano
    Up {
        /// Config file path (positional)
        file: Option<String>,

        /// Path to the directory containing config.yaml
        #[arg(long, default_value = ".")]
        path: String,

        /// Run Plano in the foreground
        #[arg(long)]
        foreground: bool,

        /// Start a local OTLP trace collector
        #[arg(long)]
        with_tracing: bool,

        /// Port for the OTLP trace collector
        #[arg(long, default_value_t = 4317)]
        tracing_port: u16,

        /// Run Plano inside Docker instead of natively
        #[arg(long)]
        docker: bool,
    },

    /// Stop Plano
    Down {
        /// Stop a Docker-based Plano instance
        #[arg(long)]
        docker: bool,
    },

    /// Build Plano from source
    Build {
        /// Build the Docker image instead of native binaries
        #[arg(long)]
        docker: bool,
    },

    /// Stream logs from Plano
    Logs {
        /// Show detailed debug logs
        #[arg(long)]
        debug: bool,

        /// Follow the logs
        #[arg(long)]
        follow: bool,

        /// Stream logs from a Docker-based Plano instance
        #[arg(long)]
        docker: bool,
    },

    /// Start a CLI agent connected to Plano
    CliAgent {
        /// The type of CLI agent to start
        #[arg(value_parser = ["claude", "codex"])]
        agent_type: String,

        /// Config file path (positional)
        file: Option<String>,

        /// Path to the directory containing plano_config.yaml
        #[arg(long, default_value = ".")]
        path: String,

        /// Additional settings as JSON string for the CLI agent
        #[arg(long, default_value = "{}")]
        settings: String,
    },

    /// Manage distributed traces
    Trace {
        #[command(subcommand)]
        command: TraceCommand,
    },

    /// Initialize a new Plano configuration
    Init {
        /// Use a built-in template
        #[arg(long)]
        template: Option<String>,

        /// Create a clean empty config
        #[arg(long)]
        clean: bool,

        /// Output file path
        #[arg(long, short)]
        output: Option<String>,

        /// Overwrite existing files
        #[arg(long)]
        force: bool,

        /// List available templates
        #[arg(long)]
        list_templates: bool,
    },

    /// Validate a Plano configuration file
    Validate {
        /// Config file path
        file: Option<String>,

        /// Path to the directory containing config.yaml
        #[arg(long, default_value = ".")]
        path: String,
    },

    /// Update planoai to the latest version
    #[command(name = "self-update")]
    SelfUpdate {
        /// Update to a specific version instead of latest
        #[arg(long)]
        version: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum TraceCommand {
    /// Start the OTLP trace listener
    Listen {
        /// Host to bind to
        #[arg(long, default_value = "0.0.0.0")]
        host: String,

        /// Port to listen on
        #[arg(long, default_value_t = 4317)]
        port: u16,
    },

    /// Stop the trace listener
    Down,

    /// Show a specific trace
    Show {
        /// Trace ID to display
        trace_id: String,

        /// Show verbose span details
        #[arg(long)]
        verbose: bool,
    },

    /// Tail recent traces
    Tail {
        /// Include spans matching these patterns
        #[arg(long)]
        include_spans: Option<String>,

        /// Exclude spans matching these patterns
        #[arg(long)]
        exclude_spans: Option<String>,

        /// Filter by attribute key=value
        #[arg(long, name = "KEY=VALUE")]
        r#where: Vec<String>,

        /// Show traces since (e.g. 10s, 5m, 1h)
        #[arg(long)]
        since: Option<String>,

        /// Show verbose span details
        #[arg(long)]
        verbose: bool,
    },
}

pub async fn run(cli: Cli) -> anyhow::Result<()> {
    // Initialize logging
    let log_level = std::env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_string());
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&log_level)),
        )
        .init();

    match cli.command {
        None => {
            print_logo();
            // Print help by re-parsing with --help
            let _ = Cli::parse_from(["planoai", "--help"]);
            Ok(())
        }
        Some(Command::Up {
            file,
            path,
            foreground,
            with_tracing,
            tracing_port,
            docker,
        }) => up::run(file, path, foreground, with_tracing, tracing_port, docker).await,
        Some(Command::Down { docker }) => down::run(docker).await,
        Some(Command::Build { docker }) => build::run(docker).await,
        Some(Command::Logs {
            debug,
            follow,
            docker,
        }) => logs::run(debug, follow, docker).await,
        Some(Command::CliAgent {
            agent_type,
            file,
            path,
            settings,
        }) => cli_agent::run(&agent_type, file, &path, &settings).await,
        Some(Command::Trace { command }) => match command {
            TraceCommand::Listen { host, port } => crate::trace::listen::run(&host, port).await,
            TraceCommand::Down => crate::trace::down::run().await,
            TraceCommand::Show { trace_id, verbose } => {
                crate::trace::show::run(&trace_id, verbose).await
            }
            TraceCommand::Tail {
                include_spans,
                exclude_spans,
                r#where,
                since,
                verbose,
            } => {
                crate::trace::tail::run(
                    include_spans.as_deref(),
                    exclude_spans.as_deref(),
                    &r#where,
                    since.as_deref(),
                    verbose,
                )
                .await
            }
        },
        Some(Command::Init {
            template,
            clean,
            output,
            force,
            list_templates,
        }) => init::run(template, clean, output, force, list_templates).await,
        Some(Command::Validate { file, path }) => validate::run(file, &path).await,
        Some(Command::SelfUpdate { version }) => self_update::run(version.as_deref()).await,
    }
}

fn print_logo() {
    let style = console::Style::new().bold().color256(141); // closest to #969FF4
    println!("{}", style.apply_to(LOGO));
    println!("  The Delivery Infrastructure for Agentic Apps\n");
}
