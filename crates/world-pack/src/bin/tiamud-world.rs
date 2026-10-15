use std::{
    collections::{BTreeSet, HashMap},
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::ExitCode,
};

#[cfg(feature = "editorial-ai")]
use std::{env, fs::File, time::Duration};

#[cfg(feature = "editorial-ai")]
use clap::Args;
use clap::{Parser, Subcommand, ValueEnum};
use fd_lock::RwLock;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
#[cfg(feature = "editorial-ai")]
use tiamud_editorial_ai::{
    AdapterLimits, EditorialAdapter, EditorialError, EditorialRequest, EditorialSuggestion,
    EditorialTask, MAX_CONNECT_TIMEOUT, MAX_INPUT_BYTES, MAX_OUTPUT_CHARS, MAX_RESPONSE_BYTES,
    MAX_TIMEOUT, ProviderConfig,
};
use world_pack::{
    ActiveAreaCatalog, AreaCatalog, AreaCatalogError, CompositionError, ConditionContext,
    DocumentToolError, LocalAreaGeneration, LocalAreaRegistryError, MAX_LOCAL_AREA_POINTER_BYTES,
    PreviewFormat, TimePeriod, Weather, WorldPack, WorldPackDiagnostic, WorldPackDocument,
    build_area_release, compose_world_project, local_active_area_catalog_path,
    local_area_bundle_path, local_area_catalog_path, local_area_world_path,
    parse_area_release_manifest, read_bounded_local_registry_file,
    read_bounded_regular_file as read_bounded_local_file, verify_area_release,
    verify_existing_registry_directory,
};
#[cfg(test)]
use world_pack::{MAX_TOTAL_AREA_BYTES, parse_area_catalog};

const MAX_DOCUMENT_BYTES: u64 = 32 * 1024 * 1024;
const MAX_AREA_RELEASE_INPUT_BYTES: usize = 256 * 1024;
const MAX_AREA_CATALOG_INPUT_BYTES: usize = 1024 * 1024;
const MAX_AREA_BUNDLE_INPUT_BYTES: usize = 16 * 1024 * 1024;
const MAX_AREA_WORLD_INPUT_BYTES: usize = 16 * 1024 * 1024;
#[cfg(feature = "editorial-ai")]
const MAX_EDITORIAL_ARTIFACT_BYTES: usize = 512 * 1024;
#[cfg(feature = "editorial-ai")]
const EDITORIAL_ARTIFACT_KIND: &str = "tiamud.editorial-review";
#[cfg(feature = "editorial-ai")]
const EDITORIAL_ARTIFACT_SCHEMA: u8 = 1;

#[derive(Debug, Parser)]
#[command(
    name = "tiamud-world",
    version,
    about = "Compose, validate, build, compare, preview, and test Tiamud WorldPacks"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Compose strict AreaPack v9 modules offline into one canonical runtime WorldPack v10.
    Compose {
        project: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Validate a WorldPack and all authoring extensions without writing files.
    Validate { input: PathBuf },
    /// Build canonical, deterministic WorldPack JSON.
    Build {
        input: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        /// Replace an existing output file. The input is always read first.
        #[arg(long)]
        force: bool,
    },
    /// Show a deterministic semantic summary between two valid WorldPacks.
    Diff { before: PathBuf, after: PathBuf },
    /// Render an authorized scene without exposing variant ids or conditions.
    Preview {
        input: PathBuf,
        #[arg(long)]
        room: String,
        #[arg(long, value_enum, default_value_t = CliPreviewFormat::Text)]
        format: CliPreviewFormat,
        #[arg(long, value_enum, default_value_t = CliTimePeriod::Day)]
        time_period: CliTimePeriod,
        #[arg(long, value_enum, default_value_t = CliWeather::Clear)]
        weather: CliWeather,
        #[arg(long, default_value_t = 12, value_parser = clap::value_parser!(u8).range(0..=23))]
        hour: u8,
        #[arg(long, default_value_t = 1)]
        actor_level: u32,
        #[arg(long)]
        admin: bool,
        #[arg(long = "actor-flag")]
        actor_flags: Vec<String>,
    },
    /// Run the content expectations embedded in a WorldPack document.
    Test { input: PathBuf },
    /// Publish an immutable validated artifact and atomically make it active.
    Publish {
        input: PathBuf,
        #[arg(long)]
        registry: PathBuf,
    },
    /// Atomically reactivate an already published artifact by its SHA-256.
    Rollback {
        #[arg(long)]
        registry: PathBuf,
        #[arg(long)]
        world: String,
        #[arg(long)]
        sha256: String,
    },
    /// Publish immutable AreaPack bundles, their catalog, and a monolithic fallback.
    AreaPublish {
        project: PathBuf,
        #[arg(long)]
        release: PathBuf,
        #[arg(long)]
        registry: PathBuf,
    },
    /// Verify one content-addressed area catalog and every referenced local blob.
    AreaVerify {
        #[arg(long)]
        catalog: PathBuf,
        #[arg(long)]
        registry: PathBuf,
    },
    /// Select an already published exact area catalog for activation or rollback.
    AreaSelect {
        #[arg(long)]
        catalog: PathBuf,
        #[arg(long)]
        registry: PathBuf,
    },
    #[cfg(feature = "editorial-ai")]
    /// Ask an optional editorial backend for an unreviewed, authoring-only draft.
    AiDraft(AiDraftArgs),
    #[cfg(feature = "editorial-ai")]
    /// Record an explicit human decision in a new authoring-only review artifact.
    AiReview(AiReviewArgs),
}

#[cfg(feature = "editorial-ai")]
#[derive(Debug, Args)]
struct AiDraftArgs {
    /// Safe default is disabled; choosing a backend is always explicit.
    #[arg(long, value_enum, default_value_t = CliAiProvider::None)]
    provider: CliAiProvider,
    /// Base endpoint. Local providers only accept loopback; remote endpoints require HTTPS.
    #[arg(long)]
    endpoint: Option<String>,
    /// Provider model name.
    #[arg(long)]
    model: Option<String>,
    /// Name of an environment variable containing an Authorization header.
    #[arg(long)]
    authorization_env: Option<String>,
    #[arg(long, value_enum)]
    task: CliEditorialTask,
    /// UTF-8 author prompt file (maximum 64 KiB).
    #[arg(long)]
    input: PathBuf,
    /// Optional UTF-8 context file (maximum 64 KiB).
    #[arg(long)]
    context: Option<PathBuf>,
    /// New JSON draft path. Existing files are never replaced.
    #[arg(short, long)]
    output: PathBuf,
    #[arg(
        long,
        default_value_t = 30,
        value_parser = clap::value_parser!(u64).range(1..=60)
    )]
    timeout_seconds: u64,
    #[arg(
        long,
        default_value_t = 3,
        value_parser = clap::value_parser!(u64).range(1..=10)
    )]
    connect_timeout_seconds: u64,
    #[arg(
        long,
        default_value_t = 262_144,
        value_parser = clap::value_parser!(u64).range(1..=262_144)
    )]
    max_response_bytes: u64,
    #[arg(
        long,
        default_value_t = 16_384,
        value_parser = clap::value_parser!(u64).range(1..=16_384)
    )]
    max_output_chars: u64,
}

#[cfg(feature = "editorial-ai")]
#[derive(Debug, Args)]
struct AiReviewArgs {
    /// Unreviewed artifact previously written by ai-draft.
    draft: PathBuf,
    #[arg(long)]
    reviewer: String,
    /// Review timestamp supplied by the reviewer, preferably RFC 3339.
    #[arg(long)]
    reviewed_at: String,
    /// Explicit decision; omission is an error.
    #[arg(long, value_enum)]
    decision: CliReviewDecision,
    /// New JSON review path. The original draft is never changed.
    #[arg(short, long)]
    output: PathBuf,
}

#[cfg(feature = "editorial-ai")]
#[derive(Debug, Clone, Copy, Default, ValueEnum)]
enum CliAiProvider {
    #[default]
    None,
    Ollama,
    LlamaCpp,
    Compatible,
}

#[cfg(feature = "editorial-ai")]
#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliEditorialTask {
    DraftDescription,
    SuggestAltText,
    ReviewConsistency,
    FindSuspiciousReferences,
    BrainstormNpc,
    BrainstormQuest,
}

#[cfg(feature = "editorial-ai")]
impl From<CliEditorialTask> for EditorialTask {
    fn from(value: CliEditorialTask) -> Self {
        match value {
            CliEditorialTask::DraftDescription => Self::DraftDescription,
            CliEditorialTask::SuggestAltText => Self::SuggestAltText,
            CliEditorialTask::ReviewConsistency => Self::ReviewConsistency,
            CliEditorialTask::FindSuspiciousReferences => Self::FindSuspiciousReferences,
            CliEditorialTask::BrainstormNpc => Self::BrainstormNpc,
            CliEditorialTask::BrainstormQuest => Self::BrainstormQuest,
        }
    }
}

#[cfg(feature = "editorial-ai")]
#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliReviewDecision {
    Accept,
    Reject,
}

#[cfg(feature = "editorial-ai")]
impl CliReviewDecision {
    const fn accepted(self) -> bool {
        matches!(self, Self::Accept)
    }
}

#[cfg(feature = "editorial-ai")]
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum EditorialArtifactState {
    Draft,
    Reviewed,
}

#[cfg(feature = "editorial-ai")]
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EditorialReviewArtifact {
    schema_version: u8,
    kind: String,
    state: EditorialArtifactState,
    suggestion: EditorialSuggestion,
    input_sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    context_sha256: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, ValueEnum)]
enum CliPreviewFormat {
    #[default]
    Text,
    Ansi,
}

impl From<CliPreviewFormat> for PreviewFormat {
    fn from(value: CliPreviewFormat) -> Self {
        match value {
            CliPreviewFormat::Text => Self::Text,
            CliPreviewFormat::Ansi => Self::Ansi,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, ValueEnum)]
enum CliTimePeriod {
    Dawn,
    #[default]
    Day,
    Dusk,
    Night,
}

impl From<CliTimePeriod> for TimePeriod {
    fn from(value: CliTimePeriod) -> Self {
        match value {
            CliTimePeriod::Dawn => Self::Dawn,
            CliTimePeriod::Day => Self::Day,
            CliTimePeriod::Dusk => Self::Dusk,
            CliTimePeriod::Night => Self::Night,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, ValueEnum)]
enum CliWeather {
    #[default]
    Clear,
    Cloudy,
    Rain,
    Storm,
    Snow,
    Fog,
}

impl From<CliWeather> for Weather {
    fn from(value: CliWeather) -> Self {
        match value {
            CliWeather::Clear => Self::Clear,
            CliWeather::Cloudy => Self::Cloudy,
            CliWeather::Rain => Self::Rain,
            CliWeather::Storm => Self::Storm,
            CliWeather::Snow => Self::Snow,
            CliWeather::Fog => Self::Fog,
        }
    }
}

#[derive(Debug, Error)]
enum CliError {
    #[error("could not compose project {path:?}: {source}")]
    Composition {
        path: PathBuf,
        source: Box<CompositionError>,
    },
    #[error("could not inspect {path:?}: {source}")]
    Inspect {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("WorldPack {path:?} contains {size} bytes; the limit is {maximum} bytes")]
    TooLarge {
        path: PathBuf,
        size: u64,
        maximum: u64,
    },
    #[error("could not read {path:?}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not process {path:?}: {source}")]
    Document {
        path: PathBuf,
        source: DocumentToolError,
    },
    #[error("asset validation failed for {path:?}: {reason}")]
    Asset { path: PathBuf, reason: String },
    #[error("output {0:?} already exists and will not be replaced")]
    OutputExists(PathBuf),
    #[error("could not write {path:?}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{failed} of {total} content test(s) failed")]
    ContentTestsFailed { failed: usize, total: usize },
    #[error("published artifact digest must be 64 lowercase hexadecimal characters")]
    InvalidDigest,
    #[error("published artifact does not belong to world {0:?}")]
    WrongPublishedWorld(String),
    #[error("published artifact digest does not match its bytes")]
    PublishedDigestMismatch,
    #[error("area catalog operation failed: {0}")]
    AreaCatalog(#[from] AreaCatalogError),
    #[error("invalid area registry: {0}")]
    InvalidAreaRegistry(String),
    #[error("could not acquire area publication lock {path:?}: {source}")]
    AreaPublicationLock {
        path: PathBuf,
        source: std::io::Error,
    },
    #[cfg(feature = "editorial-ai")]
    #[error("invalid editorial AI configuration: {0}")]
    InvalidAiConfiguration(String),
    #[cfg(feature = "editorial-ai")]
    #[error("editorial AI operation failed: {0}")]
    Editorial(#[from] EditorialError),
    #[cfg(feature = "editorial-ai")]
    #[error("editorial input {0:?} is not valid UTF-8")]
    InvalidEditorialText(PathBuf),
    #[cfg(feature = "editorial-ai")]
    #[error("environment variable {0:?} is required and must contain valid Unicode")]
    MissingAuthorization(String),
    #[cfg(feature = "editorial-ai")]
    #[error("could not decode editorial artifact {path:?}: {source}")]
    EditorialJson {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[cfg(feature = "editorial-ai")]
    #[error("invalid editorial artifact: {0}")]
    InvalidEditorialArtifact(String),
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ActivePublication {
    schema: u8,
    world_id: String,
    sha256: String,
    artifact: String,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), CliError> {
    match cli.command {
        Command::Compose { project, output } => {
            let composed =
                compose_world_project(&project).map_err(|source| CliError::Composition {
                    path: project.clone(),
                    source: Box::new(source),
                })?;
            report_world_diagnostics(&composed.document.world);
            write_artifact_atomically(&output, &composed.artifact)?;
            println!(
                "composed {} area(s) into canonical WorldPack {:?} at {:?} ({} bytes, sha256:{}, {} cross-area reference(s))",
                composed.report.areas.len(),
                composed.report.project_id,
                output,
                composed.report.artifact_bytes,
                composed.report.artifact_sha256,
                composed.report.cross_area_references.len(),
            );
        }
        Command::Validate { input } => {
            let document = load_document(&input)?;
            validate_document(&input, &document)?;
            validate_document_assets(&input, &document)?;
            report_world_diagnostics(&document.world);
            println!(
                "valid WorldPack {:?}: {} rooms, {} item prototype(s), {} initial item instance(s), {} loot table(s), {} loot source(s), {} scene variant set(s), {} content test(s)",
                document.world.id,
                document.world.rooms.len(),
                document.world.item_prototypes.len(),
                document.world.initial_item_instances.len(),
                document.world.gameplay.loot_tables.len(),
                document.world.gameplay.loot_sources.len(),
                document.scene_variants.len(),
                document.content_tests.len()
            );
        }
        Command::Build {
            input,
            output,
            force,
        } => {
            let document = load_document(&input)?;
            validate_document_assets(&input, &document)?;
            report_world_diagnostics(&document.world);
            let artifact =
                document
                    .deterministic_artifact()
                    .map_err(|source| CliError::Document {
                        path: input.clone(),
                        source,
                    })?;
            write_artifact(&output, &artifact, force)?;
            println!(
                "built deterministic WorldPack {:?} at {:?} ({} bytes)",
                document.world.id,
                output,
                artifact.len()
            );
        }
        Command::Diff { before, after } => {
            let before_document = load_document(&before)?;
            let after_document = load_document(&after)?;
            let diff =
                before_document
                    .diff(&after_document)
                    .map_err(|source| CliError::Document {
                        path: after,
                        source,
                    })?;
            print!("{diff}");
        }
        Command::Preview {
            input,
            room,
            format,
            time_period,
            weather,
            hour,
            actor_level,
            admin,
            actor_flags,
        } => {
            let document = load_document(&input)?;
            let context = ConditionContext {
                world_time_period: time_period.into(),
                world_weather: weather.into(),
                world_time_hour: hour,
                actor_level,
                actor_is_admin: admin,
                actor_flags: actor_flags.into_iter().collect::<BTreeSet<_>>(),
                ..ConditionContext::default()
            };
            let preview = document
                .render_preview(&room, &context, format.into())
                .map_err(|source| CliError::Document {
                    path: input,
                    source: source.into(),
                })?;
            print!("{preview}");
        }
        Command::Test { input } => {
            let document = load_document(&input)?;
            let report = document
                .run_content_tests()
                .map_err(|source| CliError::Document {
                    path: input,
                    source,
                })?;
            for failure in &report.failures {
                eprintln!("FAIL {}", failure.name);
                for message in &failure.messages {
                    eprintln!("  - {message}");
                }
            }
            if !report.is_success() {
                return Err(CliError::ContentTestsFailed {
                    failed: report.failures.len(),
                    total: report.total,
                });
            }
            println!("content tests passed: {}/{}", report.passed, report.total);
        }
        Command::Publish { input, registry } => {
            let document = load_document(&input)?;
            validate_document_assets(&input, &document)?;
            report_world_diagnostics(&document.world);
            let artifact =
                document
                    .deterministic_artifact()
                    .map_err(|source| CliError::Document {
                        path: input,
                        source,
                    })?;
            let digest = sha256_hex(&artifact);
            let path = publication_path(&registry, &document.world.id, &digest);
            write_immutable_artifact(&path, &artifact)?;
            activate_publication(&registry, &document.world.id, &digest, &path)?;
            println!(
                "published world {:?} as sha256:{} and activated it",
                document.world.id, digest
            );
        }
        Command::Rollback {
            registry,
            world,
            sha256,
        } => {
            validate_digest(&sha256)?;
            let path = publication_path(&registry, &world, &sha256);
            let document = load_document(&path)?;
            validate_document(&path, &document)?;
            if document.world.id != world {
                return Err(CliError::WrongPublishedWorld(world));
            }
            let bytes = fs::read(&path).map_err(|source| CliError::Read {
                path: path.clone(),
                source,
            })?;
            if sha256_hex(&bytes) != sha256 {
                return Err(CliError::PublishedDigestMismatch);
            }
            activate_publication(&registry, &world, &sha256, &path)?;
            println!("rolled world {world:?} back to sha256:{sha256}");
        }
        Command::AreaPublish {
            project,
            release,
            registry,
        } => publish_area_release(&project, &release, &registry)?,
        Command::AreaVerify { catalog, registry } => {
            let (catalog, digest) = load_verified_area_catalog(&catalog, &registry)?;
            println!(
                "verified area catalog {:?} revision {} as sha256:{} ({} area bundle(s))",
                catalog.world_id,
                catalog.catalog_revision,
                digest,
                catalog.entries.len()
            );
        }
        Command::AreaSelect { catalog, registry } => {
            let selection = select_area_catalog(&catalog, &registry)?;
            println!("{}", area_selection_success_message(&selection));
        }
        #[cfg(feature = "editorial-ai")]
        Command::AiDraft(arguments) => {
            run_ai_draft(arguments, |config, limits, request| {
                EditorialAdapter::new(config, limits)?.suggest(request)
            })?;
        }
        #[cfg(feature = "editorial-ai")]
        Command::AiReview(arguments) => run_ai_review(arguments)?,
    }
    Ok(())
}

fn report_world_diagnostics(world: &WorldPack) {
    for diagnostic in world.diagnostics() {
        match diagnostic {
            WorldPackDiagnostic::DirectedUnreachableRoom {
                room_id,
                start_room_id,
            } => eprintln!(
                "warning: room {room_id:?} is not reachable through directed exits from start room {start_room_id:?}; review scripted, portal, admin, or cross-area access"
            ),
        }
    }
}

#[cfg(feature = "editorial-ai")]
fn run_ai_draft<F>(arguments: AiDraftArgs, suggest: F) -> Result<(), CliError>
where
    F: FnOnce(
        ProviderConfig,
        AdapterLimits,
        &EditorialRequest,
    ) -> Result<EditorialSuggestion, EditorialError>,
{
    ensure_new_output(&arguments.output)?;
    let input = read_bounded_utf8(&arguments.input, MAX_INPUT_BYTES)?;
    let context = arguments
        .context
        .as_deref()
        .map(|path| read_bounded_utf8(path, MAX_INPUT_BYTES))
        .transpose()?;
    let input_sha256 = sha256_hex(input.as_bytes());
    let context_sha256 = context.as_deref().map(str::as_bytes).map(sha256_hex);
    let request = EditorialRequest {
        task: arguments.task.into(),
        input,
        context,
        max_output_chars: usize::try_from(arguments.max_output_chars).map_err(|_| {
            CliError::InvalidAiConfiguration("max output character limit is invalid".to_owned())
        })?,
    };
    let config = provider_config(&arguments)?;
    let timeout = Duration::from_secs(arguments.timeout_seconds);
    let connect_timeout = Duration::from_secs(arguments.connect_timeout_seconds);
    let max_response_bytes = usize::try_from(arguments.max_response_bytes).map_err(|_| {
        CliError::InvalidAiConfiguration("maximum response size is invalid".to_owned())
    })?;
    if timeout.is_zero()
        || timeout > MAX_TIMEOUT
        || connect_timeout.is_zero()
        || connect_timeout > MAX_CONNECT_TIMEOUT
        || max_response_bytes == 0
        || max_response_bytes > MAX_RESPONSE_BYTES
        || request.max_output_chars == 0
        || request.max_output_chars > MAX_OUTPUT_CHARS
    {
        return Err(CliError::InvalidAiConfiguration(
            "editorial limits must be non-zero and within the compiled maxima".to_owned(),
        ));
    }
    let limits = AdapterLimits {
        timeout,
        connect_timeout,
        max_response_bytes,
    };
    let suggestion = suggest(config, limits, &request)?;
    let artifact = EditorialReviewArtifact {
        schema_version: EDITORIAL_ARTIFACT_SCHEMA,
        kind: EDITORIAL_ARTIFACT_KIND.to_owned(),
        state: EditorialArtifactState::Draft,
        suggestion,
        input_sha256,
        context_sha256,
    };
    validate_editorial_artifact(&artifact)?;
    write_editorial_artifact(&arguments.output, &artifact)?;
    println!(
        "wrote unreviewed authoring draft to {:?}; inspect it, then use ai-review",
        arguments.output
    );
    Ok(())
}

#[cfg(feature = "editorial-ai")]
fn run_ai_review(arguments: AiReviewArgs) -> Result<(), CliError> {
    ensure_new_output(&arguments.output)?;
    let bytes = read_bounded(&arguments.draft, MAX_EDITORIAL_ARTIFACT_BYTES)?;
    let mut artifact: EditorialReviewArtifact =
        serde_json::from_slice(&bytes).map_err(|source| CliError::EditorialJson {
            path: arguments.draft.clone(),
            source,
        })?;
    validate_editorial_artifact(&artifact)?;
    if artifact.state != EditorialArtifactState::Draft {
        return Err(CliError::InvalidEditorialArtifact(
            "only an unreviewed draft can be reviewed".to_owned(),
        ));
    }
    artifact.suggestion = artifact.suggestion.review(
        &arguments.reviewer,
        &arguments.reviewed_at,
        arguments.decision.accepted(),
    )?;
    artifact.state = EditorialArtifactState::Reviewed;
    validate_editorial_artifact(&artifact)?;
    write_editorial_artifact(&arguments.output, &artifact)?;
    let decision = if arguments.decision.accepted() {
        "accepted"
    } else {
        "rejected"
    };
    println!(
        "wrote {decision} authoring review to {:?}; no WorldPack was changed or published",
        arguments.output
    );
    Ok(())
}

#[cfg(feature = "editorial-ai")]
fn provider_config(arguments: &AiDraftArgs) -> Result<ProviderConfig, CliError> {
    let endpoint = || {
        arguments.endpoint.clone().ok_or_else(|| {
            CliError::InvalidAiConfiguration("--endpoint is required for this provider".to_owned())
        })
    };
    let model = || {
        arguments.model.clone().ok_or_else(|| {
            CliError::InvalidAiConfiguration("--model is required for this provider".to_owned())
        })
    };
    match arguments.provider {
        CliAiProvider::None => {
            if arguments.endpoint.is_some()
                || arguments.model.is_some()
                || arguments.authorization_env.is_some()
            {
                return Err(CliError::InvalidAiConfiguration(
                    "provider none does not accept endpoint, model, or authorization".to_owned(),
                ));
            }
            Ok(ProviderConfig::None)
        }
        CliAiProvider::Ollama => {
            reject_authorization_for_local_provider(arguments)?;
            Ok(ProviderConfig::Ollama {
                endpoint: endpoint()?,
                model: model()?,
            })
        }
        CliAiProvider::LlamaCpp => {
            reject_authorization_for_local_provider(arguments)?;
            Ok(ProviderConfig::LlamaCpp {
                endpoint: endpoint()?,
                model: model()?,
            })
        }
        CliAiProvider::Compatible => Ok(ProviderConfig::Compatible {
            endpoint: endpoint()?,
            model: model()?,
            authorization: arguments
                .authorization_env
                .as_deref()
                .map(read_authorization)
                .transpose()?,
        }),
    }
}

#[cfg(feature = "editorial-ai")]
fn reject_authorization_for_local_provider(arguments: &AiDraftArgs) -> Result<(), CliError> {
    if arguments.authorization_env.is_some() {
        Err(CliError::InvalidAiConfiguration(
            "--authorization-env is only supported by the compatible provider".to_owned(),
        ))
    } else {
        Ok(())
    }
}

#[cfg(feature = "editorial-ai")]
fn read_authorization(variable: &str) -> Result<String, CliError> {
    if variable.is_empty()
        || variable.len() > 128
        || !variable
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(CliError::InvalidAiConfiguration(
            "authorization environment variable name is invalid".to_owned(),
        ));
    }
    let value =
        env::var(variable).map_err(|_| CliError::MissingAuthorization(variable.to_owned()))?;
    if value.trim().is_empty() {
        return Err(CliError::MissingAuthorization(variable.to_owned()));
    }
    Ok(value)
}

#[cfg(feature = "editorial-ai")]
fn ensure_new_output(path: &Path) -> Result<(), CliError> {
    if path.exists() {
        Err(CliError::OutputExists(path.to_owned()))
    } else {
        Ok(())
    }
}

#[cfg(feature = "editorial-ai")]
fn read_bounded_utf8(path: &Path, maximum: usize) -> Result<String, CliError> {
    let bytes = read_bounded(path, maximum)?;
    String::from_utf8(bytes).map_err(|_| CliError::InvalidEditorialText(path.to_owned()))
}

#[cfg(feature = "editorial-ai")]
fn read_bounded(path: &Path, maximum: usize) -> Result<Vec<u8>, CliError> {
    let file = File::open(path).map_err(|source| CliError::Read {
        path: path.to_owned(),
        source,
    })?;
    let mut bytes = Vec::new();
    file.take(maximum as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| CliError::Read {
            path: path.to_owned(),
            source,
        })?;
    if bytes.len() > maximum {
        return Err(CliError::TooLarge {
            path: path.to_owned(),
            size: bytes.len() as u64,
            maximum: maximum as u64,
        });
    }
    Ok(bytes)
}

#[cfg(feature = "editorial-ai")]
fn write_editorial_artifact(
    path: &Path,
    artifact: &EditorialReviewArtifact,
) -> Result<(), CliError> {
    let mut bytes =
        serde_json::to_vec_pretty(artifact).map_err(|source| CliError::EditorialJson {
            path: path.to_owned(),
            source,
        })?;
    bytes.push(b'\n');
    write_artifact(path, &bytes, false)
}

#[cfg(feature = "editorial-ai")]
fn validate_editorial_artifact(artifact: &EditorialReviewArtifact) -> Result<(), CliError> {
    if artifact.schema_version != EDITORIAL_ARTIFACT_SCHEMA
        || artifact.kind != EDITORIAL_ARTIFACT_KIND
    {
        return Err(CliError::InvalidEditorialArtifact(
            "unsupported schema or artifact kind".to_owned(),
        ));
    }
    let review_matches_state = match artifact.state {
        EditorialArtifactState::Draft => artifact.suggestion.human_review.is_none(),
        EditorialArtifactState::Reviewed => artifact.suggestion.human_review.is_some(),
    };
    if !review_matches_state {
        return Err(CliError::InvalidEditorialArtifact(
            "artifact state does not match its human review".to_owned(),
        ));
    }
    let suggestion = &artifact.suggestion;
    if suggestion.text.trim().is_empty()
        || suggestion.text.chars().count() > MAX_OUTPUT_CHARS
        || !bounded_artifact_field(&suggestion.provider, 256)
        || !bounded_artifact_field(&suggestion.model, 256)
        || !bounded_artifact_field(&suggestion.provenance, 2_048)
        || !valid_digest(&artifact.input_sha256)
        || artifact
            .context_sha256
            .as_deref()
            .is_some_and(|digest| !valid_digest(digest))
    {
        return Err(CliError::InvalidEditorialArtifact(
            "suggestion fields or source digests are invalid".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(feature = "editorial-ai")]
fn bounded_artifact_field(value: &str, maximum: usize) -> bool {
    !value.trim().is_empty() && value.len() <= maximum && !value.chars().any(char::is_control)
}

#[cfg(feature = "editorial-ai")]
fn valid_digest(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn load_document(path: &Path) -> Result<WorldPackDocument, CliError> {
    let metadata = fs::metadata(path).map_err(|source| CliError::Inspect {
        path: path.to_owned(),
        source,
    })?;
    if metadata.len() > MAX_DOCUMENT_BYTES {
        return Err(CliError::TooLarge {
            path: path.to_owned(),
            size: metadata.len(),
            maximum: MAX_DOCUMENT_BYTES,
        });
    }
    let bytes = fs::read(path).map_err(|source| CliError::Read {
        path: path.to_owned(),
        source,
    })?;
    WorldPackDocument::from_json(&bytes).map_err(|source| CliError::Document {
        path: path.to_owned(),
        source,
    })
}

fn validate_document(path: &Path, document: &WorldPackDocument) -> Result<(), CliError> {
    document.validate().map_err(|source| CliError::Document {
        path: path.to_owned(),
        source: source.into(),
    })
}

fn validate_document_assets(path: &Path, document: &WorldPackDocument) -> Result<(), CliError> {
    let media_by_room = document
        .room_media
        .iter()
        .map(|media| (media.room_id.as_str(), media))
        .collect::<HashMap<_, _>>();
    for room in &document.world.rooms {
        if let Some(manifest) = &room.asset_manifest {
            let alt = media_by_room
                .get(room.id.as_str())
                .and_then(|media| english_text(&media.image_alt))
                .expect("document validation requires localized alt for image manifests");
            validate_image_manifest(path, manifest, alt)?;
        }
        if let Some(audio) = media_by_room
            .get(room.id.as_str())
            .and_then(|media| media.audio.as_ref())
        {
            validate_asset_exists(path, &audio.path)?;
        }
    }
    for set in &document.scene_variants {
        for variant in &set.variants {
            if let Some(manifest) = &variant.asset_manifest {
                let alt = english_text(&variant.image_alt).expect(
                    "document validation requires localized alt for variant image manifests",
                );
                validate_image_manifest(path, manifest, alt)?;
            }
            if let Some(audio) = &variant.audio {
                validate_asset_exists(path, &audio.path)?;
            }
        }
    }
    for media in &document.entity_media {
        let sources = std::iter::once(&media.poses.idle)
            .chain(media.poses.attack.iter())
            .chain(media.poses.hit.iter())
            .chain(media.poses.defeated.iter());
        for source in sources {
            for relative in [source.webp.as_deref(), source.png.as_deref()]
                .into_iter()
                .flatten()
            {
                validate_asset_exists(path, relative)?;
            }
        }
    }
    Ok(())
}

fn english_text(localized: &world_pack::LocalizedText) -> Option<&str> {
    localized.resolve("en")
}

fn validate_image_manifest(
    input: &Path,
    relative: &str,
    expected_alt: &str,
) -> Result<(), CliError> {
    let manifest_path = locate_asset(input, relative)?;
    let bytes = fs::read(&manifest_path).map_err(|source| CliError::Read {
        path: manifest_path.clone(),
        source,
    })?;
    if bytes.len() > 256 * 1024 {
        return Err(CliError::Asset {
            path: manifest_path,
            reason: "image manifest exceeds 256 KiB".into(),
        });
    }
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|error| CliError::Asset {
            path: manifest_path.clone(),
            reason: format!("invalid JSON: {error}"),
        })?;
    let manifest_alt = value.get("alt").and_then(serde_json::Value::as_str);
    if manifest_alt != Some(expected_alt) {
        return Err(CliError::Asset {
            path: manifest_path,
            reason: "English imageAlt must exactly match the reviewed manifest alt".into(),
        });
    }
    Ok(())
}

fn validate_asset_exists(input: &Path, relative: &str) -> Result<(), CliError> {
    locate_asset(input, relative).map(|_| ())
}

fn locate_asset(input: &Path, relative: &str) -> Result<PathBuf, CliError> {
    if let Ok(current) = std::env::current_dir() {
        for ancestor in current.ancestors() {
            let workspace_asset = ancestor.join("apps/web/public").join(relative);
            if workspace_asset.is_file() {
                return Ok(workspace_asset);
            }
        }
    }
    let base = input.parent().unwrap_or_else(|| Path::new("."));
    for ancestor in base.ancestors() {
        for root in [ancestor.to_owned(), ancestor.join("apps/web/public")] {
            let candidate = root.join(relative);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(CliError::Asset {
        path: PathBuf::from(relative),
        reason: "referenced immutable asset does not exist under the document or web public root"
            .into(),
    })
}

fn write_artifact(path: &Path, bytes: &[u8], force: bool) -> Result<(), CliError> {
    let mut options = OpenOptions::new();
    options.write(true);
    if force {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }
    let mut file = options.open(path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::AlreadyExists && !force {
            CliError::OutputExists(path.to_owned())
        } else {
            CliError::Write {
                path: path.to_owned(),
                source,
            }
        }
    })?;
    file.write_all(bytes).map_err(|source| CliError::Write {
        path: path.to_owned(),
        source,
    })?;
    file.sync_all().map_err(|source| CliError::Write {
        path: path.to_owned(),
        source,
    })
}

/// Publish a complete new file in one filesystem operation. The destination is
/// never opened for writing, so a failed composition/write cannot expose a
/// truncated runtime artifact and an existing output is never replaced.
fn write_artifact_atomically(path: &Path, bytes: &[u8]) -> Result<(), CliError> {
    if path.exists() {
        return Err(CliError::OutputExists(path.to_owned()));
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("world.json");
    let mut temporary = None;
    for nonce in 0..128_u16 {
        let candidate = parent.join(format!(
            ".{file_name}.{}.{}.part",
            std::process::id(),
            nonce
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        match options.open(&candidate) {
            Ok(file) => {
                temporary = Some((candidate, file));
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(CliError::Write {
                    path: candidate,
                    source,
                });
            }
        }
    }
    let Some((temporary_path, mut file)) = temporary else {
        return Err(CliError::Write {
            path: path.to_owned(),
            source: std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "could not reserve a temporary output path",
            ),
        });
    };
    let result = (|| {
        file.write_all(bytes).map_err(|source| CliError::Write {
            path: temporary_path.clone(),
            source,
        })?;
        file.sync_all().map_err(|source| CliError::Write {
            path: temporary_path.clone(),
            source,
        })?;
        drop(file);
        fs::hard_link(&temporary_path, path).map_err(|source| {
            if source.kind() == std::io::ErrorKind::AlreadyExists {
                CliError::OutputExists(path.to_owned())
            } else {
                CliError::Write {
                    path: path.to_owned(),
                    source,
                }
            }
        })?;
        sync_directory(parent)?;
        Ok(())
    })();
    let _ = fs::remove_file(&temporary_path);
    result
}

fn publish_area_release(
    project: &Path,
    release_path: &Path,
    registry: &Path,
) -> Result<(), CliError> {
    let composed = compose_world_project(project).map_err(|source| CliError::Composition {
        path: project.to_owned(),
        source: Box::new(source),
    })?;
    let release_bytes = read_bounded_regular_file(
        release_path,
        MAX_AREA_RELEASE_INPUT_BYTES,
        "area release manifest",
    )?;
    let release = parse_area_release_manifest(&release_bytes)?;
    // This advisory lock serializes cooperating local `tiamud-world`
    // publishers for one world. It intentionally is not a security boundary
    // against a process that ignores locks or can mutate the registry.
    let lock_path = area_publication_lock_path(registry, &composed.report.project_id);
    let lock_file = open_area_publication_lock(registry, &lock_path)?;
    let mut publication_lock = RwLock::new(lock_file);
    let _publication_guard =
        publication_lock
            .write()
            .map_err(|source| CliError::AreaPublicationLock {
                path: lock_path,
                source,
            })?;
    let previous = load_active_area_catalog(registry, &composed.report.project_id)?;
    let verified = build_area_release(&composed, &release, previous.as_ref())?;
    verify_area_release(
        &verified.catalog,
        &verified.bundles,
        &verified.world_artifact,
    )?;

    let world_path = area_world_blob_path(
        registry,
        &verified.catalog.world_id,
        &verified.catalog.world_artifact_sha256,
    );
    write_immutable_area_blob(
        registry,
        &world_path,
        &verified.world_artifact,
        &verified.catalog.world_artifact_sha256,
        MAX_AREA_WORLD_INPUT_BYTES,
    )?;
    for (area_id, bytes) in &verified.bundles {
        let entry = verified
            .catalog
            .entries
            .iter()
            .find(|entry| &entry.area_id == area_id)
            .ok_or_else(|| {
                CliError::InvalidAreaRegistry(format!(
                    "verified bundle {area_id:?} has no catalog entry"
                ))
            })?;
        let path = area_bundle_blob_path(
            registry,
            &verified.catalog.world_id,
            area_id,
            &entry.bundle_sha256,
        );
        write_immutable_area_blob(
            registry,
            &path,
            bytes,
            &entry.bundle_sha256,
            MAX_AREA_BUNDLE_INPUT_BYTES,
        )?;
    }
    let catalog_path = area_catalog_blob_path(
        registry,
        &verified.catalog.world_id,
        &verified.catalog_sha256,
    );
    write_immutable_area_blob(
        registry,
        &catalog_path,
        &verified.catalog_bytes,
        &verified.catalog_sha256,
        MAX_AREA_CATALOG_INPUT_BYTES,
    )?;

    // Re-open every immutable blob through the same bounded, no-symlink path
    // used by `area-verify`. The active pointer is the final durable write.
    let (stored, stored_digest) = load_verified_area_catalog(&catalog_path, registry)?;
    if stored != verified.catalog || stored_digest != verified.catalog_sha256 {
        return Err(CliError::InvalidAreaRegistry(
            "stored catalog differs from the verified release".into(),
        ));
    }
    activate_area_catalog(registry, &stored, &stored_digest, &catalog_path)?;
    println!(
        "{}",
        area_publish_success_message(&stored, &stored_digest, &catalog_path)
    );
    Ok(())
}

fn area_publish_success_message(catalog: &AreaCatalog, digest: &str, path: &Path) -> String {
    format!(
        "published area catalog {:?} revision {} as sha256:{} with {} immutable bundle(s) at {:?}",
        catalog.world_id,
        catalog.catalog_revision,
        digest,
        catalog.entries.len(),
        path
    )
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum AreaSelectionKind {
    Initial,
    Activate,
    Rollback,
    Noop,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct AreaSelection {
    kind: AreaSelectionKind,
    world_id: String,
    catalog_revision: u64,
    catalog_sha256: String,
    previous_revision: Option<u64>,
    catalog_path: PathBuf,
}

fn select_area_catalog(catalog_path: &Path, registry: &Path) -> Result<AreaSelection, CliError> {
    select_area_catalog_with_observer(catalog_path, registry, || {})
}

fn select_area_catalog_with_observer(
    catalog_path: &Path,
    registry: &Path,
    after_candidate_verified: impl FnOnce(),
) -> Result<AreaSelection, CliError> {
    // Verify before waiting for the per-world lock so malformed or incomplete
    // releases never occupy the publication critical section.
    let observed = LocalAreaGeneration::open_exact(registry, catalog_path)
        .map_err(map_local_area_registry_error)?;
    observed
        .verify_complete_release()
        .map_err(map_local_area_registry_error)?;
    let world_id = observed.catalog().world_id.clone();
    let observed_digest = observed.generation_id().to_owned();
    after_candidate_verified();

    // `area-select` and `area-publish` are cooperating writers of the same
    // mutable editorial pointer. Re-open both the candidate and current
    // pointer only after this lock is held so a queued selector cannot act on
    // stale bytes or revision state.
    let lock_path = area_publication_lock_path(registry, &world_id);
    let lock_file = open_area_publication_lock(registry, &lock_path)?;
    let mut publication_lock = RwLock::new(lock_file);
    let _publication_guard =
        publication_lock
            .write()
            .map_err(|source| CliError::AreaPublicationLock {
                path: lock_path,
                source,
            })?;

    let candidate = LocalAreaGeneration::open_exact(registry, catalog_path)
        .map_err(map_local_area_registry_error)?;
    candidate
        .verify_complete_release()
        .map_err(map_local_area_registry_error)?;
    if candidate.catalog().world_id != world_id || candidate.generation_id() != observed_digest {
        return Err(CliError::InvalidAreaRegistry(
            "area catalog changed while selection waited for its publication lock".into(),
        ));
    }

    let current = load_active_area_generation(registry, &world_id)?;
    let previous_revision = current
        .as_ref()
        .map(|generation| generation.catalog().catalog_revision);
    let kind = match current.as_ref() {
        None => AreaSelectionKind::Initial,
        Some(current) if current.generation_id() == candidate.generation_id() => {
            AreaSelectionKind::Noop
        }
        Some(current) => match candidate
            .catalog()
            .catalog_revision
            .cmp(&current.catalog().catalog_revision)
        {
            std::cmp::Ordering::Greater => AreaSelectionKind::Activate,
            std::cmp::Ordering::Less => AreaSelectionKind::Rollback,
            std::cmp::Ordering::Equal => {
                return Err(CliError::InvalidAreaRegistry(
                    "different area catalogs reuse one catalog revision; activation direction is ambiguous"
                        .into(),
                ));
            }
        },
    };
    let selection = AreaSelection {
        kind,
        world_id,
        catalog_revision: candidate.catalog().catalog_revision,
        catalog_sha256: candidate.generation_id().to_owned(),
        previous_revision,
        catalog_path: catalog_path.to_owned(),
    };
    if kind != AreaSelectionKind::Noop {
        activate_area_catalog(
            registry,
            candidate.catalog(),
            candidate.generation_id(),
            catalog_path,
        )?;
    }
    Ok(selection)
}

fn area_selection_success_message(selection: &AreaSelection) -> String {
    match selection.kind {
        AreaSelectionKind::Initial => format!(
            "selected initial area catalog {:?} revision {} as sha256:{} at {:?}",
            selection.world_id,
            selection.catalog_revision,
            selection.catalog_sha256,
            selection.catalog_path
        ),
        AreaSelectionKind::Activate => format!(
            "selected area catalog {:?} for activation from revision {} to {} as sha256:{} at {:?}",
            selection.world_id,
            selection
                .previous_revision
                .expect("activation has a prior revision"),
            selection.catalog_revision,
            selection.catalog_sha256,
            selection.catalog_path
        ),
        AreaSelectionKind::Rollback => format!(
            "selected area catalog {:?} for rollback from revision {} to {} as sha256:{} at {:?}",
            selection.world_id,
            selection
                .previous_revision
                .expect("rollback has a prior revision"),
            selection.catalog_revision,
            selection.catalog_sha256,
            selection.catalog_path
        ),
        AreaSelectionKind::Noop => format!(
            "area catalog {:?} revision {} sha256:{} is already selected (noop)",
            selection.world_id, selection.catalog_revision, selection.catalog_sha256
        ),
    }
}

fn load_active_area_catalog(
    registry: &Path,
    world_id: &str,
) -> Result<Option<AreaCatalog>, CliError> {
    Ok(load_active_area_generation(registry, world_id)?
        .map(|generation| generation.catalog().clone()))
}

fn load_active_area_generation(
    registry: &Path,
    world_id: &str,
) -> Result<Option<LocalAreaGeneration>, CliError> {
    let pointer_path = local_active_area_catalog_path(registry, world_id);
    match fs::symlink_metadata(&pointer_path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(CliError::Read {
                path: pointer_path,
                source,
            });
        }
        Ok(_) => {}
    }
    let generation = LocalAreaGeneration::open_active(registry, world_id)
        .map_err(map_local_area_registry_error)?;
    generation
        .verify_complete_release()
        .map_err(map_local_area_registry_error)?;
    generation
        .revalidate_active_pointer_identity()
        .map_err(map_local_area_registry_error)?;
    Ok(Some(generation))
}

fn load_verified_area_catalog(
    catalog_path: &Path,
    registry: &Path,
) -> Result<(AreaCatalog, String), CliError> {
    let generation = LocalAreaGeneration::open_exact(registry, catalog_path)
        .map_err(map_local_area_registry_error)?;
    generation
        .verify_complete_release()
        .map_err(map_local_area_registry_error)?;
    Ok((
        generation.catalog().clone(),
        generation.generation_id().to_owned(),
    ))
}

fn map_local_area_registry_error(error: LocalAreaRegistryError) -> CliError {
    match error {
        LocalAreaRegistryError::Inspect { path, source } => CliError::Inspect { path, source },
        LocalAreaRegistryError::Read { path, source } => CliError::Read { path, source },
        LocalAreaRegistryError::TooLarge {
            path,
            size,
            maximum,
            ..
        } => CliError::TooLarge {
            path,
            size,
            maximum,
        },
        LocalAreaRegistryError::Invalid(reason) => CliError::InvalidAreaRegistry(reason),
        LocalAreaRegistryError::PublicationLockBusy => {
            CliError::InvalidAreaRegistry("area publication lock is busy".into())
        }
        LocalAreaRegistryError::Catalog(error) => CliError::AreaCatalog(error),
    }
}

fn activate_area_catalog(
    registry: &Path,
    catalog: &AreaCatalog,
    digest: &str,
    catalog_path: &Path,
) -> Result<(), CliError> {
    validate_digest(digest)?;
    let pointer_path = local_active_area_catalog_path(registry, &catalog.world_id);
    let parent = pointer_path
        .parent()
        .expect("active area catalog pointer has a parent");
    ensure_area_registry_directory(registry, parent)?;
    let pointer = ActiveAreaCatalog {
        schema: 1,
        world_id: catalog.world_id.clone(),
        catalog_sha256: digest.to_owned(),
        catalog: registry_relative_path(registry, catalog_path)?,
    };
    let mut bytes = serde_json::to_vec_pretty(&pointer).map_err(|error| {
        CliError::InvalidAreaRegistry(format!("could not encode active catalog pointer: {error}"))
    })?;
    bytes.push(b'\n');
    if bytes.len() > MAX_LOCAL_AREA_POINTER_BYTES {
        return Err(CliError::InvalidAreaRegistry(
            "active catalog pointer exceeds its byte limit".into(),
        ));
    }
    let file_name = pointer_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("active-area-catalog");
    let (temporary, mut file) = reserve_temporary(parent, file_name)?;
    let result = (|| {
        file.write_all(&bytes).map_err(|source| CliError::Write {
            path: temporary.clone(),
            source,
        })?;
        file.sync_all().map_err(|source| CliError::Write {
            path: temporary.clone(),
            source,
        })?;
        drop(file);
        fs::rename(&temporary, &pointer_path).map_err(|source| CliError::Write {
            path: pointer_path.clone(),
            source,
        })?;
        sync_directory(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn area_catalog_blob_path(registry: &Path, world_id: &str, digest: &str) -> PathBuf {
    local_area_catalog_path(registry, world_id, digest)
}

fn area_bundle_blob_path(registry: &Path, world_id: &str, area_id: &str, digest: &str) -> PathBuf {
    local_area_bundle_path(registry, world_id, area_id, digest)
}

fn area_world_blob_path(registry: &Path, world_id: &str, digest: &str) -> PathBuf {
    local_area_world_path(registry, world_id, digest)
}

fn area_publication_lock_path(registry: &Path, world_id: &str) -> PathBuf {
    world_pack::local_area_publication_lock_path(registry, world_id)
}

fn open_area_publication_lock(registry: &Path, path: &Path) -> Result<fs::File, CliError> {
    let parent = path
        .parent()
        .ok_or_else(|| CliError::InvalidAreaRegistry("area lock has no parent".into()))?;
    ensure_area_registry_directory(registry, parent)?;

    let mut create = OpenOptions::new();
    create.read(true).write(true).create_new(true);
    match create.open(path) {
        Ok(file) => {
            file.sync_all().map_err(|source| CliError::Write {
                path: path.to_owned(),
                source,
            })?;
            sync_directory(parent)?;
            Ok(file)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let before = fs::symlink_metadata(path).map_err(|source| CliError::Read {
                path: path.to_owned(),
                source,
            })?;
            if before.file_type().is_symlink() || !before.file_type().is_file() {
                return Err(CliError::InvalidAreaRegistry(
                    "area publication lock is not a regular non-symlink file".into(),
                ));
            }
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .map_err(|source| CliError::Read {
                    path: path.to_owned(),
                    source,
                })?;
            let opened = file.metadata().map_err(|source| CliError::Read {
                path: path.to_owned(),
                source,
            })?;
            if !opened.file_type().is_file() {
                return Err(CliError::InvalidAreaRegistry(
                    "area publication lock changed while it was opened".into(),
                ));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if before.dev() != opened.dev() || before.ino() != opened.ino() {
                    return Err(CliError::InvalidAreaRegistry(
                        "area publication lock changed while it was opened".into(),
                    ));
                }
            }
            Ok(file)
        }
        Err(source) => Err(CliError::Write {
            path: path.to_owned(),
            source,
        }),
    }
}

fn write_immutable_area_blob(
    registry: &Path,
    path: &Path,
    bytes: &[u8],
    expected_digest: &str,
    maximum: usize,
) -> Result<(), CliError> {
    validate_digest(expected_digest)?;
    if bytes.len() > maximum || sha256_hex(bytes) != expected_digest {
        return Err(CliError::InvalidAreaRegistry(
            "immutable area blob violates its size or digest contract".into(),
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| CliError::InvalidAreaRegistry("area blob has no parent".into()))?;
    ensure_area_registry_directory(registry, parent)?;
    match write_artifact_atomically(path, bytes) {
        Ok(()) => Ok(()),
        Err(CliError::OutputExists(_)) => {
            let existing = read_bounded_registry_file(
                registry,
                path,
                maximum,
                "existing immutable area blob",
            )?;
            if existing == bytes {
                sync_existing_registry_file(registry, path, "existing immutable area blob")?;
                Ok(())
            } else {
                Err(CliError::PublishedDigestMismatch)
            }
        }
        Err(error) => Err(error),
    }
}

fn ensure_area_registry_directory(registry: &Path, directory: &Path) -> Result<(), CliError> {
    if !directory.starts_with(registry) {
        return Err(CliError::InvalidAreaRegistry(
            "area registry path escapes its configured root".into(),
        ));
    }
    ensure_directory_without_symlink(registry)?;
    let relative = directory.strip_prefix(registry).map_err(|_| {
        CliError::InvalidAreaRegistry("area registry path escapes its configured root".into())
    })?;
    let mut current = registry.to_owned();
    for component in relative.components() {
        let std::path::Component::Normal(component) = component else {
            return Err(CliError::InvalidAreaRegistry(
                "area registry path contains a traversal component".into(),
            ));
        };
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            }
            Ok(_) => {
                return Err(CliError::InvalidAreaRegistry(format!(
                    "registry component {:?} is not a real directory",
                    current
                )));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match fs::create_dir(&current) {
                    Ok(()) => {
                        let parent = current.parent().unwrap_or(registry);
                        sync_directory(parent)?;
                        sync_directory(&current)?;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                        ensure_directory_without_symlink(&current)?;
                    }
                    Err(source) => {
                        return Err(CliError::Write {
                            path: current,
                            source,
                        });
                    }
                }
            }
            Err(source) => {
                return Err(CliError::Write {
                    path: current,
                    source,
                });
            }
        }
    }
    Ok(())
}

fn ensure_directory_without_symlink(path: &Path) -> Result<(), CliError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            Ok(())
        }
        Ok(_) => Err(CliError::InvalidAreaRegistry(format!(
            "registry component {path:?} is not a real directory"
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            create_dir_all_durable(path)?;
            let metadata = fs::symlink_metadata(path).map_err(|source| CliError::Read {
                path: path.to_owned(),
                source,
            })?;
            if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
                Ok(())
            } else {
                Err(CliError::InvalidAreaRegistry(format!(
                    "registry component {path:?} is not a real directory"
                )))
            }
        }
        Err(source) => Err(CliError::Read {
            path: path.to_owned(),
            source,
        }),
    }
}

fn read_bounded_registry_file(
    registry: &Path,
    path: &Path,
    maximum: usize,
    kind: &'static str,
) -> Result<Vec<u8>, CliError> {
    read_bounded_local_registry_file(registry, path, maximum, kind)
        .map_err(map_local_area_registry_error)
}

fn ensure_existing_registry_directory(registry: &Path, directory: &Path) -> Result<(), CliError> {
    verify_existing_registry_directory(registry, directory).map_err(map_local_area_registry_error)
}

fn sync_existing_registry_file(
    registry: &Path,
    path: &Path,
    kind: &'static str,
) -> Result<(), CliError> {
    let parent = path
        .parent()
        .ok_or_else(|| CliError::InvalidAreaRegistry(format!("{kind} has no parent")))?;
    ensure_existing_registry_directory(registry, parent)?;
    let before = fs::symlink_metadata(path).map_err(|source| CliError::Read {
        path: path.to_owned(),
        source,
    })?;
    if before.file_type().is_symlink() || !before.file_type().is_file() {
        return Err(CliError::InvalidAreaRegistry(format!(
            "{kind} is not a regular non-symlink file"
        )));
    }
    let file = OpenOptions::new()
        .read(true)
        .open(path)
        .map_err(|source| CliError::Read {
            path: path.to_owned(),
            source,
        })?;
    let opened = file.metadata().map_err(|source| CliError::Read {
        path: path.to_owned(),
        source,
    })?;
    if !opened.file_type().is_file() {
        return Err(CliError::InvalidAreaRegistry(format!(
            "{kind} changed while it was opened"
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.dev() != opened.dev() || before.ino() != opened.ino() {
            return Err(CliError::InvalidAreaRegistry(format!(
                "{kind} changed while it was opened"
            )));
        }
    }
    file.sync_all().map_err(|source| CliError::Write {
        path: path.to_owned(),
        source,
    })?;
    sync_directory(parent)
}

fn read_bounded_regular_file(
    path: &Path,
    maximum: usize,
    kind: &'static str,
) -> Result<Vec<u8>, CliError> {
    read_bounded_local_file(path, maximum, kind).map_err(map_local_area_registry_error)
}

fn registry_relative_path(registry: &Path, path: &Path) -> Result<String, CliError> {
    let relative = path.strip_prefix(registry).map_err(|_| {
        CliError::InvalidAreaRegistry("registry pointer path escapes its root".into())
    })?;
    if relative
        .components()
        .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(CliError::InvalidAreaRegistry(
            "registry pointer path contains traversal".into(),
        ));
    }
    Ok(relative.to_string_lossy().replace('\\', "/"))
}

fn world_storage_id(world_id: &str) -> String {
    sha256_hex(world_id.as_bytes())
}

fn publication_path(registry: &Path, world_id: &str, digest: &str) -> PathBuf {
    registry
        .join("packs")
        .join(world_storage_id(world_id))
        .join(format!("{digest}.world.json"))
}

fn active_pointer_path(registry: &Path, world_id: &str) -> PathBuf {
    registry
        .join("active")
        .join(format!("{}.json", world_storage_id(world_id)))
}

fn write_immutable_artifact(path: &Path, bytes: &[u8]) -> Result<(), CliError> {
    let parent = path.parent().expect("publication path has a parent");
    create_dir_all_durable(parent)?;
    match write_artifact_atomically(path, bytes) {
        Ok(()) => Ok(()),
        Err(CliError::OutputExists(_)) => {
            let existing = fs::read(path).map_err(|source| CliError::Read {
                path: path.to_owned(),
                source,
            })?;
            if existing == bytes {
                Ok(())
            } else {
                Err(CliError::PublishedDigestMismatch)
            }
        }
        Err(error) => Err(error),
    }
}

fn activate_publication(
    registry: &Path,
    world_id: &str,
    digest: &str,
    artifact_path: &Path,
) -> Result<(), CliError> {
    validate_digest(digest)?;
    let pointer_path = active_pointer_path(registry, world_id);
    let parent = pointer_path.parent().expect("active pointer has a parent");
    create_dir_all_durable(parent)?;
    let relative = artifact_path
        .strip_prefix(registry)
        .map_err(|_| CliError::WrongPublishedWorld(world_id.to_owned()))?;
    let pointer = ActivePublication {
        schema: 1,
        world_id: world_id.to_owned(),
        sha256: digest.to_owned(),
        artifact: relative.to_string_lossy().replace('\\', "/"),
    };
    let mut bytes = serde_json::to_vec_pretty(&pointer).map_err(|source| CliError::Document {
        path: pointer_path.clone(),
        source: DocumentToolError::Json(source),
    })?;
    bytes.push(b'\n');
    let file_name = pointer_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("active");
    let (temporary, mut file) = reserve_temporary(parent, file_name)?;
    let result = (|| {
        file.write_all(&bytes).map_err(|source| CliError::Write {
            path: temporary.clone(),
            source,
        })?;
        file.sync_all().map_err(|source| CliError::Write {
            path: temporary.clone(),
            source,
        })?;
        drop(file);
        fs::rename(&temporary, &pointer_path).map_err(|source| CliError::Write {
            path: pointer_path.clone(),
            source,
        })?;
        sync_directory(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn reserve_temporary(parent: &Path, file_name: &str) -> Result<(PathBuf, fs::File), CliError> {
    for nonce in 0..128_u16 {
        let candidate = parent.join(format!(
            ".{file_name}.{}.{}.part",
            std::process::id(),
            nonce
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        match options.open(&candidate) {
            Ok(file) => return Ok((candidate, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(CliError::Write {
                    path: candidate,
                    source,
                });
            }
        }
    }
    Err(CliError::Write {
        path: parent.join(file_name),
        source: std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "could not reserve a unique temporary path",
        ),
    })
}

fn create_dir_all_durable(path: &Path) -> Result<(), CliError> {
    let missing = path
        .ancestors()
        .take_while(|candidate| !candidate.exists())
        .map(Path::to_owned)
        .collect::<Vec<_>>();
    fs::create_dir_all(path).map_err(|source| CliError::Write {
        path: path.to_owned(),
        source,
    })?;
    for directory in missing.iter().rev() {
        if let Some(parent) = directory.parent() {
            sync_directory(parent)?;
        }
        sync_directory(directory)?;
    }
    if missing.is_empty() {
        sync_directory(path)?;
    }
    Ok(())
}

fn sync_directory(path: &Path) -> Result<(), CliError> {
    let directory = fs::File::open(path).map_err(|source| CliError::Write {
        path: path.to_owned(),
        source,
    })?;
    directory.sync_all().map_err(|source| CliError::Write {
        path: path.to_owned(),
        source,
    })
}

fn validate_digest(digest: &str) -> Result<(), CliError> {
    if digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        Ok(())
    } else {
        Err(CliError::InvalidDigest)
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_command() {
        assert!(matches!(
            Cli::try_parse_from([
                "tiamud-world",
                "compose",
                "world.project.json",
                "--output",
                "world.json"
            ])
            .unwrap()
            .command,
            Command::Compose { .. }
        ));
        assert!(matches!(
            Cli::try_parse_from(["tiamud-world", "validate", "world.json"])
                .unwrap()
                .command,
            Command::Validate { .. }
        ));
        assert!(matches!(
            Cli::try_parse_from([
                "tiamud-world",
                "build",
                "world.json",
                "--output",
                "world.pack.json"
            ])
            .unwrap()
            .command,
            Command::Build { .. }
        ));
        assert!(matches!(
            Cli::try_parse_from(["tiamud-world", "diff", "old.json", "new.json"])
                .unwrap()
                .command,
            Command::Diff { .. }
        ));
        assert!(matches!(
            Cli::try_parse_from([
                "tiamud-world",
                "preview",
                "world.json",
                "--room",
                "demo:start",
                "--format",
                "ansi",
                "--time-period",
                "night",
                "--weather",
                "rain"
            ])
            .unwrap()
            .command,
            Command::Preview { .. }
        ));
        assert!(matches!(
            Cli::try_parse_from(["tiamud-world", "test", "world.json"])
                .unwrap()
                .command,
            Command::Test { .. }
        ));
        assert!(matches!(
            Cli::try_parse_from([
                "tiamud-world",
                "publish",
                "world.json",
                "--registry",
                "worlds"
            ])
            .unwrap()
            .command,
            Command::Publish { .. }
        ));
        assert!(matches!(
            Cli::try_parse_from([
                "tiamud-world",
                "rollback",
                "--registry",
                "worlds",
                "--world",
                "demo",
                "--sha256",
                &"a".repeat(64)
            ])
            .unwrap()
            .command,
            Command::Rollback { .. }
        ));
        assert!(matches!(
            Cli::try_parse_from([
                "tiamud-world",
                "area-publish",
                "world.project.json",
                "--release",
                "area-release.json",
                "--registry",
                "registry"
            ])
            .unwrap()
            .command,
            Command::AreaPublish { .. }
        ));
        assert!(matches!(
            Cli::try_parse_from([
                "tiamud-world",
                "area-verify",
                "--catalog",
                "registry/area-catalogs/world/catalog.json",
                "--registry",
                "registry"
            ])
            .unwrap()
            .command,
            Command::AreaVerify { .. }
        ));
        assert!(matches!(
            Cli::try_parse_from([
                "tiamud-world",
                "area-select",
                "--catalog",
                "registry/area-catalogs/world/catalog.json",
                "--registry",
                "registry"
            ])
            .unwrap()
            .command,
            Command::AreaSelect { .. }
        ));
        #[cfg(feature = "editorial-ai")]
        assert!(matches!(
            Cli::try_parse_from([
                "tiamud-world",
                "ai-draft",
                "--task",
                "suggest-alt-text",
                "--input",
                "prompt.txt",
                "--output",
                "draft.json"
            ])
            .unwrap()
            .command,
            Command::AiDraft(_)
        ));
        #[cfg(feature = "editorial-ai")]
        assert!(matches!(
            Cli::try_parse_from([
                "tiamud-world",
                "ai-review",
                "draft.json",
                "--reviewer",
                "Editor",
                "--reviewed-at",
                "2026-07-15T00:00:00Z",
                "--decision",
                "accept",
                "--output",
                "review.json"
            ])
            .unwrap()
            .command,
            Command::AiReview(_)
        ));
    }

    #[test]
    fn preview_rejects_out_of_range_hour() {
        assert!(
            Cli::try_parse_from([
                "tiamud-world",
                "preview",
                "world.json",
                "--room",
                "demo:start",
                "--hour",
                "24"
            ])
            .is_err()
        );
    }

    #[test]
    fn publish_is_immutable_and_rollback_only_switches_the_atomic_pointer() {
        let temp = tempfile::tempdir().unwrap();
        let registry = temp.path().join("registry");
        let input = temp.path().join("world.json");
        let document = |title: &str| {
            format!(
                r#"{{"schemaVersion":2,"id":"demo","name":"Demo","startRoomId":"demo:start","rooms":[{{"id":"demo:start","title":"{title}","description":"A room.","exits":[]}}]}}"#
            )
        };

        fs::write(&input, document("First")).unwrap();
        run(Cli {
            command: Command::Publish {
                input: input.clone(),
                registry: registry.clone(),
            },
        })
        .unwrap();
        let first_artifact = WorldPackDocument::from_json(document("First").as_bytes())
            .unwrap()
            .deterministic_artifact()
            .unwrap();
        let first_digest = sha256_hex(&first_artifact);

        let pointer_path = active_pointer_path(&registry, "demo");
        let pointer_name = pointer_path.file_name().unwrap().to_string_lossy();
        let stale_pointer = pointer_path
            .parent()
            .unwrap()
            .join(format!(".{pointer_name}.{}.0.part", std::process::id()));
        fs::write(&stale_pointer, b"orphaned previous attempt").unwrap();

        fs::write(&input, document("Second")).unwrap();
        run(Cli {
            command: Command::Publish {
                input,
                registry: registry.clone(),
            },
        })
        .unwrap();
        assert!(stale_pointer.exists());
        let first_path = publication_path(&registry, "demo", &first_digest);
        assert_eq!(fs::read(&first_path).unwrap(), first_artifact);

        run(Cli {
            command: Command::Rollback {
                registry: registry.clone(),
                world: "demo".to_owned(),
                sha256: first_digest.clone(),
            },
        })
        .unwrap();
        let active: ActivePublication =
            serde_json::from_slice(&fs::read(active_pointer_path(&registry, "demo")).unwrap())
                .unwrap();
        assert_eq!(active.sha256, first_digest);
        assert_eq!(
            active.artifact,
            first_path
                .strip_prefix(&registry)
                .unwrap()
                .to_string_lossy()
        );
    }

    #[test]
    fn immutable_publication_never_overwrites_divergent_existing_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path().join("packs/example");
        create_dir_all_durable(&parent).unwrap();
        let retry_path = parent.join("retry.world.json");
        let stale = parent.join(format!(".retry.world.json.{}.0.part", std::process::id()));
        fs::write(&stale, b"orphaned partial bytes").unwrap();
        write_immutable_artifact(&retry_path, b"complete reviewed bytes").unwrap();
        assert_eq!(fs::read(&retry_path).unwrap(), b"complete reviewed bytes");
        assert!(stale.exists());

        let divergent_path = parent.join("digest.world.json");
        fs::write(&divergent_path, b"existing divergent bytes").unwrap();

        assert!(matches!(
            write_immutable_artifact(&divergent_path, b"new reviewed bytes"),
            Err(CliError::PublishedDigestMismatch)
        ));
        assert_eq!(
            fs::read(divergent_path).unwrap(),
            b"existing divergent bytes"
        );
    }

    fn lantern_project_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("worlds/lantern-square.project.json")
    }

    fn write_area_release(
        path: &Path,
        catalog_revision: u64,
        square_revision: u64,
        square_reviewer: &str,
    ) {
        let release = serde_json::json!({
            "schemaVersion": 1,
            "trustMode": "unsigned_local",
            "catalogRevision": catalog_revision,
            "areas": [
                {
                    "areaId": "archive",
                    "revision": 1,
                    "contentLicense": "CC-BY-4.0",
                    "scriptLicense": "MIT OR Apache-2.0",
                    "reviewer": "Tiamud maintainers",
                    "reviewedAt": "2026-07-17T00:00:00Z",
                    "provenance": "Reviewed original Tiamud source."
                },
                {
                    "areaId": "north-gate",
                    "revision": 2,
                    "contentLicense": "CC-BY-4.0",
                    "scriptLicense": "MIT OR Apache-2.0",
                    "reviewer": "Tiamud maintainers",
                    "reviewedAt": "2026-07-17T00:00:00Z",
                    "provenance": "Reviewed original Tiamud source."
                },
                {
                    "areaId": "square",
                    "revision": square_revision,
                    "contentLicense": "CC-BY-4.0",
                    "scriptLicense": "MIT OR Apache-2.0",
                    "reviewer": square_reviewer,
                    "reviewedAt": "2026-07-17T00:00:00Z",
                    "provenance": "Reviewed original Tiamud source."
                }
            ]
        });
        fs::write(path, serde_json::to_vec(&release).unwrap()).unwrap();
    }

    fn published_area_catalog_path(registry: &Path, world_id: &str) -> PathBuf {
        let pointer: ActiveAreaCatalog = serde_json::from_slice(
            &fs::read(local_active_area_catalog_path(registry, world_id)).unwrap(),
        )
        .unwrap();
        registry.join(pointer.catalog)
    }

    fn immutable_area_blobs(registry: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        fn collect(root: &Path, directory: &Path, output: &mut Vec<(PathBuf, Vec<u8>)>) {
            if !directory.exists() {
                return;
            }
            for entry in fs::read_dir(directory).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    collect(root, &path, output);
                } else {
                    output.push((
                        path.strip_prefix(root).unwrap().to_owned(),
                        fs::read(path).unwrap(),
                    ));
                }
            }
        }

        let mut blobs = Vec::new();
        for directory in ["area-catalogs", "area-bundles", "area-worlds"] {
            collect(registry, &registry.join(directory), &mut blobs);
        }
        blobs.sort_by(|left, right| left.0.cmp(&right.0));
        blobs
    }

    #[test]
    fn area_publication_is_immutable_verified_and_revision_guarded() {
        let temp = tempfile::tempdir().unwrap();
        let registry = temp.path().join("registry");
        let release = temp.path().join("release.json");
        write_area_release(&release, 1, 3, "Tiamud maintainers");

        run(Cli {
            command: Command::AreaPublish {
                project: lantern_project_path(),
                release: release.clone(),
                registry: registry.clone(),
            },
        })
        .unwrap();
        let first_catalog_path = published_area_catalog_path(&registry, "lantern-square");
        let first_catalog_bytes = fs::read(&first_catalog_path).unwrap();
        let (first_catalog, first_digest) =
            load_verified_area_catalog(&first_catalog_path, &registry).unwrap();
        let success =
            area_publish_success_message(&first_catalog, &first_digest, &first_catalog_path);
        assert!(success.contains(&format!("at {first_catalog_path:?}")));
        run(Cli {
            command: Command::AreaVerify {
                catalog: first_catalog_path.clone(),
                registry: registry.clone(),
            },
        })
        .unwrap();

        // Changing reviewed metadata changes the immutable bundle and therefore
        // requires a strictly newer area revision as well as a catalog revision.
        write_area_release(&release, 2, 3, "Second reviewer");
        assert!(matches!(
            run(Cli {
                command: Command::AreaPublish {
                    project: lantern_project_path(),
                    release: release.clone(),
                    registry: registry.clone(),
                },
            }),
            Err(CliError::AreaCatalog(
                AreaCatalogError::RevisionNotAdvanced { .. }
            ))
        ));
        assert_eq!(fs::read(&first_catalog_path).unwrap(), first_catalog_bytes);
        assert_eq!(
            published_area_catalog_path(&registry, "lantern-square"),
            first_catalog_path
        );

        write_area_release(&release, 2, 4, "Second reviewer");
        run(Cli {
            command: Command::AreaPublish {
                project: lantern_project_path(),
                release,
                registry: registry.clone(),
            },
        })
        .unwrap();
        let second_catalog_path = published_area_catalog_path(&registry, "lantern-square");
        assert_ne!(second_catalog_path, first_catalog_path);
        assert_eq!(fs::read(first_catalog_path).unwrap(), first_catalog_bytes);
        run(Cli {
            command: Command::AreaVerify {
                catalog: second_catalog_path,
                registry,
            },
        })
        .unwrap();
    }

    #[test]
    fn area_select_supports_forward_rollback_and_idempotent_noop_without_mutating_blobs() {
        let temp = tempfile::tempdir().unwrap();
        let registry = temp.path().join("registry");
        let release = temp.path().join("release.json");
        write_area_release(&release, 1, 3, "Initial reviewer");
        run(Cli {
            command: Command::AreaPublish {
                project: lantern_project_path(),
                release: release.clone(),
                registry: registry.clone(),
            },
        })
        .unwrap();
        let first_catalog_path = published_area_catalog_path(&registry, "lantern-square");

        write_area_release(&release, 2, 4, "Second reviewer");
        run(Cli {
            command: Command::AreaPublish {
                project: lantern_project_path(),
                release,
                registry: registry.clone(),
            },
        })
        .unwrap();
        let second_catalog_path = published_area_catalog_path(&registry, "lantern-square");
        let immutable_before = immutable_area_blobs(&registry);

        let rollback = select_area_catalog(&first_catalog_path, &registry).unwrap();
        assert_eq!(rollback.kind, AreaSelectionKind::Rollback);
        assert_eq!(rollback.previous_revision, Some(2));
        assert!(area_selection_success_message(&rollback).contains("for rollback"));
        assert_eq!(
            published_area_catalog_path(&registry, "lantern-square"),
            first_catalog_path
        );

        let activation = select_area_catalog(&second_catalog_path, &registry).unwrap();
        assert_eq!(activation.kind, AreaSelectionKind::Activate);
        assert_eq!(activation.previous_revision, Some(1));
        assert!(area_selection_success_message(&activation).contains("for activation"));
        assert_eq!(
            published_area_catalog_path(&registry, "lantern-square"),
            second_catalog_path
        );

        let pointer_path = local_active_area_catalog_path(&registry, "lantern-square");
        let pointer_before_noop = fs::read(&pointer_path).unwrap();
        let noop = select_area_catalog(&second_catalog_path, &registry).unwrap();
        assert_eq!(noop.kind, AreaSelectionKind::Noop);
        assert!(area_selection_success_message(&noop).contains("(noop)"));
        assert_eq!(fs::read(pointer_path).unwrap(), pointer_before_noop);
        assert_eq!(immutable_area_blobs(&registry), immutable_before);
    }

    #[test]
    fn area_select_rejects_tamper_and_catalog_symlinks_without_changing_selection() {
        let temp = tempfile::tempdir().unwrap();
        let registry = temp.path().join("registry");
        let release = temp.path().join("release.json");
        write_area_release(&release, 1, 3, "Initial reviewer");
        run(Cli {
            command: Command::AreaPublish {
                project: lantern_project_path(),
                release,
                registry: registry.clone(),
            },
        })
        .unwrap();
        let catalog_path = published_area_catalog_path(&registry, "lantern-square");
        let pointer_path = local_active_area_catalog_path(&registry, "lantern-square");
        let pointer_before = fs::read(&pointer_path).unwrap();
        let catalog = parse_area_catalog(&fs::read(&catalog_path).unwrap()).unwrap();
        let entry = &catalog.entries[0];
        let bundle_path = area_bundle_blob_path(
            &registry,
            &catalog.world_id,
            &entry.area_id,
            &entry.bundle_sha256,
        );
        let bundle_before = fs::read(&bundle_path).unwrap();
        fs::write(&bundle_path, [bundle_before.as_slice(), b" "].concat()).unwrap();
        assert!(select_area_catalog(&catalog_path, &registry).is_err());
        assert_eq!(fs::read(&pointer_path).unwrap(), pointer_before);
        fs::write(&bundle_path, bundle_before).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let outside = temp.path().join("outside-catalog.json");
            fs::write(&outside, fs::read(&catalog_path).unwrap()).unwrap();
            fs::remove_file(&catalog_path).unwrap();
            symlink(outside, &catalog_path).unwrap();
            assert!(matches!(
                select_area_catalog(&catalog_path, &registry),
                Err(CliError::InvalidAreaRegistry(_))
            ));
            assert_eq!(fs::read(pointer_path).unwrap(), pointer_before);
        }
    }

    #[test]
    fn area_select_waits_for_lock_then_revalidates_pointer_and_candidate() {
        let temp = tempfile::tempdir().unwrap();
        let registry = temp.path().join("registry");
        let release = temp.path().join("release.json");
        write_area_release(&release, 1, 3, "Initial reviewer");
        run(Cli {
            command: Command::AreaPublish {
                project: lantern_project_path(),
                release: release.clone(),
                registry: registry.clone(),
            },
        })
        .unwrap();
        let first_catalog_path = published_area_catalog_path(&registry, "lantern-square");
        let (first_catalog, first_digest) =
            load_verified_area_catalog(&first_catalog_path, &registry).unwrap();
        write_area_release(&release, 2, 4, "Second reviewer");
        run(Cli {
            command: Command::AreaPublish {
                project: lantern_project_path(),
                release,
                registry: registry.clone(),
            },
        })
        .unwrap();
        let second_catalog_path = published_area_catalog_path(&registry, "lantern-square");

        let lock_path = area_publication_lock_path(&registry, "lantern-square");
        let lock_file = open_area_publication_lock(&registry, &lock_path).unwrap();
        let mut held_lock = RwLock::new(lock_file);
        let held_guard = held_lock.write().unwrap();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let waiting_registry = registry.clone();
        let waiting_catalog = first_catalog_path.clone();
        let selector = std::thread::spawn(move || {
            select_area_catalog_with_observer(&waiting_catalog, &waiting_registry, move || {
                ready_tx.send(()).unwrap()
            })
            .map_err(|error| error.to_string())
        });
        ready_rx.recv().unwrap();
        activate_area_catalog(
            &registry,
            &first_catalog,
            &first_digest,
            &first_catalog_path,
        )
        .unwrap();
        drop(held_guard);
        let selection = selector.join().unwrap().unwrap();
        assert_eq!(selection.kind, AreaSelectionKind::Noop);

        let lock_file = open_area_publication_lock(&registry, &lock_path).unwrap();
        let mut held_lock = RwLock::new(lock_file);
        let held_guard = held_lock.write().unwrap();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let waiting_registry = registry.clone();
        let waiting_catalog = second_catalog_path.clone();
        let selector = std::thread::spawn(move || {
            select_area_catalog_with_observer(&waiting_catalog, &waiting_registry, move || {
                ready_tx.send(()).unwrap()
            })
            .map_err(|error| error.to_string())
        });
        ready_rx.recv().unwrap();
        let mut changed = fs::read(&second_catalog_path).unwrap();
        changed.push(b' ');
        fs::write(&second_catalog_path, changed).unwrap();
        drop(held_guard);
        let error = selector.join().unwrap().unwrap_err();
        assert!(error.contains("catalog bytes are not canonical"), "{error}");
        assert_eq!(
            published_area_catalog_path(&registry, "lantern-square"),
            first_catalog_path
        );
    }

    #[test]
    fn area_verify_rejects_aggregate_bundle_budget_before_blob_io() {
        let temp = tempfile::tempdir().unwrap();
        let registry = temp.path().join("registry");
        let release_path = temp.path().join("release.json");
        write_area_release(&release_path, 1, 3, "Tiamud maintainers");
        let composed = compose_world_project(lantern_project_path()).unwrap();
        let release = parse_area_release_manifest(&fs::read(release_path).unwrap()).unwrap();
        let mut catalog = build_area_release(&composed, &release, None)
            .unwrap()
            .catalog;

        // Each entry remains below the individual 16 MiB limit, but the set
        // exceeds the aggregate budget. Verification must reject the catalog
        // before trying to open the absent WorldPack or any bundle.
        for entry in &mut catalog.entries {
            entry.stored_bytes = MAX_TOTAL_AREA_BYTES / 2;
        }
        let bytes = serde_json::to_vec(&catalog).unwrap();
        let digest = sha256_hex(&bytes);
        let path = area_catalog_blob_path(&registry, &catalog.world_id, &digest);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();

        let error = load_verified_area_catalog(&path, &registry).unwrap_err();
        assert!(matches!(
            error,
            CliError::InvalidAreaRegistry(reason)
                if reason.contains("area bundle set declares")
        ));
        assert!(!registry.join("area-worlds").exists());
        assert!(!registry.join("area-bundles").exists());
    }

    #[test]
    fn area_verify_does_not_create_a_missing_registry() {
        let temp = tempfile::tempdir().unwrap();
        let registry = temp.path().join("missing-registry");
        let catalog = area_catalog_blob_path(&registry, "demo", &"a".repeat(64));

        assert!(
            run(Cli {
                command: Command::AreaVerify {
                    catalog,
                    registry: registry.clone(),
                },
            })
            .is_err()
        );
        assert!(!registry.exists());
    }

    #[test]
    fn identical_existing_area_blob_is_reused_after_durable_resync() {
        let temp = tempfile::tempdir().unwrap();
        let registry = temp.path().join("registry");
        let bytes = b"reviewed immutable area bytes";
        let digest = sha256_hex(bytes);
        let path = area_bundle_blob_path(&registry, "demo", "square", &digest);

        write_immutable_area_blob(&registry, &path, bytes, &digest, bytes.len()).unwrap();
        write_immutable_area_blob(&registry, &path, bytes, &digest, bytes.len()).unwrap();
        assert_eq!(fs::read(path).unwrap(), bytes);
    }

    #[test]
    fn concurrent_area_publishers_serialize_and_revalidate_active_revision() {
        let temp = tempfile::tempdir().unwrap();
        let registry = temp.path().join("registry");
        let first_release = temp.path().join("release-1.json");
        write_area_release(&first_release, 1, 3, "Initial reviewer");
        run(Cli {
            command: Command::AreaPublish {
                project: lantern_project_path(),
                release: first_release,
                registry: registry.clone(),
            },
        })
        .unwrap();

        let release_a = temp.path().join("release-2-a.json");
        let release_b = temp.path().join("release-2-b.json");
        write_area_release(&release_a, 2, 4, "Concurrent reviewer A");
        write_area_release(&release_b, 2, 4, "Concurrent reviewer B");

        // Hold the same per-world advisory lock so both publishers are ready
        // concurrently before either can inspect the active revision.
        let lock_path = area_publication_lock_path(&registry, "lantern-square");
        let lock_file = open_area_publication_lock(&registry, &lock_path).unwrap();
        let mut held_lock = RwLock::new(lock_file);
        let held_guard = held_lock.write().unwrap();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let spawn = |release: PathBuf| {
            let registry = registry.clone();
            let ready_tx = ready_tx.clone();
            std::thread::spawn(move || {
                ready_tx.send(()).unwrap();
                run(Cli {
                    command: Command::AreaPublish {
                        project: lantern_project_path(),
                        release,
                        registry,
                    },
                })
                .map_err(|error| error.to_string())
            })
        };
        let publisher_a = spawn(release_a);
        let publisher_b = spawn(release_b);
        ready_rx.recv().unwrap();
        ready_rx.recv().unwrap();
        drop(held_guard);

        let outcomes = [publisher_a.join().unwrap(), publisher_b.join().unwrap()];
        assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
        let rejected = outcomes
            .iter()
            .find_map(|result| result.as_ref().err())
            .unwrap();
        assert!(rejected.contains("catalog revision 2 must be newer than active revision 2"));

        let active = load_active_area_catalog(&registry, "lantern-square")
            .unwrap()
            .unwrap();
        assert_eq!(active.catalog_revision, 2);
    }

    #[test]
    fn area_verify_rejects_tampered_blob_and_symlink() {
        let temp = tempfile::tempdir().unwrap();
        let registry = temp.path().join("registry");
        let release = temp.path().join("release.json");
        write_area_release(&release, 1, 3, "Tiamud maintainers");
        run(Cli {
            command: Command::AreaPublish {
                project: lantern_project_path(),
                release,
                registry: registry.clone(),
            },
        })
        .unwrap();
        let catalog_path = published_area_catalog_path(&registry, "lantern-square");
        let catalog = parse_area_catalog(&fs::read(&catalog_path).unwrap()).unwrap();
        let entry = &catalog.entries[0];
        let bundle_path = area_bundle_blob_path(
            &registry,
            &catalog.world_id,
            &entry.area_id,
            &entry.bundle_sha256,
        );
        let original = fs::read(&bundle_path).unwrap();
        fs::write(&bundle_path, [original.as_slice(), b" "].concat()).unwrap();
        assert!(
            run(Cli {
                command: Command::AreaVerify {
                    catalog: catalog_path.clone(),
                    registry: registry.clone(),
                },
            })
            .is_err()
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            fs::write(&bundle_path, original).unwrap();
            let outside = temp.path().join("outside-catalog.json");
            fs::write(&outside, fs::read(&catalog_path).unwrap()).unwrap();
            fs::remove_file(&catalog_path).unwrap();
            symlink(&outside, &catalog_path).unwrap();
            assert!(matches!(
                run(Cli {
                    command: Command::AreaVerify {
                        catalog: catalog_path,
                        registry,
                    },
                }),
                Err(CliError::InvalidAreaRegistry(_))
            ));
        }
    }

    #[cfg(feature = "editorial-ai")]
    fn fixture_ai_draft_args(input: PathBuf, output: PathBuf) -> AiDraftArgs {
        AiDraftArgs {
            provider: CliAiProvider::None,
            endpoint: None,
            model: None,
            authorization_env: None,
            task: CliEditorialTask::SuggestAltText,
            input,
            context: None,
            output,
            timeout_seconds: 30,
            connect_timeout_seconds: 3,
            max_response_bytes: MAX_RESPONSE_BYTES as u64,
            max_output_chars: 512,
        }
    }

    #[cfg(feature = "editorial-ai")]
    #[test]
    fn editorial_draft_and_human_review_are_separate_authoring_artifacts() {
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("prompt.txt");
        let draft_path = temp.path().join("draft.json");
        let reviewed_path = temp.path().join("reviewed.json");
        let prompt = "Describe the square without revealing internal ids.";
        fs::write(&input, prompt).unwrap();

        run_ai_draft(
            fixture_ai_draft_args(input, draft_path.clone()),
            |config, limits, request| {
                assert!(matches!(config, ProviderConfig::None));
                assert_eq!(limits.timeout, Duration::from_secs(30));
                assert_eq!(request.input, prompt);
                Ok(EditorialSuggestion {
                    provider: "fixture".to_owned(),
                    model: "deterministic".to_owned(),
                    task: request.task,
                    text: "Lantern light crosses the rain-dark stones.".to_owned(),
                    provenance: "deterministic test fixture".to_owned(),
                    human_review: None,
                })
            },
        )
        .unwrap();

        let draft_bytes = fs::read(&draft_path).unwrap();
        let draft: EditorialReviewArtifact = serde_json::from_slice(&draft_bytes).unwrap();
        assert_eq!(draft.state, EditorialArtifactState::Draft);
        assert!(draft.suggestion.human_review.is_none());
        assert!(!draft.suggestion.is_publishable());
        assert!(!String::from_utf8_lossy(&draft_bytes).contains(prompt));

        run_ai_review(AiReviewArgs {
            draft: draft_path.clone(),
            reviewer: "Editor".to_owned(),
            reviewed_at: "2026-07-15T00:00:00Z".to_owned(),
            decision: CliReviewDecision::Accept,
            output: reviewed_path.clone(),
        })
        .unwrap();

        assert_eq!(fs::read(&draft_path).unwrap(), draft_bytes);
        let reviewed_bytes = fs::read(&reviewed_path).unwrap();
        let reviewed: EditorialReviewArtifact = serde_json::from_slice(&reviewed_bytes).unwrap();
        assert_eq!(reviewed.state, EditorialArtifactState::Reviewed);
        assert!(reviewed.suggestion.is_publishable());
        assert!(WorldPackDocument::from_json(&reviewed_bytes).is_err());

        let registry = temp.path().join("registry");
        assert!(matches!(
            run(Cli {
                command: Command::Publish {
                    input: reviewed_path,
                    registry: registry.clone(),
                },
            }),
            Err(CliError::Document { .. })
        ));
        assert!(!registry.exists());
    }

    #[cfg(feature = "editorial-ai")]
    #[test]
    fn editorial_backend_is_disabled_by_default_and_writes_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("prompt.txt");
        let output = temp.path().join("draft.json");
        fs::write(&input, "A bounded prompt").unwrap();
        let result = run(Cli {
            command: Command::AiDraft(fixture_ai_draft_args(input, output.clone())),
        });
        assert!(matches!(
            result,
            Err(CliError::Editorial(EditorialError::Disabled))
        ));
        assert!(!output.exists());
    }
}
