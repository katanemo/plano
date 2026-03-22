#![allow(dead_code)]

mod commands;
mod config;
mod consts;
mod docker;
mod native;
mod trace;
mod utils;
mod version;

use clap::Parser;
use commands::Cli;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    if let Err(e) = commands::run(cli).await {
        eprintln!("Error: {e:#}");
        std::process::exit(1);
    }
}
