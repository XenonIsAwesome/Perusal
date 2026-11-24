use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "perusal-cli", about = "Headless harness for Perusal")]
struct Cli { #[command(subcommand)] cmd: Cmd }

#[derive(Subcommand)]
enum Cmd {
    /// Generate a synthetic MR repository (main + feature branches).
    Gen { dir: PathBuf, #[arg(long, default_value_t = 10_000)] files: usize, #[arg(long, default_value_t = 500_000)] lines: usize },
}

fn main() -> std::io::Result<()> {
    match Cli::parse().cmd {
        Cmd::Gen { dir, files, lines } => perusal_cli::generate(&dir, files, lines),
    }
}
