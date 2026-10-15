//! Single-player story sessions (ADR 0029, phase S2 of ADR 0025).
//!
//! A [`StoryCatalog`] of validated StoryPacks is loaded once at startup from a
//! configured directory. A [`StoryManager`] owns the live instances: each one
//! is a single-writer Tokio task with a bounded mailbox that owns its
//! [`StoryState`], its in-memory [`UndoRing`] and the opaque ids of its current
//! view. Instances are keyed by account (one per account in this phase),
//! bounded globally and evicted after an idle timeout.
//!
//! With a character store attached, the persisted save slots of ADR 0030 are
//! wired here: manual slots `1..=9`, and an autosave in slot `0` written on
//! leave, on idle eviction and on graceful shutdown. Store calls run on the
//! session task (or on the exiting instance task), never inside the running
//! instance or the world actor; the instance only hands out and accepts
//! validated `StoryState` values. Loaded bytes are re-validated with
//! `StoryState::from_json_slice` against the exact pack revision (its
//! SHA-256). The in-memory undo history is never persisted.
//!
//! The ruleset is `tiamud-story-core`; this module only serializes inputs,
//! projects views and enforces bounds. It never reads `tiamud-core` world
//! state and the world actor never sees story input.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fmt::Write as _,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use crate::{
    operations::KeyedRateLimiter,
    persistence::{
        CharacterStore, StoreError, StoryPackCompatibility, StorySaveSummary, StorySaveWrite,
        StorySlot,
    },
};
use sha2::{Digest, Sha256};

use rand_core::{OsRng, RngCore};
use serde::Deserialize;
use thiserror::Error;
use tiamud_core::AccountId;
use tiamud_protocol::{
    self as protocol, MediaChannel, STORY_CHOICE_ID_BYTES, ServerMessageType, StoryBlockKind,
    StoryCueAction, StoryEnding, StoryEndingKind, StoryErrorCode, StoryItem,
};
use tiamud_story_core::{
    Cue, EndingKind, MAX_PACK_BYTES, PageItem, StoryEvent, StoryInput, StoryPack, StoryState,
    UndoRing, Value, step, view as core_view, view_pages,
};
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};

/// Locale used to resolve story text in S2. Per-session locales are later work.
pub const STORY_LOCALE: &str = "es";
/// Maximum StoryPacks in one catalog directory.
pub const MAX_CATALOG_STORIES: usize = 64;
/// Maximum directory entries scanned while loading a catalog.
const MAX_CATALOG_DIRECTORY_ENTRIES: usize = 1_024;
/// Default and absolute cap of live instances.
pub const DEFAULT_MAX_INSTANCES: usize = 256;
pub const MAX_INSTANCES_CAP: usize = 4_096;
/// Live instances per account in S2.
pub const MAX_INSTANCES_PER_ACCOUNT: usize = 1;
/// Idle eviction bounds, in seconds.
pub const DEFAULT_IDLE_SECONDS: u64 = 30 * 60;
pub const MIN_IDLE_SECONDS: u64 = 60;
pub const MAX_IDLE_SECONDS: u64 = 24 * 60 * 60;
/// Bounded mailbox of one instance task.
pub const INSTANCE_MAILBOX: usize = 16;
/// In-memory undo depth of one instance.
pub const UNDO_CAPACITY: usize = 16;
/// How long a session waits for its instance before reporting `busy`.
const REPLY_TIMEOUT: Duration = Duration::from_secs(5);
/// Optional reviewed media map inside the catalog directory.
pub const STORY_MEDIA_FILE: &str = "story-media.json";
const MAX_MEDIA_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_MEDIA_ASSETS: usize = 256;
/// Every WebSocket frame stays within the 16 KiB protocol limit.
#[cfg(test)]
pub const MAX_STORY_FRAME_BYTES: usize = 16 * 1024;
/// Encoded budget of one `story.view` frame, below the frame limit.
const STORY_VIEW_FRAME_BUDGET: usize = 15 * 1024;
/// Maximum bytes of one projected text block.
pub const MAX_STORY_BLOCK_BYTES: usize = 2 * 1024;
/// Slot writes (`save`, `delete`) per account and minute, also the burst.
pub const SLOT_WRITES_PER_MINUTE: u32 = 6;
/// Slot reads (`slots`, `restore`, `continue`) per account and minute.
pub const SLOT_READS_PER_MINUTE: u32 = 30;
const SLOT_READ_BURST: u32 = 10;
/// Accounts tracked by each slot rate limiter (least recently seen evicted).
const MAX_TRACKED_SLOT_ACCOUNTS: usize = 4_096;
/// Upper bound of one store call made for a story slot.
const STORE_TIMEOUT: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoryLimits {
    pub max_instances: usize,
    pub idle_timeout: Duration,
}

impl Default for StoryLimits {
    fn default() -> Self {
        Self {
            max_instances: DEFAULT_MAX_INSTANCES,
            idle_timeout: Duration::from_secs(DEFAULT_IDLE_SECONDS),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct StoryConfig {
    /// `None` disables story sessions entirely.
    pub directory: Option<PathBuf>,
    pub limits: StoryLimits,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum StoryConfigError {
    #[error("TIAMUD_STORY_DIR must not be empty or contain surrounding whitespace")]
    InvalidDirectory,
    #[error("TIAMUD_STORY_MAX_INSTANCES must be an integer in 1..={MAX_INSTANCES_CAP}")]
    InvalidMaxInstances,
    #[error(
        "TIAMUD_STORY_IDLE_SECONDS must be an integer in {MIN_IDLE_SECONDS}..={MAX_IDLE_SECONDS}"
    )]
    InvalidIdleSeconds,
    #[error("story limits were configured but TIAMUD_STORY_DIR is not set")]
    LimitsWithoutDirectory,
}

/// Parses the story configuration from the raw environment values. Stories
/// are disabled unless a directory is configured; out-of-range limits fail
/// closed instead of being clamped silently.
pub fn story_config(
    directory: Option<String>,
    max_instances: Option<String>,
    idle_seconds: Option<String>,
) -> Result<StoryConfig, StoryConfigError> {
    let mut limits = StoryLimits::default();
    if let Some(value) = max_instances.as_deref() {
        limits.max_instances = value
            .parse::<usize>()
            .ok()
            .filter(|value| (1..=MAX_INSTANCES_CAP).contains(value))
            .ok_or(StoryConfigError::InvalidMaxInstances)?;
    }
    if let Some(value) = idle_seconds.as_deref() {
        let seconds = value
            .parse::<u64>()
            .ok()
            .filter(|value| (MIN_IDLE_SECONDS..=MAX_IDLE_SECONDS).contains(value))
            .ok_or(StoryConfigError::InvalidIdleSeconds)?;
        limits.idle_timeout = Duration::from_secs(seconds);
    }
    let directory = match directory {
        None => {
            if max_instances.is_some() || idle_seconds.is_some() {
                return Err(StoryConfigError::LimitsWithoutDirectory);
            }
            None
        }
        Some(path) if path.is_empty() || path.trim() != path => {
            return Err(StoryConfigError::InvalidDirectory);
        }
        Some(path) => Some(PathBuf::from(path)),
    };
    Ok(StoryConfig { directory, limits })
}

// ---------------------------------------------------------------------------
// Catalog
// ---------------------------------------------------------------------------

/// One validated, immutable StoryPack shared by reference with its instances.
#[derive(Debug)]
pub struct CatalogStory {
    pub id: String,
    pub title: String,
    pack: StoryPack,
    /// Lowercase hexadecimal SHA-256 of the exact pack bytes; the revision a
    /// save slot is bound to (ADR 0030).
    pack_sha256: String,
}

/// Validated StoryPacks plus the reviewed media map for their cues.
#[derive(Debug, Default)]
pub struct StoryCatalog {
    stories: BTreeMap<String, Arc<CatalogStory>>,
    /// Authored asset id → content-addressed `/assets/<sha256>/audio-*` URL.
    media: BTreeMap<String, String>,
}

#[derive(Debug, Error)]
pub enum StoryCatalogError {
    #[error("could not read the story directory {path}: {message}")]
    Io { path: String, message: String },
    #[error("the story directory has more than {MAX_CATALOG_DIRECTORY_ENTRIES} entries")]
    TooManyEntries,
    #[error("the story directory has more than {MAX_CATALOG_STORIES} StoryPacks")]
    TooManyStories,
    #[error("the story directory contains no *.story.json StoryPack")]
    Empty,
    #[error("{file} exceeds {limit} bytes")]
    TooLarge { file: String, limit: u64 },
    #[error("{file} is not a valid StoryPack: {reason}")]
    Invalid { file: String, reason: String },
    #[error("story id {0:?} is declared by more than one StoryPack")]
    Duplicate(String),
    #[error("{STORY_MEDIA_FILE} is invalid: {0}")]
    Media(String),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct StoryMediaManifest {
    format_version: u32,
    assets: BTreeMap<String, String>,
}

impl StoryCatalog {
    /// Loads every `*.story.json` in `directory` (non-recursive), validating
    /// each pack with `story-core`, plus the optional reviewed media map. Any
    /// invalid pack fails the whole load: the server never starts with a
    /// partially trusted catalog.
    pub fn load_dir(directory: &Path) -> Result<Self, StoryCatalogError> {
        let io_error = |path: &Path, error: std::io::Error| StoryCatalogError::Io {
            path: path.display().to_string(),
            message: error.to_string(),
        };
        let mut names = Vec::new();
        for entry in std::fs::read_dir(directory).map_err(|error| io_error(directory, error))? {
            if names.len() >= MAX_CATALOG_DIRECTORY_ENTRIES {
                return Err(StoryCatalogError::TooManyEntries);
            }
            let entry = entry.map_err(|error| io_error(directory, error))?;
            if let Some(name) = entry.file_name().to_str() {
                names.push(name.to_owned());
            }
        }
        names.sort();
        let mut packs = Vec::new();
        let mut media = BTreeMap::new();
        for name in names {
            let path = directory.join(&name);
            if name == STORY_MEDIA_FILE {
                let bytes = read_bounded(&path, MAX_MEDIA_MANIFEST_BYTES, &name)?;
                media = parse_media_manifest(&bytes)?;
            } else if name.ends_with(".story.json") {
                if packs.len() >= MAX_CATALOG_STORIES {
                    return Err(StoryCatalogError::TooManyStories);
                }
                let bytes = read_bounded(&path, MAX_PACK_BYTES as u64, &name)?;
                let pack = StoryPack::from_json_slice(&bytes).map_err(|error| {
                    StoryCatalogError::Invalid {
                        file: name.clone(),
                        reason: error.to_string(),
                    }
                })?;
                let digest = sha256_hex(&bytes);
                packs.push((name, pack, digest));
            }
        }
        if packs.is_empty() {
            return Err(StoryCatalogError::Empty);
        }
        Self::from_named_packs(packs, media)
    }

    /// Builds a catalog from in-memory packs (tests).
    #[cfg(test)]
    pub fn from_packs(
        packs: Vec<StoryPack>,
        media: BTreeMap<String, String>,
    ) -> Result<Self, StoryCatalogError> {
        let named = packs
            .into_iter()
            .map(|pack| {
                // In-memory packs have no file bytes; their canonical JSON
                // stands in for the revision digest.
                let digest = sha256_hex(&serde_json::to_vec(&pack).unwrap_or_default());
                (format!("{}.story.json", pack.id), pack, digest)
            })
            .collect();
        Self::from_named_packs(named, media)
    }

    fn from_named_packs(
        packs: Vec<(String, StoryPack, String)>,
        media: BTreeMap<String, String>,
    ) -> Result<Self, StoryCatalogError> {
        if packs.len() > MAX_CATALOG_STORIES {
            return Err(StoryCatalogError::TooManyStories);
        }
        let mut stories = BTreeMap::new();
        for (file, pack, pack_sha256) in packs {
            if let Err(errors) = pack.validate() {
                return Err(StoryCatalogError::Invalid {
                    file,
                    reason: errors
                        .errors
                        .first()
                        .map(ToString::to_string)
                        .unwrap_or_else(|| "validation failed".to_owned()),
                });
            }
            let title = pack
                .title
                .resolve(STORY_LOCALE)
                .unwrap_or(pack.id.as_str())
                .to_owned();
            let id = pack.id.clone();
            if stories
                .insert(
                    id.clone(),
                    Arc::new(CatalogStory {
                        id: id.clone(),
                        title,
                        pack,
                        pack_sha256,
                    }),
                )
                .is_some()
            {
                return Err(StoryCatalogError::Duplicate(id));
            }
        }
        Ok(Self { stories, media })
    }

    pub fn len(&self) -> usize {
        self.stories.len()
    }

    pub fn get(&self, id: &str) -> Option<&Arc<CatalogStory>> {
        self.stories.get(id)
    }

    /// `(id, title)` in id order.
    pub fn listing(&self) -> Vec<(String, String)> {
        self.stories
            .values()
            .map(|story| (story.id.clone(), story.title.clone()))
            .collect()
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn read_bounded(path: &Path, limit: u64, name: &str) -> Result<Vec<u8>, StoryCatalogError> {
    let io_error = |error: std::io::Error| StoryCatalogError::Io {
        path: path.display().to_string(),
        message: error.to_string(),
    };
    let metadata = std::fs::metadata(path).map_err(io_error)?;
    if !metadata.is_file() {
        return Err(StoryCatalogError::Invalid {
            file: name.to_owned(),
            reason: "not a regular file".to_owned(),
        });
    }
    if metadata.len() > limit {
        return Err(StoryCatalogError::TooLarge {
            file: name.to_owned(),
            limit,
        });
    }
    let bytes = std::fs::read(path).map_err(io_error)?;
    if bytes.len() as u64 > limit {
        return Err(StoryCatalogError::TooLarge {
            file: name.to_owned(),
            limit,
        });
    }
    Ok(bytes)
}

fn parse_media_manifest(bytes: &[u8]) -> Result<BTreeMap<String, String>, StoryCatalogError> {
    let manifest: StoryMediaManifest = serde_json::from_slice(bytes)
        .map_err(|_| StoryCatalogError::Media("malformed JSON document".to_owned()))?;
    if manifest.format_version != 1 {
        return Err(StoryCatalogError::Media(
            "formatVersion must be 1".to_owned(),
        ));
    }
    if manifest.assets.len() > MAX_MEDIA_ASSETS {
        return Err(StoryCatalogError::Media(format!(
            "at most {MAX_MEDIA_ASSETS} assets are allowed"
        )));
    }
    let mut media = BTreeMap::new();
    for (asset, path) in manifest.assets {
        let url = format!("/{path}");
        if !valid_asset_id(&asset) || !crate::web::safe_content_addressed_audio_url(&url) {
            return Err(StoryCatalogError::Media(format!(
                "asset {asset:?} must map to assets/<sha256>/audio-<name>.{{opus,ogg,mp3}}"
            )));
        }
        media.insert(asset, url);
    }
    Ok(media)
}

fn valid_asset_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 64
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
        })
}

// ---------------------------------------------------------------------------
// Failures and metrics
// ---------------------------------------------------------------------------

/// Typed, public story failure. Messages never name authored ids.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoryFailure {
    Disabled,
    UnknownStory,
    AlreadyInStory,
    Capacity,
    NotInStory,
    StaleView,
    UnknownChoice,
    Ended,
    NothingToUndo,
    Busy,
    /// The instance no longer exists (idle eviction or closed elsewhere).
    Gone,
    Internal,
    /// No character store implements save slots.
    SlotsUnavailable,
    /// The slot number is outside the range of the operation.
    InvalidSlot,
    SlotEmpty,
    /// Stored bytes failed their digest, metadata or ruleset validation.
    SlotCorrupt,
    /// The slot was saved with another revision of the story pack.
    SlotIncompatible,
    /// Per-account slot capacity (stories or bytes) or the state size limit.
    SlotCapacity,
    RateLimited,
    /// The store failed or timed out; nothing changed.
    SlotStore,
}

impl StoryFailure {
    pub const fn code(self) -> StoryErrorCode {
        match self {
            Self::Disabled => StoryErrorCode::Disabled,
            Self::UnknownStory => StoryErrorCode::UnknownStory,
            Self::AlreadyInStory => StoryErrorCode::AlreadyInStory,
            Self::Capacity => StoryErrorCode::Capacity,
            Self::NotInStory | Self::Gone => StoryErrorCode::NotInStory,
            Self::StaleView => StoryErrorCode::StaleView,
            Self::UnknownChoice => StoryErrorCode::UnknownChoice,
            Self::Ended => StoryErrorCode::Ended,
            Self::NothingToUndo => StoryErrorCode::NothingToUndo,
            Self::Busy => StoryErrorCode::Busy,
            Self::Internal | Self::SlotStore => StoryErrorCode::Internal,
            Self::SlotsUnavailable => StoryErrorCode::SlotsUnavailable,
            Self::InvalidSlot => StoryErrorCode::InvalidSlot,
            Self::SlotEmpty => StoryErrorCode::SlotEmpty,
            Self::SlotCorrupt => StoryErrorCode::SlotCorrupt,
            Self::SlotIncompatible => StoryErrorCode::SlotIncompatible,
            Self::SlotCapacity => StoryErrorCode::SlotCapacity,
            Self::RateLimited => StoryErrorCode::RateLimited,
        }
    }

    pub const fn message(self) -> &'static str {
        match self {
            Self::Disabled => "Las historias no están disponibles en este servidor.",
            Self::UnknownStory => "No existe esa historia. Escribe «historias» para ver la lista.",
            Self::AlreadyInStory => {
                "Ya estás dentro de otra historia; escribe «salir» antes de empezar otra."
            }
            Self::Capacity => "Hay demasiadas historias en curso. Inténtalo más tarde.",
            Self::NotInStory => "No estás dentro de ninguna historia.",
            Self::StaleView => "Esa opción pertenece a una vista anterior; se muestra la actual.",
            Self::UnknownChoice => "Esa opción no está disponible.",
            Self::Ended => "La historia ha terminado. Escribe «salir» para volver al mundo.",
            Self::NothingToUndo => "No queda nada que deshacer.",
            Self::Busy => "La historia está ocupada. Inténtalo de nuevo.",
            Self::Gone => {
                "La historia se cerró por inactividad o desde otra sesión. Si hay autoguardado, al volver a entrar podrás continuar."
            }
            Self::Internal => "La historia no pudo continuar.",
            Self::SlotsUnavailable => {
                "Las partidas guardadas no están disponibles en este servidor."
            }
            Self::InvalidSlot => {
                "Usa una ranura del 1 al 9 para guardar; la 0 es el autoguardado (solo cargar o borrar)."
            }
            Self::SlotEmpty => "Esa ranura está vacía.",
            Self::SlotCorrupt => {
                "Esa partida guardada está dañada y no se puede cargar. Puedes sobrescribirla guardando de nuevo."
            }
            Self::SlotIncompatible => {
                "Esa partida se guardó con otra versión de la historia y ya no se puede cargar."
            }
            Self::SlotCapacity => {
                "No queda espacio para guardar esta partida. Borra alguna partida guardada e inténtalo de nuevo."
            }
            Self::RateLimited => {
                "Demasiadas operaciones con partidas guardadas. Espera un momento."
            }
            Self::SlotStore => {
                "No se pudo acceder a las partidas guardadas. Inténtalo de nuevo más tarde."
            }
        }
    }

    /// Maps a store error to a public failure without leaking its details.
    fn from_store(error: &StoreError) -> Self {
        match error {
            StoreError::StorySaveUnsupported => Self::SlotsUnavailable,
            StoreError::StorySaveCapacity | StoreError::StorySaveTooLarge { .. } => {
                Self::SlotCapacity
            }
            StoreError::StorySaveCorrupt => Self::SlotCorrupt,
            StoreError::StorySavePackMismatch => Self::SlotIncompatible,
            _ => Self::SlotStore,
        }
    }

    pub fn protocol_error(self) -> protocol::StoryError {
        protocol::StoryError {
            code: self.code(),
            message: self.message().to_owned(),
        }
    }
}

/// Story metrics. Labels never carry story, node, choice or account ids.
#[derive(Debug, Default)]
pub struct StoryMetrics {
    live: AtomicUsize,
    started: AtomicU64,
    closed_left: AtomicU64,
    closed_evicted: AtomicU64,
    closed_shutdown: AtomicU64,
    rejected_capacity: AtomicU64,
    rejected_account: AtomicU64,
    rejected_unknown: AtomicU64,
    inputs_accepted: AtomicU64,
    inputs_rejected: AtomicU64,
    saves_manual_ok: AtomicU64,
    saves_manual_failed: AtomicU64,
    saves_auto_ok: AtomicU64,
    saves_auto_failed: AtomicU64,
    restores_ok: AtomicU64,
    restores_rejected: AtomicU64,
    slot_rate_limited: AtomicU64,
}

impl StoryMetrics {
    fn save(&self, autosave: bool, ok: bool) {
        match (autosave, ok) {
            (false, true) => &self.saves_manual_ok,
            (false, false) => &self.saves_manual_failed,
            (true, true) => &self.saves_auto_ok,
            (true, false) => &self.saves_auto_failed,
        }
        .fetch_add(1, Ordering::Relaxed);
    }

    fn restore(&self, ok: bool) {
        if ok {
            &self.restores_ok
        } else {
            &self.restores_rejected
        }
        .fetch_add(1, Ordering::Relaxed);
    }

    fn input(&self, accepted: bool) {
        let counter = if accepted {
            &self.inputs_accepted
        } else {
            &self.inputs_rejected
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub fn prometheus(&self, catalog_stories: usize) -> String {
        format!(
            concat!(
                "# TYPE tiamud_story_catalog_stories gauge\n",
                "tiamud_story_catalog_stories {}\n",
                "# TYPE tiamud_story_instances gauge\n",
                "tiamud_story_instances {}\n",
                "# TYPE tiamud_story_instances_started_total counter\n",
                "tiamud_story_instances_started_total {}\n",
                "# TYPE tiamud_story_instances_closed_total counter\n",
                "tiamud_story_instances_closed_total{{reason=\"left\"}} {}\n",
                "tiamud_story_instances_closed_total{{reason=\"evicted\"}} {}\n",
                "tiamud_story_instances_closed_total{{reason=\"shutdown\"}} {}\n",
                "# TYPE tiamud_story_enter_rejected_total counter\n",
                "tiamud_story_enter_rejected_total{{reason=\"capacity\"}} {}\n",
                "tiamud_story_enter_rejected_total{{reason=\"account\"}} {}\n",
                "tiamud_story_enter_rejected_total{{reason=\"unknown\"}} {}\n",
                "# TYPE tiamud_story_inputs_total counter\n",
                "tiamud_story_inputs_total{{result=\"accepted\"}} {}\n",
                "tiamud_story_inputs_total{{result=\"rejected\"}} {}\n",
                "# TYPE tiamud_story_saves_total counter\n",
                "tiamud_story_saves_total{{kind=\"manual\",result=\"ok\"}} {}\n",
                "tiamud_story_saves_total{{kind=\"manual\",result=\"failed\"}} {}\n",
                "tiamud_story_saves_total{{kind=\"autosave\",result=\"ok\"}} {}\n",
                "tiamud_story_saves_total{{kind=\"autosave\",result=\"failed\"}} {}\n",
                "# TYPE tiamud_story_restores_total counter\n",
                "tiamud_story_restores_total{{result=\"ok\"}} {}\n",
                "tiamud_story_restores_total{{result=\"rejected\"}} {}\n",
                "# TYPE tiamud_story_slot_rate_limited_total counter\n",
                "tiamud_story_slot_rate_limited_total {}\n",
            ),
            catalog_stories,
            self.live.load(Ordering::Relaxed),
            self.started.load(Ordering::Relaxed),
            self.closed_left.load(Ordering::Relaxed),
            self.closed_evicted.load(Ordering::Relaxed),
            self.closed_shutdown.load(Ordering::Relaxed),
            self.rejected_capacity.load(Ordering::Relaxed),
            self.rejected_account.load(Ordering::Relaxed),
            self.rejected_unknown.load(Ordering::Relaxed),
            self.inputs_accepted.load(Ordering::Relaxed),
            self.inputs_rejected.load(Ordering::Relaxed),
            self.saves_manual_ok.load(Ordering::Relaxed),
            self.saves_manual_failed.load(Ordering::Relaxed),
            self.saves_auto_ok.load(Ordering::Relaxed),
            self.saves_auto_failed.load(Ordering::Relaxed),
            self.restores_ok.load(Ordering::Relaxed),
            self.restores_rejected.load(Ordering::Relaxed),
            self.slot_rate_limited.load(Ordering::Relaxed),
        )
    }
}

// ---------------------------------------------------------------------------
// Projection
// ---------------------------------------------------------------------------

/// Transport-neutral, already perceived story view: localized items, opaque
/// per-view ids and resolved media only. Hidden variables, authored ids,
/// unavailable entries, use counters and RNG state never reach it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoryProjection {
    pub story_id: String,
    pub title: String,
    pub view_rev: u32,
    pub turn: u32,
    pub score: i64,
    pub ending: Option<StoryEndingKind>,
    pub can_undo: bool,
    /// Ordered items as produced by `story-core::view_pages`, with tokens
    /// replaced by opaque ids, plus event blocks and resolved cues.
    pub items: Vec<StoryItem>,
}

/// One numbered player action of a view, in display order. Telnet and typed
/// browser numbers address these.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StoryAction {
    Choose {
        id: String,
        label: String,
    },
    Dialogue {
        id: String,
        label: String,
    },
    Menu {
        verb: String,
        target: Option<String>,
        label: String,
    },
}

impl StoryAction {
    pub fn label(&self) -> &str {
        match self {
            Self::Choose { label, .. }
            | Self::Dialogue { label, .. }
            | Self::Menu { label, .. } => label,
        }
    }
}

impl StoryProjection {
    /// Every action the view offers, numbered from 1 in this order: gamebook
    /// choices, dialogue options, then each verb (or verb → target pair).
    pub fn actions(&self) -> Vec<StoryAction> {
        let mut actions = Vec::new();
        let mut verbs: Vec<(&str, &str)> = Vec::new();
        for item in &self.items {
            match item {
                StoryItem::Choice { id, label } => actions.push(StoryAction::Choose {
                    id: id.clone(),
                    label: label.clone(),
                }),
                StoryItem::DialogueOption { id, label } => actions.push(StoryAction::Dialogue {
                    id: id.clone(),
                    label: label.clone(),
                }),
                StoryItem::Verb {
                    id,
                    label,
                    needs_target,
                } => {
                    verbs.push((id, label));
                    if !needs_target {
                        actions.push(StoryAction::Menu {
                            verb: id.clone(),
                            target: None,
                            label: label.clone(),
                        });
                    }
                }
                StoryItem::Target { verb, id, label } => {
                    let verb_label = verbs
                        .iter()
                        .find(|(candidate, _)| candidate == verb)
                        .map_or("", |(_, label)| label);
                    actions.push(StoryAction::Menu {
                        verb: verb.clone(),
                        target: Some(id.clone()),
                        label: format!("{verb_label} → {label}"),
                    });
                }
                _ => {}
            }
        }
        actions
    }

    /// Splits the view into `story.view` payloads whose sequenced envelopes
    /// each stay within the frame budget. Items keep their order and are never
    /// split. Media cues are omitted when the client did not opt into audio.
    pub fn web_parts(&self, include_media: bool) -> Vec<protocol::StoryView> {
        let empty = || protocol::StoryView {
            view_rev: self.view_rev,
            part: 0,
            parts: 1,
            turn: self.turn,
            can_undo: self.can_undo,
            items: Vec::new(),
        };
        // Worst-case scalar digits so that renumbering parts never grows a frame.
        let base = encoded_len(&protocol::ServerEnvelope::new(
            ServerMessageType::StoryView,
            u64::MAX,
            protocol::StoryView {
                part: u16::MAX,
                parts: u16::MAX,
                ..empty()
            },
        ));
        let mut parts = vec![empty()];
        let mut size = base;
        for item in self
            .items
            .iter()
            .filter(|item| include_media || !matches!(item, StoryItem::Cue { .. }))
        {
            let cost = encoded_len(item) + 1;
            if size + cost > STORY_VIEW_FRAME_BUDGET
                && parts.last().is_some_and(|part| !part.items.is_empty())
            {
                parts.push(empty());
                size = base;
            }
            size += cost;
            parts
                .last_mut()
                .expect("parts is never empty")
                .items
                .push(item.clone());
        }
        let total = u16::try_from(parts.len()).unwrap_or(u16::MAX);
        for (index, part) in parts.iter_mut().enumerate() {
            part.part = u16::try_from(index).unwrap_or(u16::MAX);
            part.parts = total;
        }
        parts
    }
}

/// Compact JSON length; an unencodable value counts as oversized.
fn encoded_len<T: serde::Serialize>(value: &T) -> usize {
    serde_json::to_vec(value).map_or(usize::MAX / 4, |bytes| bytes.len())
}

/// Renders a projection as plain Telnet text with numbered actions. The text
/// carries every playable fact; media and GMCP are optional.
pub fn render_text(projection: &StoryProjection) -> String {
    let mut text = String::new();
    let mut stats = Vec::new();
    let mut inventory = Vec::new();
    let mut present = Vec::new();
    for item in &projection.items {
        match item {
            StoryItem::Scene {
                title, text: prose, ..
            } => {
                let _ = write!(text, "\r\n== {title} ==\r\n");
                for line in prose.split('\n').filter(|line| !line.trim().is_empty()) {
                    let _ = write!(text, "{line}\r\n");
                }
            }
            StoryItem::Block {
                kind: StoryBlockKind::Event,
                text: line,
                ..
            } => {
                let _ = write!(text, "* {line}\r\n");
            }
            StoryItem::Block {
                kind: StoryBlockKind::Line,
                speaker,
                text: line,
            } => match speaker {
                Some(speaker) => {
                    let _ = write!(text, "{speaker}: «{line}»\r\n");
                }
                None => {
                    let _ = write!(text, "{line}\r\n");
                }
            },
            StoryItem::Portrait { name } => present.push(name.as_str()),
            StoryItem::Stat { label, value } => stats.push(format!("{label}: {value}")),
            StoryItem::Item { name } => inventory.push(name.as_str()),
            _ => {}
        }
    }
    if !present.is_empty() {
        let _ = write!(text, "Presentes: {}\r\n", present.join(", "));
    }
    let mut status = format!(
        "Turno {} · Puntuación {}",
        projection.turn, projection.score
    );
    for stat in &stats {
        let _ = write!(status, " · {stat}");
    }
    let _ = write!(text, "[{status}]\r\n");
    if inventory.is_empty() {
        text.push_str("Inventario: (vacío)\r\n");
    } else {
        let _ = write!(text, "Inventario: {}\r\n", inventory.join(", "));
    }
    if let Some(ending) = projection.ending {
        let _ = write!(
            text,
            "FIN — {}. Puntuación final: {}.\r\n",
            ending_label(ending),
            projection.score
        );
        text.push_str("Escribe «salir» (leave) para volver al mundo");
        if projection.can_undo {
            text.push_str(" o «deshacer» (undo)");
        }
        text.push_str(".\r\n");
    } else {
        for (index, action) in projection.actions().iter().enumerate() {
            let _ = write!(text, "{}) {}\r\n", index + 1, action.label());
        }
        text.push_str("Escribe un número para elegir, «deshacer» (undo) o «salir» (leave).\r\n");
    }
    text
}

pub const fn ending_label(kind: StoryEndingKind) -> &'static str {
    match kind {
        StoryEndingKind::Victory => "victoria",
        StoryEndingKind::Defeat => "derrota",
        StoryEndingKind::Neutral => "final",
    }
}

// ---------------------------------------------------------------------------
// Story commands (shared by Web `command.submit` and Telnet)
// ---------------------------------------------------------------------------

/// Text commands recognised outside a story.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StoryCommand {
    List,
    Enter(String),
    Leave,
}

/// Input recognised while a session is inside a story.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StoryInputCommand {
    /// 1-based action number of the current view.
    Number(usize),
    Undo,
    Leave,
    Look,
    List,
    /// `guardar N`/`save N`; `None` when the slot is missing or malformed.
    Save(Option<u8>),
    /// `cargar N`/`load N`/`restore N`.
    Restore(Option<u8>),
    /// `partidas`/`saves`.
    Slots,
    /// `borrar N`/`delete N`.
    Delete(Option<u8>),
    /// `continuar`/`continue`: restores the autosave.
    Continue,
    Other,
}

/// Parses `historias`/`stories`, `historia <id>`/`story <id>` and
/// `salir historia`/`story leave`. Everything else stays a world command.
pub fn parse_story_command(input: &str) -> Option<StoryCommand> {
    let lower = input
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>();
    match lower
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["historias"] | ["stories"] | ["historia"] | ["story"] | ["story", "list"] => {
            Some(StoryCommand::List)
        }
        ["salir", "historia"] | ["story", "leave"] => Some(StoryCommand::Leave),
        ["historia", id] | ["story", id] => Some(StoryCommand::Enter((*id).to_owned())),
        _ => None,
    }
}

/// Parses one line typed inside a story. Menu verbs such as MIRAR are
/// numbered actions, so only `ver`/`look` re-render the view.
pub fn parse_story_input(input: &str) -> StoryInputCommand {
    let trimmed = input.trim();
    if !trimmed.is_empty() && trimmed.len() <= 3 && trimmed.bytes().all(|b| b.is_ascii_digit()) {
        return trimmed
            .parse::<usize>()
            .ok()
            .filter(|number| *number >= 1)
            .map_or(StoryInputCommand::Other, StoryInputCommand::Number);
    }
    let lower = trimmed.to_lowercase();
    let words = lower.split_whitespace().collect::<Vec<_>>();
    let slot = |word: Option<&&str>| {
        word.filter(|word| word.len() <= 2)
            .and_then(|word| word.parse::<u8>().ok())
    };
    match words.as_slice() {
        ["guardar" | "save", rest @ ..] if rest.len() <= 1 => {
            return StoryInputCommand::Save(slot(rest.first()));
        }
        ["cargar" | "load" | "restore", rest @ ..] if rest.len() <= 1 => {
            return StoryInputCommand::Restore(slot(rest.first()));
        }
        ["borrar" | "delete", rest @ ..] if rest.len() <= 1 => {
            return StoryInputCommand::Delete(slot(rest.first()));
        }
        ["partidas" | "saves"] => return StoryInputCommand::Slots,
        ["continuar" | "continue"] => return StoryInputCommand::Continue,
        _ => {}
    }
    match lower.as_str() {
        "deshacer" | "undo" | "story undo" => StoryInputCommand::Undo,
        "salir" | "leave" | "salir historia" | "story leave" => StoryInputCommand::Leave,
        "ver" | "look" | "l" => StoryInputCommand::Look,
        "historias" | "stories" => StoryInputCommand::List,
        _ => StoryInputCommand::Other,
    }
}

pub const STORY_INPUT_HELP: &str = "Estás absorto en una historia: escribe un número para elegir, «ver» (look), «deshacer» (undo), «guardar N» (save), «cargar N» (load), «partidas» (saves), «borrar N» (delete) o «salir» (leave).";

/// Renders the slot listing of the current story for text clients.
pub fn render_slots(slots: &[protocol::StorySlotSummary]) -> String {
    if slots.is_empty() {
        return "No tienes partidas guardadas de esta historia. Escribe «guardar N» (save N) con N del 1 al 9.".to_owned();
    }
    let mut text = String::from("Partidas guardadas:");
    for slot in slots {
        let name = if slot.slot == 0 {
            "0 (autoguardado)".to_owned()
        } else {
            slot.slot.to_string()
        };
        let _ = write!(text, "\n  {name} — {}", format_utc(slot.updated_at));
        if !slot.compatible {
            text.push_str(" (otra versión de la historia; no se puede cargar)");
        }
    }
    text.push_str("\nEscribe «cargar N» (load N) para cargar o «borrar N» (delete N) para borrar.");
    text
}

/// The entry offer for text clients, when a compatible autosave exists.
pub fn render_continue_offer(slots: &protocol::StorySlots) -> Option<String> {
    let autosave = slots
        .slots
        .iter()
        .find(|slot| slot.slot == 0 && slot.compatible)
        .filter(|_| slots.offer_continue)?;
    Some(format!(
        "Tienes una partida autoguardada ({}). Escribe «continuar» (continue) para retomarla o juega para empezar de nuevo.",
        format_utc(autosave.updated_at)
    ))
}

/// `YYYY-MM-DD HH:MM UTC` for Unix seconds (proleptic Gregorian calendar).
pub fn format_utc(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        rest / 3_600,
        rest % 3_600 / 60
    )
}

/// Renders the catalog listing for `historias`.
pub fn render_listing(listing: &[(String, String)]) -> String {
    let mut text = String::from("Historias disponibles:");
    for (id, title) in listing {
        let _ = write!(text, "\n  {id} — {title}");
    }
    text.push_str("\nEscribe «historia <id>» (story <id>) para empezar.");
    text
}

// ---------------------------------------------------------------------------
// Manager and instances
// ---------------------------------------------------------------------------

/// One input addressed by opaque ids of a view.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StoryRequest {
    /// `story.choose`: a gamebook choice or a dialogue option.
    Choose { id: String },
    /// `story.dialogue`: a dialogue option only.
    Dialogue { id: String },
    /// `story.menu`: a verb with an optional target of that verb.
    Menu {
        verb: String,
        target: Option<String>,
    },
}

impl StoryAction {
    pub fn request(&self) -> StoryRequest {
        match self {
            Self::Choose { id, .. } => StoryRequest::Choose { id: id.clone() },
            Self::Dialogue { id, .. } => StoryRequest::Dialogue { id: id.clone() },
            Self::Menu { verb, target, .. } => StoryRequest::Menu {
                verb: verb.clone(),
                target: target.clone(),
            },
        }
    }
}

enum Command {
    View(oneshot::Sender<StoryProjection>),
    Input {
        request: StoryRequest,
        view_rev: u32,
        reply: oneshot::Sender<Result<StoryProjection, StoryFailure>>,
    },
    Undo(oneshot::Sender<Result<StoryProjection, StoryFailure>>),
    /// The current state as the exact bytes a save slot stores.
    Snapshot(oneshot::Sender<Result<SavedState, StoryFailure>>),
    /// Replaces the state with one already validated against the pack.
    Restore {
        state: Box<StoryState>,
        reply: oneshot::Sender<Result<StoryProjection, StoryFailure>>,
    },
    /// Closes the instance and hands back its state when it has progress
    /// (turn > 0), for the autosave.
    Leave {
        reason: CloseReason,
        reply: oneshot::Sender<Option<SavedState>>,
    },
}

/// A serialized `StoryState` ready for a slot write.
#[derive(Clone, Debug)]
struct SavedState {
    json: String,
    format_version: u32,
    pack_id: String,
}

impl SavedState {
    fn of(state: &StoryState) -> Result<Self, StoryFailure> {
        let bytes = state.to_json().map_err(|_| StoryFailure::Internal)?;
        let json = String::from_utf8(bytes).map_err(|_| StoryFailure::Internal)?;
        Ok(Self {
            json,
            format_version: state.format_version,
            pack_id: state.pack_id.clone(),
        })
    }
}

struct LiveEntry {
    instance: u64,
    sender: mpsc::Sender<Command>,
    story: Arc<CatalogStory>,
}

struct ManagerInner {
    catalog: StoryCatalog,
    limits: StoryLimits,
    live: Mutex<HashMap<AccountId, LiveEntry>>,
    next_instance: AtomicU64,
    metrics: StoryMetrics,
    /// Character store holding the save slots; unset when none is attached.
    store: OnceLock<Arc<dyn CharacterStore>>,
    slot_writes: KeyedRateLimiter<AccountId>,
    slot_reads: KeyedRateLimiter<AccountId>,
    /// Set by graceful shutdown: no new instance starts afterwards.
    closing: AtomicBool,
}

impl ManagerInner {
    fn remove(&self, account: AccountId, instance: u64, reason: CloseReason) {
        let mut live = self.live.lock().unwrap_or_else(|error| error.into_inner());
        if live
            .get(&account)
            .is_some_and(|entry| entry.instance == instance)
        {
            live.remove(&account);
            self.metrics.live.store(live.len(), Ordering::Relaxed);
            match reason {
                CloseReason::Left => &self.metrics.closed_left,
                CloseReason::Evicted => &self.metrics.closed_evicted,
                CloseReason::Shutdown => &self.metrics.closed_shutdown,
            }
            .fetch_add(1, Ordering::Relaxed);
        }
    }

    fn store(&self) -> Result<Arc<dyn CharacterStore>, StoryFailure> {
        self.store
            .get()
            .cloned()
            .ok_or(StoryFailure::SlotsUnavailable)
    }

    fn admit(&self, account: AccountId, write: bool) -> Result<(), StoryFailure> {
        let limiter = if write {
            &self.slot_writes
        } else {
            &self.slot_reads
        };
        if limiter.check(account, Instant::now()) {
            Ok(())
        } else {
            self.metrics
                .slot_rate_limited
                .fetch_add(1, Ordering::Relaxed);
            Err(StoryFailure::RateLimited)
        }
    }

    /// Runs one bounded store call and maps its error to a public failure.
    async fn call<T>(
        &self,
        operation: impl Future<Output = Result<T, StoreError>>,
    ) -> Result<T, StoryFailure> {
        match tokio::time::timeout(STORE_TIMEOUT, operation).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => {
                let failure = StoryFailure::from_store(&error);
                if failure == StoryFailure::SlotStore {
                    warn!(%error, "story slot store call failed");
                }
                Err(failure)
            }
            Err(_) => {
                warn!("story slot store call timed out");
                Err(StoryFailure::SlotStore)
            }
        }
    }

    async fn write_slot(
        &self,
        account: AccountId,
        story: &CatalogStory,
        slot: StorySlot,
        saved: SavedState,
    ) -> Result<StorySaveSummary, StoryFailure> {
        let store = self.store()?;
        let result = self
            .call(store.save_story_slot(StorySaveWrite {
                account_id: account,
                story_id: story.id.clone(),
                slot,
                pack_id: saved.pack_id,
                pack_sha256: story.pack_sha256.clone(),
                state_format_version: saved.format_version,
                state_json: saved.json,
            }))
            .await;
        self.metrics.save(slot.is_autosave(), result.is_ok());
        result
    }

    /// Writes the autosave slot. Failures are logged and counted, never
    /// surfaced as a gameplay error: leaving always succeeds.
    async fn autosave(&self, account: AccountId, story: &CatalogStory, saved: SavedState) -> bool {
        if self.store.get().is_none() {
            return false;
        }
        match self
            .write_slot(account, story, StorySlot::AUTOSAVE, saved)
            .await
        {
            Ok(_) => true,
            Err(failure) => {
                warn!(?failure, "story autosave failed");
                false
            }
        }
    }

    async fn list_slots(
        &self,
        account: AccountId,
        story: &CatalogStory,
    ) -> Result<Vec<protocol::StorySlotSummary>, StoryFailure> {
        let store = self.store()?;
        let slots = self
            .call(store.list_story_slots(account, &story.id))
            .await?;
        Ok(slots
            .into_iter()
            .map(|summary| protocol::StorySlotSummary {
                slot: summary.slot.get(),
                updated_at: summary.updated_at,
                compatible: summary.pack_id == story.pack.id
                    && summary.pack_sha256 == story.pack_sha256,
            })
            .collect())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CloseReason {
    Left,
    Evicted,
    Shutdown,
}

/// Owner of the story catalog and every live instance.
#[derive(Clone)]
pub struct StoryManager {
    inner: Arc<ManagerInner>,
}

/// A session's handle to its account's live instance.
#[derive(Clone)]
pub struct StoryLink {
    account: AccountId,
    story: Arc<CatalogStory>,
    /// True when the link joined an instance that already existed (resume or
    /// re-entry); only a fresh instance offers to continue the autosave.
    resumed: bool,
    sender: mpsc::Sender<Command>,
    inner: Arc<ManagerInner>,
}

/// Result of leaving a story.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LeaveOutcome {
    /// The state was written to the autosave slot.
    pub autosaved: bool,
}

impl StoryManager {
    pub fn new(catalog: StoryCatalog, limits: StoryLimits) -> Self {
        Self {
            inner: Arc::new(ManagerInner {
                catalog,
                limits,
                live: Mutex::new(HashMap::new()),
                next_instance: AtomicU64::new(1),
                metrics: StoryMetrics::default(),
                store: OnceLock::new(),
                slot_writes: KeyedRateLimiter::per_minute_tracking(
                    SLOT_WRITES_PER_MINUTE,
                    SLOT_WRITES_PER_MINUTE,
                    MAX_TRACKED_SLOT_ACCOUNTS,
                ),
                slot_reads: KeyedRateLimiter::per_minute_tracking(
                    SLOT_READS_PER_MINUTE,
                    SLOT_READ_BURST,
                    MAX_TRACKED_SLOT_ACCOUNTS,
                ),
                closing: AtomicBool::new(false),
            }),
        }
    }

    /// Connects the save slots of ADR 0030. Without a store every slot
    /// operation answers `SlotsUnavailable` and leaving does not autosave.
    /// A second call is ignored.
    pub fn attach_store(&self, store: Arc<dyn CharacterStore>) {
        let _ = self.inner.store.set(store);
    }

    pub fn listing(&self) -> Vec<(String, String)> {
        self.inner.catalog.listing()
    }

    #[cfg(test)]
    pub fn live_instances(&self) -> usize {
        self.inner
            .live
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .len()
    }

    pub fn prometheus(&self) -> String {
        self.inner.metrics.prometheus(self.inner.catalog.len())
    }

    /// Re-attaches a session to its account's live instance, if any.
    pub fn attach(&self, account: AccountId) -> Option<StoryLink> {
        let live = self
            .inner
            .live
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        live.get(&account)
            .map(|entry| self.link(account, entry, true))
    }

    fn link(&self, account: AccountId, entry: &LiveEntry, resumed: bool) -> StoryLink {
        StoryLink {
            account,
            story: entry.story.clone(),
            resumed,
            sender: entry.sender.clone(),
            inner: self.inner.clone(),
        }
    }

    /// Starts (or resumes, for the same story) the account's instance.
    pub fn enter(&self, account: AccountId, story_id: &str) -> Result<StoryLink, StoryFailure> {
        // The seed comes from the server's secure RNG and is recorded in the
        // instance; story-core is deterministic from that point on.
        self.enter_seeded(account, story_id, OsRng.next_u64(), None)
    }

    /// Enters with an explicit story seed and, for reproducible fixtures only,
    /// a deterministic id sequence instead of the secure RNG.
    pub(crate) fn enter_seeded(
        &self,
        account: AccountId,
        story_id: &str,
        seed: u64,
        id_seed: Option<u64>,
    ) -> Result<StoryLink, StoryFailure> {
        let Some(story) = self.inner.catalog.get(story_id).cloned() else {
            self.inner
                .metrics
                .rejected_unknown
                .fetch_add(1, Ordering::Relaxed);
            return Err(StoryFailure::UnknownStory);
        };
        let mut live = self
            .inner
            .live
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(entry) = live.get(&account) {
            if entry.story.id == story.id && !entry.sender.is_closed() {
                return Ok(self.link(account, entry, true));
            }
            self.inner
                .metrics
                .rejected_account
                .fetch_add(1, Ordering::Relaxed);
            return Err(StoryFailure::AlreadyInStory);
        }
        debug_assert_eq!(MAX_INSTANCES_PER_ACCOUNT, 1);
        if self.inner.closing.load(Ordering::Acquire) {
            return Err(StoryFailure::Busy);
        }
        if live.len() >= self.inner.limits.max_instances {
            self.inner
                .metrics
                .rejected_capacity
                .fetch_add(1, Ordering::Relaxed);
            return Err(StoryFailure::Capacity);
        }
        let instance = Instance::new(story.clone(), seed, id_seed, &self.inner.catalog.media)
            .map_err(|_| StoryFailure::Internal)?;
        let instance_id = self.inner.next_instance.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = mpsc::channel(INSTANCE_MAILBOX);
        let entry = LiveEntry {
            instance: instance_id,
            sender,
            story,
        };
        let link = self.link(account, &entry, false);
        live.insert(account, entry);
        self.inner.metrics.live.store(live.len(), Ordering::Relaxed);
        self.inner.metrics.started.fetch_add(1, Ordering::Relaxed);
        drop(live);
        tokio::spawn(run_instance(
            self.inner.clone(),
            account,
            instance_id,
            instance,
            receiver,
        ));
        Ok(link)
    }

    /// Graceful shutdown: refuses new instances, closes every live instance
    /// and writes its autosave. Returns how many autosaves were written. The
    /// caller bounds the whole call; each store call is bounded too.
    pub async fn shutdown(&self) -> usize {
        self.inner.closing.store(true, Ordering::Release);
        let entries = {
            let live = self
                .inner
                .live
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            live.iter()
                .map(|(account, entry)| (*account, entry.sender.clone(), entry.story.clone()))
                .collect::<Vec<_>>()
        };
        let mut tasks = tokio::task::JoinSet::new();
        for (account, sender, story) in entries {
            let inner = self.inner.clone();
            tasks.spawn(async move {
                let (reply, response) = oneshot::channel();
                let command = Command::Leave {
                    reason: CloseReason::Shutdown,
                    reply,
                };
                let sent = tokio::time::timeout(REPLY_TIMEOUT, sender.send(command)).await;
                if !matches!(sent, Ok(Ok(()))) {
                    return false;
                }
                match tokio::time::timeout(REPLY_TIMEOUT, response).await {
                    Ok(Ok(Some(saved))) => inner.autosave(account, &story, saved).await,
                    _ => false,
                }
            });
        }
        let mut written = 0;
        while let Some(result) = tasks.join_next().await {
            written += usize::from(result.unwrap_or(false));
        }
        info!(autosaves = written, "story instances closed for shutdown");
        written
    }
}

impl StoryLink {
    pub fn story_id(&self) -> &str {
        &self.story.id
    }

    pub fn title(&self) -> &str {
        &self.story.title
    }

    async fn request<T>(
        &self,
        make: impl FnOnce(oneshot::Sender<T>) -> Command,
    ) -> Result<T, StoryFailure> {
        let (reply, response) = oneshot::channel();
        match self.sender.try_send(make(reply)) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => return Err(StoryFailure::Busy),
            Err(mpsc::error::TrySendError::Closed(_)) => return Err(StoryFailure::Gone),
        }
        match tokio::time::timeout(REPLY_TIMEOUT, response).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(_)) => Err(StoryFailure::Gone),
            Err(_) => Err(StoryFailure::Busy),
        }
    }

    /// The current view, re-sent on entry, reconnect and recovery. Its cues
    /// restore the current music only.
    pub async fn view(&self) -> Result<StoryProjection, StoryFailure> {
        self.request(Command::View).await
    }

    /// Applies one input addressed by opaque ids of view `view_rev`.
    pub async fn input(
        &self,
        request: StoryRequest,
        view_rev: u32,
    ) -> Result<StoryProjection, StoryFailure> {
        let result = self
            .request(|reply| Command::Input {
                request,
                view_rev,
                reply,
            })
            .await
            .and_then(|result| result);
        self.record(&result);
        result
    }

    #[cfg(test)]
    pub async fn choose(&self, id: &str, view_rev: u32) -> Result<StoryProjection, StoryFailure> {
        self.input(StoryRequest::Choose { id: id.to_owned() }, view_rev)
            .await
    }

    pub async fn undo(&self) -> Result<StoryProjection, StoryFailure> {
        let result = self.request(Command::Undo).await.and_then(|result| result);
        self.record(&result);
        result
    }

    /// Leaves the story. When the instance had progress and a store is
    /// attached, its state is written to the autosave slot first.
    pub async fn leave(&self) -> Result<LeaveOutcome, StoryFailure> {
        let saved = self
            .request(|reply| Command::Leave {
                reason: CloseReason::Left,
                reply,
            })
            .await?;
        let autosaved = match saved {
            Some(saved) => self.inner.autosave(self.account, &self.story, saved).await,
            None => false,
        };
        Ok(LeaveOutcome { autosaved })
    }

    /// `story.save`: writes the current state to manual slot `1..=9`.
    pub async fn save(&self, slot: u8) -> Result<protocol::StorySaved, StoryFailure> {
        let slot = StorySlot::new(slot)
            .filter(|slot| !slot.is_autosave())
            .ok_or(StoryFailure::InvalidSlot)?;
        self.inner.store()?;
        self.inner.admit(self.account, true)?;
        let saved = self
            .request(Command::Snapshot)
            .await
            .and_then(|result| result)?;
        let summary = self
            .inner
            .write_slot(self.account, &self.story, slot, saved)
            .await?;
        Ok(protocol::StorySaved {
            slot: summary.slot.get(),
            updated_at: summary.updated_at,
        })
    }

    /// `story.restore` (and `story.continue` for slot 0): loads the slot of
    /// the exact current pack revision, re-validates it against the pack and
    /// replaces the instance state. The undo history starts empty.
    pub async fn restore(&self, slot: u8) -> Result<StoryProjection, StoryFailure> {
        let result = self.restore_inner(slot).await;
        self.inner.metrics.restore(result.is_ok());
        result
    }

    async fn restore_inner(&self, slot: u8) -> Result<StoryProjection, StoryFailure> {
        let slot = StorySlot::new(slot).ok_or(StoryFailure::InvalidSlot)?;
        let store = self.inner.store()?;
        self.inner.admit(self.account, false)?;
        let compatibility = StoryPackCompatibility {
            pack_id: self.story.pack.id.clone(),
            pack_sha256: self.story.pack_sha256.clone(),
            // StoryPack declares no compatible revisions yet (ADR 0030).
            compatible_pack_sha256: BTreeSet::new(),
        };
        let record = self
            .inner
            .call(store.load_story_slot(self.account, &self.story.id, slot, compatibility))
            .await?
            .ok_or(StoryFailure::SlotEmpty)?;
        let state = StoryState::from_json_slice(record.state_json.as_bytes(), &self.story.pack)
            .map_err(|error| {
                // The reason may name authored ids; it stays in the server log.
                warn!(%error, "stored story state failed validation against its pack");
                StoryFailure::SlotCorrupt
            })?;
        self.request(|reply| Command::Restore {
            state: Box::new(state),
            reply,
        })
        .await
        .and_then(|result| result)
    }

    /// `story.slots`: metadata of the occupied slots of this story.
    pub async fn slots(&self) -> Result<Vec<protocol::StorySlotSummary>, StoryFailure> {
        self.inner.store()?;
        self.inner.admit(self.account, false)?;
        self.inner.list_slots(self.account, &self.story).await
    }

    /// `story.deleteSlot`: removes slot `0..=9` (an empty slot is an error)
    /// and returns the remaining slots. The listing is part of the admitted
    /// write and is not rate limited separately.
    pub async fn delete_slot(
        &self,
        slot: u8,
    ) -> Result<Vec<protocol::StorySlotSummary>, StoryFailure> {
        let slot = StorySlot::new(slot).ok_or(StoryFailure::InvalidSlot)?;
        let store = self.inner.store()?;
        self.inner.admit(self.account, true)?;
        if !self
            .inner
            .call(store.delete_story_slot(self.account, &self.story.id, slot))
            .await?
        {
            return Err(StoryFailure::SlotEmpty);
        }
        self.inner.list_slots(self.account, &self.story).await
    }

    /// The listing sent once after entering a fresh instance, with the
    /// continue offer when a compatible autosave exists. `None` when the
    /// instance was resumed, no store is attached or listing failed. It is
    /// not rate limited: entering is already bounded by the command limiter.
    pub async fn entry_slots(&self) -> Option<protocol::StorySlots> {
        if self.resumed || self.inner.store.get().is_none() {
            return None;
        }
        let slots = self
            .inner
            .list_slots(self.account, &self.story)
            .await
            .ok()?;
        let offer_continue = slots.iter().any(|slot| slot.slot == 0 && slot.compatible);
        Some(protocol::StorySlots {
            slots,
            offer_continue,
        })
    }

    fn record<T>(&self, result: &Result<T, StoryFailure>) {
        self.inner.metrics.input(result.is_ok());
    }
}

async fn run_instance(
    inner: Arc<ManagerInner>,
    account: AccountId,
    instance_id: u64,
    mut instance: Instance,
    mut receiver: mpsc::Receiver<Command>,
) {
    let idle = inner.limits.idle_timeout;
    loop {
        let command = match tokio::time::timeout(idle, receiver.recv()).await {
            Ok(Some(command)) => command,
            Ok(None) | Err(_) => {
                inner.remove(account, instance_id, CloseReason::Evicted);
                info!(instance = instance_id, "idle story instance evicted");
                // The instance is already unregistered; the autosave runs on
                // this exiting task and never delays a live instance.
                if let Some(saved) = instance.progress() {
                    let story = instance.story.clone();
                    inner.autosave(account, &story, saved).await;
                }
                return;
            }
        };
        match command {
            Command::View(reply) => {
                let _ = reply.send(instance.resume_projection());
            }
            Command::Input {
                request,
                view_rev,
                reply,
            } => {
                let _ = reply.send(instance.input(&request, view_rev, &inner.catalog.media));
            }
            Command::Undo(reply) => {
                let _ = reply.send(instance.undo(&inner.catalog.media));
            }
            Command::Snapshot(reply) => {
                let _ = reply.send(SavedState::of(&instance.state));
            }
            Command::Restore { state, reply } => {
                let _ = reply.send(instance.restore(*state, &inner.catalog.media));
            }
            Command::Leave { reason, reply } => {
                // Unregister before acknowledging so the session can start
                // another story immediately.
                inner.remove(account, instance_id, reason);
                let _ = reply.send(instance.progress());
                return;
            }
        }
    }
}

/// What an opaque id of the current view stands for.
#[derive(Clone, Debug)]
enum Mapped {
    Choice(String),
    Dialogue(String),
    Verb { token: String, needs_target: bool },
    Target { verb: String, token: String },
}

/// Single-writer state of one story instance.
struct Instance {
    story: Arc<CatalogStory>,
    /// Recorded seed of this playthrough (diagnostics and future save slots).
    #[allow(dead_code)]
    seed: u64,
    state: StoryState,
    undo: UndoRing,
    view_rev: u32,
    /// Opaque per-view id → story-core token of the current view.
    ids: Vec<(String, Mapped)>,
    /// Content-addressed URL of the music currently playing.
    music: Option<String>,
    current: StoryProjection,
    /// Deterministic id state for reproducible fixtures; `None` (the
    /// production path) draws every id from the secure RNG.
    id_state: Option<u64>,
}

impl Instance {
    fn new(
        story: Arc<CatalogStory>,
        seed: u64,
        id_state: Option<u64>,
        media: &BTreeMap<String, String>,
    ) -> Result<Self, tiamud_story_core::StoryError> {
        let state = StoryState::new(&story.pack, seed)?;
        let undo = UndoRing::new(UNDO_CAPACITY)?;
        // The start scene's cues play on entry like any later scene's.
        let initial_cues = core_view(&story.pack, &state)?
            .cues
            .into_iter()
            .map(StoryEvent::Cue)
            .collect::<Vec<_>>();
        let current = StoryProjection {
            story_id: story.id.clone(),
            title: story.title.clone(),
            view_rev: 0,
            turn: 0,
            score: 0,
            ending: None,
            can_undo: false,
            items: Vec::new(),
        };
        let mut instance = Self {
            story,
            seed,
            state,
            undo,
            view_rev: 0,
            ids: Vec::new(),
            music: None,
            current,
            id_state,
        };
        instance.project(&initial_cues, media)?;
        Ok(instance)
    }

    /// The current view with its transition cues replaced by the music that
    /// is playing now, so a reconnecting client resynchronizes once.
    fn resume_projection(&self) -> StoryProjection {
        let mut projection = self.current.clone();
        projection
            .items
            .retain(|item| !matches!(item, StoryItem::Cue { .. }));
        if let Some(url) = &self.music {
            let at = usize::from(!projection.items.is_empty());
            projection.items.insert(
                at,
                StoryItem::Cue {
                    action: StoryCueAction::Play,
                    channel: MediaChannel::Music,
                    url: Some(url.clone()),
                    looped: true,
                },
            );
        }
        projection
    }

    fn lookup(&self, id: &str) -> Result<&Mapped, StoryFailure> {
        if !valid_choice_id(id) {
            return Err(StoryFailure::UnknownChoice);
        }
        self.ids
            .iter()
            .find(|(candidate, _)| candidate == id)
            .map(|(_, mapped)| mapped)
            .ok_or(StoryFailure::UnknownChoice)
    }

    fn input(
        &mut self,
        request: &StoryRequest,
        view_rev: u32,
        media: &BTreeMap<String, String>,
    ) -> Result<StoryProjection, StoryFailure> {
        // Shape first, then staleness, then whether the id was published.
        let shaped = match request {
            StoryRequest::Choose { id } | StoryRequest::Dialogue { id } => valid_choice_id(id),
            StoryRequest::Menu { verb, target } => {
                valid_choice_id(verb) && target.as_deref().is_none_or(valid_choice_id)
            }
        };
        if !shaped {
            return Err(StoryFailure::UnknownChoice);
        }
        if view_rev != self.view_rev {
            return Err(StoryFailure::StaleView);
        }
        if self.state.ended.is_some() {
            return Err(StoryFailure::Ended);
        }
        let input = match request {
            StoryRequest::Choose { id } => match self.lookup(id)? {
                Mapped::Choice(token) => StoryInput::Choose {
                    token: token.clone(),
                },
                Mapped::Dialogue(token) => StoryInput::DialogueChoice {
                    token: token.clone(),
                },
                _ => return Err(StoryFailure::UnknownChoice),
            },
            StoryRequest::Dialogue { id } => match self.lookup(id)? {
                Mapped::Dialogue(token) => StoryInput::DialogueChoice {
                    token: token.clone(),
                },
                _ => return Err(StoryFailure::UnknownChoice),
            },
            StoryRequest::Menu { verb, target } => {
                let Mapped::Verb {
                    token: verb_token,
                    needs_target,
                } = self.lookup(verb)?.clone()
                else {
                    return Err(StoryFailure::UnknownChoice);
                };
                let target = match (needs_target, target) {
                    (false, None) => None,
                    (true, Some(target)) => match self.lookup(target)? {
                        Mapped::Target { verb: owner, token } if owner == verb => {
                            Some(token.clone())
                        }
                        _ => return Err(StoryFailure::UnknownChoice),
                    },
                    _ => return Err(StoryFailure::UnknownChoice),
                };
                StoryInput::MenuAction {
                    verb: verb_token,
                    target,
                }
            }
        };
        let (next, events) = step(&self.story.pack, &self.state, input).map_err(|error| {
            use tiamud_story_core::StoryError as Core;
            match error {
                Core::Ended => StoryFailure::Ended,
                Core::StaleChoice => StoryFailure::StaleView,
                Core::UnknownChoice
                | Core::MalformedToken
                | Core::BadTarget
                | Core::InDialogue
                | Core::NotInDialogue => StoryFailure::UnknownChoice,
                error => {
                    warn!(%error, "story step failed");
                    StoryFailure::Internal
                }
            }
        })?;
        let previous = std::mem::replace(&mut self.state, next);
        let music = self.music.clone();
        self.undo.push(previous.clone());
        if self.project(&events, media).is_err() {
            // Restore the exact previous state; a projection failure never
            // leaves a half-applied transition.
            self.state = previous;
            self.music = music;
            self.undo.pop();
            return Err(StoryFailure::Internal);
        }
        Ok(self.current.clone())
    }

    /// The state to autosave when the playthrough has progressed.
    fn progress(&self) -> Option<SavedState> {
        if self.state.turn == 0 {
            return None;
        }
        SavedState::of(&self.state)
            .inspect_err(|_| warn!("story state could not be serialized for the autosave"))
            .ok()
    }

    /// Replaces the state with a restored one (already validated against the
    /// pack). The undo history is not persisted, so it restarts empty; the
    /// restored scene's cues play as on entry and earlier music stops.
    fn restore(
        &mut self,
        state: StoryState,
        media: &BTreeMap<String, String>,
    ) -> Result<StoryProjection, StoryFailure> {
        let scene_cues = core_view(&self.story.pack, &state)
            .map_err(|_| StoryFailure::SlotCorrupt)?
            .cues;
        let undo = UndoRing::new(UNDO_CAPACITY).map_err(|_| StoryFailure::Internal)?;
        let mut events = Vec::with_capacity(scene_cues.len() + 1);
        if self.music.is_some() {
            events.push(StoryEvent::Cue(Cue::StopMusic));
        }
        events.extend(scene_cues.into_iter().map(StoryEvent::Cue));
        let previous_state = std::mem::replace(&mut self.state, state);
        let previous_undo = std::mem::replace(&mut self.undo, undo);
        let music = self.music.clone();
        if self.project(&events, media).is_err() {
            self.state = previous_state;
            self.undo = previous_undo;
            self.music = music;
            return Err(StoryFailure::SlotCorrupt);
        }
        Ok(self.current.clone())
    }

    fn undo(&mut self, media: &BTreeMap<String, String>) -> Result<StoryProjection, StoryFailure> {
        let Some(previous) = self.undo.pop() else {
            return Err(StoryFailure::NothingToUndo);
        };
        let current = std::mem::replace(&mut self.state, previous);
        if self.project(&[], media).is_err() {
            self.state = current;
            return Err(StoryFailure::Internal);
        }
        Ok(self.current.clone())
    }

    fn next_id(&mut self, taken: &[(String, Mapped)]) -> String {
        loop {
            let candidate = format!("c{:016x}", next_id_word(&mut self.id_state));
            if !taken.iter().any(|(existing, _)| existing == &candidate) {
                return candidate;
            }
        }
    }

    /// Projects the current state through `story-core::view_pages` and
    /// publishes fresh opaque ids under a new view revision. Ids of earlier
    /// views stop resolving.
    fn project(
        &mut self,
        events: &[StoryEvent],
        media: &BTreeMap<String, String>,
    ) -> Result<(), tiamud_story_core::StoryError> {
        let story = self.story.clone();
        let pack = &story.pack;
        let pages = view_pages(pack, &self.state, STORY_LOCALE)?;
        let event_items = events
            .iter()
            .filter_map(|event| render_event(pack, event))
            .map(|text| StoryItem::Block {
                kind: StoryBlockKind::Event,
                speaker: None,
                text: bounded_text(&text),
            })
            .collect::<Vec<_>>();
        let cue_items = events
            .iter()
            .filter_map(|event| match event {
                StoryEvent::Cue(cue) => Some(cue),
                _ => None,
            })
            .filter_map(|cue| self.resolve_cue(cue, media))
            .collect::<Vec<_>>();
        let mut ids: Vec<(String, Mapped)> = Vec::new();
        let mut verb_ids: Vec<(String, String)> = Vec::new();
        let mut items = Vec::new();
        let mut title = story.title.clone();
        let mut score = self.state.score;
        let mut ending = None;
        let mut turn = self.state.turn;
        let mut extras = Some((event_items, cue_items));
        for page in pages {
            turn = page.turn;
            for item in page.items {
                match item {
                    PageItem::Scene {
                        title: scene_title,
                        text,
                        score: scene_score,
                        ending: scene_ending,
                        ..
                    } => {
                        title = scene_title.clone();
                        score = scene_score;
                        ending = scene_ending.map(|ending| match ending.kind {
                            EndingKind::Victory => StoryEndingKind::Victory,
                            EndingKind::Defeat => StoryEndingKind::Defeat,
                            EndingKind::Neutral => StoryEndingKind::Neutral,
                        });
                        items.push(StoryItem::Scene {
                            title: scene_title,
                            text,
                            score: scene_score,
                            ending: ending.map(|kind| StoryEnding { kind }),
                        });
                        if let Some((events, cues)) = extras.take() {
                            items.extend(cues);
                            items.extend(events);
                        }
                    }
                    // Scene cues are emitted as events on entry; resending
                    // them with every view would restart music.
                    PageItem::Cue(_) => {}
                    PageItem::Portrait { name, .. } => items.push(StoryItem::Portrait { name }),
                    PageItem::Block { speaker, text, .. } => items.push(StoryItem::Block {
                        kind: StoryBlockKind::Line,
                        speaker,
                        text,
                    }),
                    PageItem::Stat { label, value } => items.push(StoryItem::Stat {
                        label,
                        value: render_value(value),
                    }),
                    PageItem::Item { name } => items.push(StoryItem::Item { name }),
                    PageItem::Choice { token, label } => {
                        let id = self.next_id(&ids);
                        ids.push((id.clone(), Mapped::Choice(token)));
                        items.push(StoryItem::Choice { id, label });
                    }
                    PageItem::DialogueOption { token, label } => {
                        let id = self.next_id(&ids);
                        ids.push((id.clone(), Mapped::Dialogue(token)));
                        items.push(StoryItem::DialogueOption { id, label });
                    }
                    PageItem::Verb {
                        token,
                        label,
                        needs_target,
                    } => {
                        let id = self.next_id(&ids);
                        verb_ids.push((token.clone(), id.clone()));
                        ids.push((
                            id.clone(),
                            Mapped::Verb {
                                token,
                                needs_target,
                            },
                        ));
                        items.push(StoryItem::Verb {
                            id,
                            label,
                            needs_target,
                        });
                    }
                    PageItem::Target { verb, token, label } => {
                        let Some(verb_id) = verb_ids
                            .iter()
                            .find(|(core, _)| core == &verb)
                            .map(|(_, id)| id.clone())
                        else {
                            return Err(tiamud_story_core::StoryError::InvalidPack(
                                "menu target precedes its verb".to_owned(),
                            ));
                        };
                        let id = self.next_id(&ids);
                        ids.push((
                            id.clone(),
                            Mapped::Target {
                                verb: verb_id.clone(),
                                token,
                            },
                        ));
                        items.push(StoryItem::Target {
                            verb: verb_id,
                            id,
                            label,
                        });
                    }
                }
            }
        }
        self.view_rev = self.view_rev.saturating_add(1);
        self.ids = ids;
        self.current = StoryProjection {
            story_id: story.id.clone(),
            title,
            view_rev: self.view_rev,
            turn,
            score,
            ending,
            can_undo: !self.undo.is_empty(),
            items,
        };
        Ok(())
    }

    fn resolve_cue(&mut self, cue: &Cue, media: &BTreeMap<String, String>) -> Option<StoryItem> {
        match cue {
            Cue::Music { asset } => match media.get(asset) {
                Some(url) => {
                    self.music = Some(url.clone());
                    Some(StoryItem::Cue {
                        action: StoryCueAction::Play,
                        channel: MediaChannel::Music,
                        url: Some(url.clone()),
                        looped: true,
                    })
                }
                // An unreviewed track never plays; the previous one stops.
                None => self.stop_music(),
            },
            Cue::Sfx { asset } => media.get(asset).map(|url| StoryItem::Cue {
                action: StoryCueAction::Play,
                channel: MediaChannel::Effects,
                url: Some(url.clone()),
                looped: false,
            }),
            Cue::StopMusic => self.stop_music(),
        }
    }

    fn stop_music(&mut self) -> Option<StoryItem> {
        self.music.take().map(|_| StoryItem::Cue {
            action: StoryCueAction::Stop,
            channel: MediaChannel::Music,
            url: None,
            looped: false,
        })
    }
}

fn next_id_word(state: &mut Option<u64>) -> u64 {
    match state {
        None => OsRng.next_u64(),
        Some(state) => {
            // SplitMix64 step; used only for reproducible test fixtures.
            *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = *state;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        }
    }
}

fn valid_choice_id(id: &str) -> bool {
    id.len() == STORY_CHOICE_ID_BYTES
        && id.starts_with('c')
        && id[1..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn resolve(text: &tiamud_story_core::LocalizedText) -> String {
    text.resolve(STORY_LOCALE).unwrap_or_default().to_owned()
}

fn render_value(value: Value) -> String {
    match value {
        Value::Int(value) => value.to_string(),
        Value::Bool(true) => "sí".to_owned(),
        Value::Bool(false) => "no".to_owned(),
    }
}

/// Bounds a rendered event line on a char boundary. Event lines are short by
/// construction (labels are at most 256 bytes); this is defensive only.
fn bounded_text(text: &str) -> String {
    let mut cut = text.len().min(MAX_STORY_BLOCK_BYTES);
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    text[..cut].to_owned()
}

/// Renders the player-visible part of one event. Hidden variables, node
/// entries, menu bookkeeping and media cues produce no text (menu responses
/// and dialogue lines arrive as `line` blocks of the view itself).
fn render_event(pack: &StoryPack, event: &StoryEvent) -> Option<String> {
    match event {
        StoryEvent::VariableChanged { var, from, to, .. } => {
            let decl = pack
                .variables
                .iter()
                .find(|decl| &decl.id == var && decl.visible)?;
            let label = decl
                .label
                .as_ref()
                .map(resolve)
                .unwrap_or_else(|| decl.id.clone());
            Some(format!(
                "{label}: {} → {}",
                render_value(*from),
                render_value(*to)
            ))
        }
        StoryEvent::ItemGained { item } => pack
            .items
            .iter()
            .find(|decl| &decl.id == item)
            .map(|decl| format!("Obtienes: {}", resolve(&decl.name))),
        StoryEvent::ItemLost { item } => pack
            .items
            .iter()
            .find(|decl| &decl.id == item)
            .map(|decl| format!("Pierdes: {}", resolve(&decl.name))),
        StoryEvent::DiceRolled {
            count,
            sides,
            rolls,
            modifier,
            total,
            passed,
        } => {
            let mut text = format!("Tirada {count}d{sides}");
            if *modifier != 0 {
                let _ = write!(text, "{modifier:+}");
            }
            let faces = rolls
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            let _ = write!(text, ": [{faces}] → total {total}");
            if let Some(passed) = passed {
                text.push_str(if *passed { " (éxito)" } else { " (fallo)" });
            }
            Some(text)
        }
        StoryEvent::ScoreChanged { delta, total } => {
            Some(format!("Puntuación {delta:+} (total {total})"))
        }
        StoryEvent::NodeEntered { .. }
        | StoryEvent::Cue(_)
        | StoryEvent::Ended(_)
        | StoryEvent::MenuAnswered { .. }
        | StoryEvent::DialogueEntered { .. }
        | StoryEvent::DialogueExited { .. } => None,
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod slot_tests;
