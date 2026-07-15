use std::path::PathBuf;

use clap::{Parser, Subcommand};
use tiamud_asset_pipeline::{
    AuthoringMetadata, BuildLimits, BuildRequest, FocalPoint, QuarantineRequest, build_asset,
    build_quarantined, quarantine_source, review_quarantine,
};

#[derive(Debug, Parser)]
#[command(
    name = "tiamud-assets",
    about = "Compile bounded atmospheric scene assets"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Decode and sanitize an untrusted source into a private pending-review area.
    Quarantine {
        #[arg(long)]
        source: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },

    /// Attach an explicit, immutable human review to a quarantined source.
    Review {
        #[arg(long)]
        record: PathBuf,
        #[arg(long)]
        reviewer: String,
        #[arg(long)]
        reviewed_at: String,
    },

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

        /// Monotonic editorial revision of this logical asset.
        #[arg(long, default_value_t = 1)]
        revision: u32,

        /// Horizontal focal point, from 0 (left) to 1000 (right).
        #[arg(long, default_value_t = 500)]
        focal_x_per_mille: u16,

        /// Vertical focal point, from 0 (top) to 1000 (bottom).
        #[arg(long, default_value_t = 500)]
        focal_y_per_mille: u16,

        /// Human or organization responsible for the reviewed asset.
        #[arg(long)]
        author: String,

        /// SPDX expression or project-specific LicenseRef.
        #[arg(long)]
        license: String,

        /// Traceable description of the source and editorial process.
        #[arg(long)]
        provenance: String,

        /// Optional tool or generative model used during authoring.
        #[arg(long)]
        generator: Option<String>,

        /// Operator-defined content classification.
        #[arg(long, default_value = "general")]
        content_classification: String,

        /// Human review timestamp (RFC 3339 recommended).
        #[arg(long)]
        reviewed_at: String,

        /// Publish deterministic PNG-only variants for lossless content.
        #[arg(long)]
        lossless: bool,
    },

    /// Publish variants from a reviewed, digest-verified quarantine record.
    Publish {
        #[arg(long)]
        record: PathBuf,
        #[arg(long)]
        alt: String,
        #[arg(long)]
        output: PathBuf,
        #[arg(long, default_value_t = 1)]
        revision: u32,
        #[arg(long, default_value_t = 500)]
        focal_x_per_mille: u16,
        #[arg(long, default_value_t = 500)]
        focal_y_per_mille: u16,
        #[arg(long)]
        author: String,
        #[arg(long)]
        license: String,
        #[arg(long)]
        provenance: String,
        #[arg(long)]
        generator: Option<String>,
        #[arg(long, default_value = "general")]
        content_classification: String,
        #[arg(long)]
        reviewed_at: String,
        #[arg(long)]
        lossless: bool,
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
        Command::Quarantine { source, output } => {
            let result = quarantine_source(&QuarantineRequest {
                source,
                quarantine_dir: output,
                limits: BuildLimits::default(),
            })?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Review {
            record,
            reviewer,
            reviewed_at,
        } => {
            let result = review_quarantine(
                &record,
                &reviewer,
                &reviewed_at,
                BuildLimits::default().max_source_bytes,
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Build {
            source,
            alt,
            output,
            revision,
            focal_x_per_mille,
            focal_y_per_mille,
            author,
            license,
            provenance,
            generator,
            content_classification,
            reviewed_at,
            lossless,
        } => {
            let result = build_asset(&BuildRequest {
                source,
                alt,
                output_dir: output,
                limits: BuildLimits::default(),
                metadata: AuthoringMetadata {
                    revision,
                    focal_point: FocalPoint {
                        x_per_mille: focal_x_per_mille,
                        y_per_mille: focal_y_per_mille,
                    },
                    author,
                    license,
                    provenance,
                    generator,
                    content_classification,
                    human_reviewed_at: reviewed_at,
                },
                lossless,
            })?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Publish {
            record,
            alt,
            output,
            revision,
            focal_x_per_mille,
            focal_y_per_mille,
            author,
            license,
            provenance,
            generator,
            content_classification,
            reviewed_at,
            lossless,
        } => {
            let result = build_quarantined(
                &record,
                BuildRequest {
                    source: PathBuf::new(),
                    alt,
                    output_dir: output,
                    limits: BuildLimits::default(),
                    metadata: AuthoringMetadata {
                        revision,
                        focal_point: FocalPoint {
                            x_per_mille: focal_x_per_mille,
                            y_per_mille: focal_y_per_mille,
                        },
                        author,
                        license,
                        provenance,
                        generator,
                        content_classification,
                        human_reviewed_at: reviewed_at,
                    },
                    lossless,
                },
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
    }
    Ok(())
}
