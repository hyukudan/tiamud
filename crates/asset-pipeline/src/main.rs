use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use tiamud_asset_pipeline::{
    AssetProfile, AuthoringMetadata, BuildLimits, BuildRequest, FocalPoint, QuarantineRequest,
    SpritePose, build_asset, build_quarantined, quarantine_source, review_quarantine,
};

#[derive(Debug, Parser)]
#[command(
    name = "tiamud-assets",
    about = "Compile bounded atmospheric scenes and static entity sprites"
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

    /// Build profile-specific browser variants and a content-addressed manifest.
    Build {
        /// Reviewed PNG, JPEG, or WebP source (minimum width depends on kind).
        #[arg(long)]
        source: PathBuf,

        /// Publication profile; defaults to the backwards-compatible atmosphere profile.
        #[arg(long, value_enum, default_value_t = CliAssetKind::Atmosphere)]
        kind: CliAssetKind,

        /// Static entity pose. Required only with --kind entity-sprite.
        #[arg(long, value_enum)]
        pose: Option<CliSpritePose>,

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

        /// Publish deterministic PNG-only atmosphere variants for lossless content.
        #[arg(long)]
        lossless: bool,
    },

    /// Publish variants from a reviewed, digest-verified quarantine record.
    Publish {
        #[arg(long)]
        record: PathBuf,
        #[arg(long, value_enum, default_value_t = CliAssetKind::Atmosphere)]
        kind: CliAssetKind,
        #[arg(long, value_enum)]
        pose: Option<CliSpritePose>,
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
        /// Publish deterministic PNG-only atmosphere variants for lossless content.
        #[arg(long)]
        lossless: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum CliAssetKind {
    Atmosphere,
    EntitySprite,
}

impl std::fmt::Display for CliAssetKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Atmosphere => "atmosphere",
            Self::EntitySprite => "entity-sprite",
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum CliSpritePose {
    Idle,
    Attack,
    Hit,
    Defeated,
}

fn asset_profile(
    kind: CliAssetKind,
    pose: Option<CliSpritePose>,
) -> Result<AssetProfile, &'static str> {
    match (kind, pose) {
        (CliAssetKind::Atmosphere, None) => Ok(AssetProfile::Atmosphere),
        (CliAssetKind::Atmosphere, Some(_)) => {
            Err("--pose is valid only with --kind entity-sprite")
        }
        (CliAssetKind::EntitySprite, None) => Err("--pose is required with --kind entity-sprite"),
        (CliAssetKind::EntitySprite, Some(pose)) => Ok(AssetProfile::EntitySprite {
            pose: match pose {
                CliSpritePose::Idle => SpritePose::Idle,
                CliSpritePose::Attack => SpritePose::Attack,
                CliSpritePose::Hit => SpritePose::Hit,
                CliSpritePose::Defeated => SpritePose::Defeated,
            },
        }),
    }
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
            kind,
            pose,
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
                profile: asset_profile(kind, pose)?,
                lossless,
            })?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Publish {
            record,
            kind,
            pose,
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
                    profile: asset_profile(kind, pose)?,
                    lossless,
                },
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn required_build_args() -> [&'static str; 15] {
        [
            "tiamud-assets",
            "build",
            "--source",
            "source.png",
            "--alt",
            "A sprite.",
            "--output",
            "out",
            "--author",
            "Tester",
            "--license",
            "LicenseRef-Test",
            "--provenance",
            "Fixture",
            "--reviewed-at",
        ]
    }

    #[test]
    fn build_cli_defaults_to_atmosphere_and_accepts_an_explicit_static_sprite_pose() {
        let mut atmosphere = required_build_args().to_vec();
        atmosphere.push("2026-07-16T00:00:00Z");
        let parsed = Cli::try_parse_from(atmosphere).unwrap();
        assert!(matches!(
            parsed.command,
            Command::Build {
                kind: CliAssetKind::Atmosphere,
                pose: None,
                ..
            }
        ));

        let mut sprite = required_build_args().to_vec();
        sprite.extend([
            "2026-07-16T00:00:00Z",
            "--kind",
            "entity-sprite",
            "--pose",
            "hit",
        ]);
        let parsed = Cli::try_parse_from(sprite).unwrap();
        let Command::Build { kind, pose, .. } = parsed.command else {
            panic!("expected build command");
        };
        assert_eq!(
            asset_profile(kind, pose).unwrap(),
            AssetProfile::EntitySprite {
                pose: SpritePose::Hit
            }
        );
    }

    #[test]
    fn profile_validation_rejects_missing_or_misplaced_poses() {
        assert!(asset_profile(CliAssetKind::EntitySprite, None).is_err());
        assert!(asset_profile(CliAssetKind::Atmosphere, Some(CliSpritePose::Idle)).is_err());
    }

    #[test]
    fn animated_cli_options_are_not_part_of_the_static_contract() {
        let mut args = required_build_args().to_vec();
        args.extend([
            "2026-07-16T00:00:00Z",
            "--kind",
            "entity-sprite",
            "--pose",
            "idle",
            "--animated",
        ]);
        assert!(Cli::try_parse_from(args).is_err());
    }
}
