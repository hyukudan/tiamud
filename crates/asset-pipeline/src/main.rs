use std::path::PathBuf;

use clap::{Parser, Subcommand};
use envy_asset_pipeline::{BuildLimits, BuildRequest, build_asset};

#[derive(Debug, Parser)]
#[command(
    name = "envy-assets",
    about = "Compile bounded atmospheric scene assets"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Build AVIF-first browser variants and a content-addressed manifest.
    Build {
        /// Reviewed PNG, JPEG, or WebP source (at least 1280 pixels wide).
        #[arg(long)]
        source: PathBuf,

        /// Perception-safe alternative text for the scene.
        #[arg(long)]
        alt: String,

        /// Directory under which the content-addressed bundle is published.
        #[arg(long)]
        output: PathBuf,
    },
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let Cli { command } = Cli::parse();
    match command {
        Command::Build {
            source,
            alt,
            output,
        } => {
            let result = build_asset(&BuildRequest {
                source,
                alt,
                output_dir: output,
                limits: BuildLimits::default(),
            })?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
    }
    Ok(())
}
