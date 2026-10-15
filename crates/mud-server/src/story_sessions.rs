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
//! An instance may be shared (ADR 0033, phase S7): the host shares it and
//! other characters join by invitation or with an opaque join code, seated
//! (at most eight) or as read-only spectators (at most 32). The instance
//! task stays the single writer; its [`shared::Table`] decides which action is
//! applied (host decides, majority vote with a bounded window, or round-robin
//! turns) and every participant's session follows a `watch` of the latest
//! shared frame, so slow sessions coalesce instead of queueing.
//!
//! The ruleset is `tiamud-story-core`; this module only serializes inputs,
//! projects views and enforces bounds. It never reads `tiamud-core` world
//! state and the world actor never sees story input.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fmt::Write as _,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

pub mod shared;
use shared::{SharedLimits, Table};

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
    StoryCueAction, StoryDecisionMode, StoryEnding, StoryEndingKind, StoryErrorCode, StoryItem,
    StorySeatRole,
};
use tiamud_story_core::{
    Cue, EndingKind, MAX_PACK_BYTES, PageItem, StoryEvent, StoryInput, StoryPack, StoryState,
    UndoRing, Value, step, step_command, view as core_view, view_pages,
};
use tokio::sync::{mpsc, oneshot, watch};
use tracing::{info, warn};

/// Default locale of a story instance when the pack offers it (otherwise the
/// pack's first locale). A session may pick another one of the pack's
/// locales when it enters (`historia <id> <locale>`).
pub const STORY_LOCALE: &str = "es";
/// Maximum bytes of a requested story locale tag.
const MAX_LOCALE_BYTES: usize = 35;
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
/// Maximum reviewed images (backgrounds and portraits) of the media map.
const MAX_MEDIA_IMAGES: usize = 256;
/// Maximum locales of one image's alternative text.
const MAX_IMAGE_ALT_LOCALES: usize = 16;
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
/// Clock period of a shared instance (presence, vote windows, turns).
const SHARED_TICK: Duration = Duration::from_millis(500);
/// Join attempts per account and minute (codes are 40-bit secrets).
pub const JOINS_PER_MINUTE: u32 = 10;
const JOIN_BURST: u32 = 5;
/// Invitations sent per host account and minute.
pub const INVITES_PER_MINUTE: u32 = 10;
const INVITE_BURST: u32 = 5;
/// Pending invitations queued per connected character.
const INVITATION_INBOX: usize = 4;
/// Registered invitation inboxes (one per connected story-capable session).
const MAX_INBOXES: usize = 16_384;
/// Maximum bytes of one parser command line (StoryPack v3).
pub const MAX_COMMAND_BYTES: usize = protocol::MAX_STORY_COMMAND_BYTES;
/// Parser command lines per account and minute, and their burst. Typing is
/// slower than this; the bound keeps a script from monopolizing an instance.
pub const COMMANDS_PER_MINUTE: u32 = 120;
const COMMAND_BURST: u32 = 20;
/// Depth of the catalog scan: the directory and its direct subdirectories.
const CATALOG_SUBDIRECTORY_DEPTH: usize = 1;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoryLimits {
    pub max_instances: usize,
    pub idle_timeout: Duration,
    /// Seats, spectators, vote window and seat grace of shared instances.
    pub shared: SharedLimits,
}

impl Default for StoryLimits {
    fn default() -> Self {
        Self {
            max_instances: DEFAULT_MAX_INSTANCES,
            idle_timeout: Duration::from_secs(DEFAULT_IDLE_SECONDS),
            shared: SharedLimits::default(),
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
    #[error(
        "TIAMUD_STORY_MAX_SEATS must be an integer in {}..={}",
        shared::MIN_SEATS,
        protocol::MAX_STORY_SEATS
    )]
    InvalidMaxSeats,
    #[error(
        "TIAMUD_STORY_MAX_SPECTATORS must be an integer in 0..={}",
        protocol::MAX_STORY_SPECTATORS
    )]
    InvalidMaxSpectators,
    #[error(
        "TIAMUD_STORY_VOTE_SECONDS must be an integer in {}..={}",
        protocol::MIN_STORY_VOTE_SECONDS,
        protocol::MAX_STORY_VOTE_SECONDS
    )]
    InvalidVoteSeconds,
    #[error(
        "TIAMUD_STORY_SEAT_GRACE_SECONDS must be an integer in {}..={}",
        shared::MIN_GRACE_SECONDS,
        shared::MAX_GRACE_SECONDS
    )]
    InvalidGraceSeconds,
}

/// Raw shared-instance settings (`TIAMUD_STORY_MAX_SEATS`,
/// `TIAMUD_STORY_MAX_SPECTATORS`, `TIAMUD_STORY_VOTE_SECONDS`,
/// `TIAMUD_STORY_SEAT_GRACE_SECONDS`).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SharedStoryEnv {
    pub max_seats: Option<String>,
    pub max_spectators: Option<String>,
    pub vote_seconds: Option<String>,
    pub grace_seconds: Option<String>,
}

impl SharedStoryEnv {
    fn any(&self) -> bool {
        self.max_seats.is_some()
            || self.max_spectators.is_some()
            || self.vote_seconds.is_some()
            || self.grace_seconds.is_some()
    }
}

/// Applies the shared-instance settings to `config`. Out-of-range values
/// fail closed; settings without a story directory are refused.
pub fn apply_shared_story_env(
    config: &mut StoryConfig,
    env: &SharedStoryEnv,
) -> Result<(), StoryConfigError> {
    fn ranged<T: std::str::FromStr + PartialOrd>(
        value: Option<&str>,
        range: std::ops::RangeInclusive<T>,
        error: StoryConfigError,
    ) -> Result<Option<T>, StoryConfigError> {
        value
            .map(|value| {
                value
                    .parse::<T>()
                    .ok()
                    .filter(|value| range.contains(value))
                    .ok_or(error)
            })
            .transpose()
    }
    if config.directory.is_none() && env.any() {
        return Err(StoryConfigError::LimitsWithoutDirectory);
    }
    let limits = &mut config.limits.shared;
    if let Some(seats) = ranged(
        env.max_seats.as_deref(),
        shared::MIN_SEATS..=usize::from(protocol::MAX_STORY_SEATS),
        StoryConfigError::InvalidMaxSeats,
    )? {
        limits.max_seats = seats;
    }
    if let Some(spectators) = ranged(
        env.max_spectators.as_deref(),
        0..=usize::from(protocol::MAX_STORY_SPECTATORS),
        StoryConfigError::InvalidMaxSpectators,
    )? {
        limits.max_spectators = spectators;
    }
    if let Some(seconds) = ranged(
        env.vote_seconds.as_deref(),
        protocol::MIN_STORY_VOTE_SECONDS..=protocol::MAX_STORY_VOTE_SECONDS,
        StoryConfigError::InvalidVoteSeconds,
    )? {
        limits.vote_seconds = seconds;
    }
    if let Some(seconds) = ranged(
        env.grace_seconds.as_deref(),
        shared::MIN_GRACE_SECONDS..=shared::MAX_GRACE_SECONDS,
        StoryConfigError::InvalidGraceSeconds,
    )? {
        limits.grace = Duration::from_secs(seconds);
    }
    Ok(())
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
    /// Default language of an instance: the parser lexicon (`es` when the
    /// pack has one, else its first lexicon) and the locale its views resolve.
    language: String,
    /// Every language an instance may play in, default first: the parser
    /// lexicons of a v3 world, else the locales of the pack title.
    locales: Vec<String>,
    /// Earlier revisions (SHA-256) whose save slots still load after
    /// re-validation, as declared by the pack (StoryPack v3).
    compatible: BTreeSet<String>,
}

impl CatalogStory {
    /// True for a StoryPack v3 with a parser world: it accepts free text.
    pub fn is_parser(&self) -> bool {
        self.pack.world.is_some()
    }

    /// The default instance language (`es` unless the pack only declares others).
    #[cfg(test)]
    pub fn language(&self) -> &str {
        &self.language
    }

    /// Every language an instance may play in, default first.
    pub fn locales(&self) -> &[String] {
        &self.locales
    }

    /// The pack title in `locale`.
    pub fn title_in(&self, locale: &str) -> String {
        self.pack
            .title
            .resolve(locale)
            .unwrap_or(self.id.as_str())
            .to_owned()
    }

    /// The language a new instance plays in: the default without a request,
    /// else the requested locale (or its language part, `en-GB` → `en`) when
    /// the pack offers it. Anything else is [`StoryFailure::UnknownLocale`].
    pub fn resolve_locale(&self, requested: Option<&str>) -> Result<String, StoryFailure> {
        let Some(requested) = requested else {
            return Ok(self.language.clone());
        };
        if requested.is_empty() || requested.len() > MAX_LOCALE_BYTES {
            return Err(StoryFailure::UnknownLocale);
        }
        let wanted = requested.to_ascii_lowercase();
        let language = wanted
            .split_once('-')
            .map_or(wanted.as_str(), |(head, _)| head);
        self.locales
            .iter()
            .find(|locale| **locale == wanted)
            .or_else(|| self.locales.iter().find(|locale| **locale == language))
            .cloned()
            .ok_or(StoryFailure::UnknownLocale)
    }
}

/// Every language of `pack`, `default` first and without duplicates: its
/// parser lexicons, or the locales of its title when it has no parser.
fn pack_locales(pack: &StoryPack, default: &str) -> Vec<String> {
    let declared: Vec<String> = match &pack.world {
        Some(world) => world
            .lexicons
            .iter()
            .map(|lexicon| lexicon.id.to_ascii_lowercase())
            .collect(),
        None => pack
            .title
            .0
            .keys()
            .map(|locale| locale.to_ascii_lowercase())
            .collect(),
    };
    let mut locales = vec![default.to_owned()];
    for locale in declared {
        if !locales.contains(&locale) {
            locales.push(locale);
        }
    }
    locales
}

/// The default language of an instance of `pack`: Spanish when the parser
/// world has a Spanish lexicon (or, without a parser, when the title has a
/// Spanish text or no locale at all), else its first lexicon or title locale.
fn pack_language(pack: &StoryPack) -> String {
    let Some(world) = &pack.world else {
        if pack
            .title
            .0
            .keys()
            .any(|locale| locale.eq_ignore_ascii_case(STORY_LOCALE))
        {
            return STORY_LOCALE.to_owned();
        }
        return pack.title.0.keys().next().map_or_else(
            || STORY_LOCALE.to_owned(),
            |locale| locale.to_ascii_lowercase(),
        );
    };
    if world
        .lexicons
        .iter()
        .any(|lexicon| lexicon.id == STORY_LOCALE)
    {
        return STORY_LOCALE.to_owned();
    }
    world
        .lexicons
        .first()
        .map_or_else(|| STORY_LOCALE.to_owned(), |lexicon| lexicon.id.clone())
}

/// A reviewed story image of the media map: the content-addressed pipeline
/// manifest (`/assets/<sha256>/manifest.json`) and its localized alt text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoryImageAsset {
    manifest: String,
    alt: tiamud_story_core::LocalizedText,
}

/// The reviewed story media map (`story-media.json`): audio cues and, from
/// format 2, background and portrait images. An id that is not listed never
/// produces media; the text of the view still carries every playable fact.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StoryMedia {
    /// Authored asset id → content-addressed `/assets/<sha256>/audio-*` URL.
    audio: BTreeMap<String, String>,
    /// Authored image id → reviewed manifest and alt text.
    images: BTreeMap<String, StoryImageAsset>,
}

impl From<BTreeMap<String, String>> for StoryMedia {
    fn from(audio: BTreeMap<String, String>) -> Self {
        Self {
            audio,
            images: BTreeMap::new(),
        }
    }
}

impl StoryMedia {
    /// The wire image of `id` with its alt text in `locale`, when reviewed.
    pub fn image(&self, id: &str, locale: &str) -> Option<protocol::StoryImage> {
        let asset = self.images.get(id)?;
        Some(protocol::StoryImage {
            manifest: asset.manifest.clone(),
            alt: asset.alt.resolve(locale)?.to_owned(),
        })
    }
}

/// Validated StoryPacks plus the reviewed media map for their cues and images.
#[derive(Debug, Default)]
pub struct StoryCatalog {
    stories: BTreeMap<String, Arc<CatalogStory>>,
    media: StoryMedia,
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
    #[serde(default)]
    assets: BTreeMap<String, String>,
    /// Format 2: reviewed background and portrait images.
    #[serde(default)]
    images: BTreeMap<String, StoryMediaImage>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoryMediaImage {
    /// `assets/<sha256>/manifest.json`, published by the asset pipeline.
    manifest: String,
    /// Locale → alternative text (1..=16 locales, 1..=512 bytes each).
    alt: BTreeMap<String, String>,
}

impl StoryCatalog {
    /// Loads every `*.story.json` in `directory` and in its direct
    /// subdirectories (one level, real directories only, never symlinks;
    /// for example `parser/` for StoryPack v3 parser adventures), validating
    /// each pack with `story-core`, plus the optional reviewed media map of
    /// the top directory. At most 1,024 entries are scanned in total. Any
    /// invalid pack fails the whole load: the server never starts with a
    /// partially trusted catalog.
    pub fn load_dir(directory: &Path) -> Result<Self, StoryCatalogError> {
        let io_error = |path: &Path, error: std::io::Error| StoryCatalogError::Io {
            path: path.display().to_string(),
            message: error.to_string(),
        };
        // (relative name, depth) of every scanned file; directories are
        // queued breadth-first and bounded by the same entry budget.
        let mut names = Vec::new();
        let mut scanned = 0_usize;
        let mut pending = vec![(PathBuf::new(), 0_usize)];
        while let Some((relative, depth)) = pending.pop() {
            let current = directory.join(&relative);
            for entry in std::fs::read_dir(&current).map_err(|error| io_error(&current, error))? {
                scanned = scanned.saturating_add(1);
                if scanned > MAX_CATALOG_DIRECTORY_ENTRIES {
                    return Err(StoryCatalogError::TooManyEntries);
                }
                let entry = entry.map_err(|error| io_error(&current, error))?;
                let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                    continue;
                };
                let kind = entry
                    .file_type()
                    .map_err(|error| io_error(&current, error))?;
                if kind.is_dir() {
                    if depth < CATALOG_SUBDIRECTORY_DEPTH && !name.starts_with('.') {
                        pending.push((relative.join(&name), depth + 1));
                    }
                } else {
                    names.push((relative.join(&name), depth));
                }
            }
        }
        names.sort();
        let mut packs = Vec::new();
        let mut media = StoryMedia::default();
        for (relative, depth) in names {
            let path = directory.join(&relative);
            let name = relative.to_string_lossy().into_owned();
            if depth == 0 && name == STORY_MEDIA_FILE {
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
        media: impl Into<StoryMedia>,
    ) -> Result<Self, StoryCatalogError> {
        let media = media.into();
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
        media: StoryMedia,
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
            let language = pack_language(&pack);
            let locales = pack_locales(&pack, &language);
            let compatible = pack.compatible_revisions.iter().cloned().collect();
            if stories
                .insert(
                    id.clone(),
                    Arc::new(CatalogStory {
                        id: id.clone(),
                        title,
                        pack,
                        pack_sha256,
                        language,
                        locales,
                        compatible,
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

fn parse_media_manifest(bytes: &[u8]) -> Result<StoryMedia, StoryCatalogError> {
    let manifest: StoryMediaManifest = serde_json::from_slice(bytes)
        .map_err(|_| StoryCatalogError::Media("malformed JSON document".to_owned()))?;
    if !(1..=2).contains(&manifest.format_version) {
        return Err(StoryCatalogError::Media(
            "formatVersion must be 1 or 2".to_owned(),
        ));
    }
    if manifest.format_version < 2 && !manifest.images.is_empty() {
        return Err(StoryCatalogError::Media(
            "images need formatVersion 2".to_owned(),
        ));
    }
    if manifest.images.len() > MAX_MEDIA_IMAGES {
        return Err(StoryCatalogError::Media(format!(
            "at most {MAX_MEDIA_IMAGES} images are allowed"
        )));
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
    let mut images = BTreeMap::new();
    for (id, image) in manifest.images {
        if !valid_asset_id(&id) || !safe_image_manifest_path(&image.manifest) {
            return Err(StoryCatalogError::Media(format!(
                "image {id:?} must map to assets/<sha256>/manifest.json"
            )));
        }
        if !valid_image_alt(&image.alt) {
            return Err(StoryCatalogError::Media(format!(
                "image {id:?} needs alt text: 1..={MAX_IMAGE_ALT_LOCALES} locales of 1..={} bytes without control characters",
                protocol::MAX_STORY_IMAGE_ALT_BYTES
            )));
        }
        images.insert(
            id,
            StoryImageAsset {
                manifest: format!("/{}", image.manifest),
                alt: tiamud_story_core::LocalizedText(image.alt),
            },
        );
    }
    Ok(StoryMedia {
        audio: media,
        images,
    })
}

/// `assets/<64 lowercase hex>/manifest.json`: a pipeline-published bundle.
fn safe_image_manifest_path(path: &str) -> bool {
    path.strip_prefix("assets/")
        .and_then(|rest| rest.strip_suffix("/manifest.json"))
        .is_some_and(|digest| {
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
}

fn valid_image_alt(alt: &BTreeMap<String, String>) -> bool {
    let mut seen = BTreeSet::new();
    !alt.is_empty()
        && alt.len() <= MAX_IMAGE_ALT_LOCALES
        && alt.iter().all(|(locale, text)| {
            let bytes = locale.as_bytes();
            (2..=MAX_LOCALE_BYTES).contains(&bytes.len())
                && bytes[0].is_ascii_alphabetic()
                && bytes
                    .iter()
                    .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
                && seen.insert(locale.to_ascii_lowercase())
                && !text.trim().is_empty()
                && text.len() <= protocol::MAX_STORY_IMAGE_ALT_BYTES
                && !text.chars().any(char::is_control)
        })
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
    /// Shared instance: only the host may do this.
    HostOnly,
    /// Shared instance: spectators are read-only.
    Spectator,
    /// Shared instance, turns mode: another seat decides now.
    NotYourTurn,
    /// No shared instance accepts that join code.
    UnknownCode,
    SeatsFull,
    /// The operation needs a shared instance.
    NotShared,
    /// No participant has that name (or it names the host).
    UnknownMember,
    /// A shared-instance setting is out of range.
    InvalidSetting,
    /// `story.command` in a story without a parser world.
    NoParser,
    /// The story is not offered in the requested language.
    UnknownLocale,
    /// A command line that is too long, or free text while no prompt is
    /// shown (an active dialogue).
    InvalidCommand,
    /// Too many parser command lines from this account.
    CommandsRateLimited,
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
            Self::HostOnly => StoryErrorCode::HostOnly,
            Self::Spectator => StoryErrorCode::Spectator,
            Self::NotYourTurn => StoryErrorCode::NotYourTurn,
            Self::UnknownCode => StoryErrorCode::UnknownCode,
            Self::SeatsFull => StoryErrorCode::SeatsFull,
            Self::NotShared => StoryErrorCode::NotShared,
            Self::UnknownMember => StoryErrorCode::UnknownMember,
            Self::InvalidSetting => StoryErrorCode::InvalidSetting,
            Self::NoParser => StoryErrorCode::NoParser,
            Self::InvalidCommand => StoryErrorCode::InvalidCommand,
            Self::UnknownLocale => StoryErrorCode::UnknownLocale,
            Self::CommandsRateLimited => StoryErrorCode::RateLimited,
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
            Self::HostOnly => "Solo el anfitrión de la historia compartida puede hacer eso.",
            Self::Spectator => "Los espectadores solo pueden mirar.",
            Self::NotYourTurn => "No es tu turno; espera a que decida quien tiene el turno.",
            Self::UnknownCode => "Ese código no corresponde a ninguna historia compartida.",
            Self::SeatsFull => "No quedan plazas libres en esa historia compartida.",
            Self::NotShared => {
                "Esta historia no está compartida. Escribe «compartir» (share) para abrirla a otros."
            }
            Self::UnknownMember => "No hay ningún participante con ese nombre.",
            Self::InvalidSetting => {
                "Valor no válido: el plazo de votación va de 30 a 120 segundos."
            }
            Self::NoParser => {
                "Esta historia no entiende texto libre: elige una de las acciones numeradas."
            }
            Self::InvalidCommand => {
                "Escribe una orden de hasta 256 bytes mientras la historia muestre el indicador «>»."
            }
            Self::UnknownLocale => {
                "Esta historia no está disponible en ese idioma. Escribe «historia <id>» para jugarla en su idioma predeterminado."
            }
            Self::CommandsRateLimited => "Demasiadas órdenes seguidas. Espera un momento.",
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
    shared_instances: AtomicUsize,
    shared_started: AtomicU64,
    shared_joins_ok: AtomicU64,
    shared_joins_rejected: AtomicU64,
    shared_votes: AtomicU64,
    shared_decisions: AtomicU64,
    shared_invitations: AtomicU64,
    shared_kicks: AtomicU64,
    shared_seat_lapses: AtomicU64,
    commands_accepted: AtomicU64,
    commands_rejected: AtomicU64,
    commands_rate_limited: AtomicU64,
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
                "# TYPE tiamud_story_shared_instances gauge\n",
                "tiamud_story_shared_instances {}\n",
                "# TYPE tiamud_story_shared_started_total counter\n",
                "tiamud_story_shared_started_total {}\n",
                "# TYPE tiamud_story_shared_joins_total counter\n",
                "tiamud_story_shared_joins_total{{result=\"ok\"}} {}\n",
                "tiamud_story_shared_joins_total{{result=\"rejected\"}} {}\n",
                "# TYPE tiamud_story_shared_votes_total counter\n",
                "tiamud_story_shared_votes_total {}\n",
                "# TYPE tiamud_story_shared_decisions_total counter\n",
                "tiamud_story_shared_decisions_total {}\n",
                "# TYPE tiamud_story_shared_invitations_total counter\n",
                "tiamud_story_shared_invitations_total {}\n",
                "# TYPE tiamud_story_shared_kicks_total counter\n",
                "tiamud_story_shared_kicks_total {}\n",
                "# TYPE tiamud_story_shared_seat_lapses_total counter\n",
                "tiamud_story_shared_seat_lapses_total {}\n",
                "# TYPE tiamud_story_commands_total counter\n",
                "tiamud_story_commands_total{{result=\"accepted\"}} {}\n",
                "tiamud_story_commands_total{{result=\"rejected\"}} {}\n",
                "# TYPE tiamud_story_command_rate_limited_total counter\n",
                "tiamud_story_command_rate_limited_total {}\n",
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
            self.shared_instances.load(Ordering::Relaxed),
            self.shared_started.load(Ordering::Relaxed),
            self.shared_joins_ok.load(Ordering::Relaxed),
            self.shared_joins_rejected.load(Ordering::Relaxed),
            self.shared_votes.load(Ordering::Relaxed),
            self.shared_decisions.load(Ordering::Relaxed),
            self.shared_invitations.load(Ordering::Relaxed),
            self.shared_kicks.load(Ordering::Relaxed),
            self.shared_seat_lapses.load(Ordering::Relaxed),
            self.commands_accepted.load(Ordering::Relaxed),
            self.commands_rejected.load(Ordering::Relaxed),
            self.commands_rate_limited.load(Ordering::Relaxed),
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
    /// The opaque id that names this action alone in a view: the choice or
    /// dialogue option id, the verb target id, or the untargeted verb id.
    /// `story.vote` and `story.tally` address actions by it.
    pub fn leaf_id(&self) -> &str {
        match self {
            Self::Choose { id, .. } | Self::Dialogue { id, .. } => id,
            Self::Menu {
                target: Some(target),
                ..
            } => target,
            Self::Menu { verb, .. } => verb,
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Choose { label, .. }
            | Self::Dialogue { label, .. }
            | Self::Menu { label, .. } => label,
        }
    }
}

impl StoryProjection {
    /// The free-text prompt of the view, when it accepts parser commands.
    pub fn prompt(&self) -> Option<&protocol::StoryPrompt> {
        self.items.iter().find_map(|item| match item {
            StoryItem::Scene { prompt, .. } => prompt.as_ref(),
            _ => None,
        })
    }

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
                title,
                text: prose,
                background,
                ..
            } => {
                let _ = write!(text, "\r\n== {title} ==\r\n");
                // The image itself is web-only; its alt text is the equivalent.
                if let Some(image) = background {
                    let _ = write!(text, "[{}]\r\n", image.alt);
                }
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
                kind: StoryBlockKind::Command,
                text: line,
                ..
            } => {
                let _ = write!(text, "> {line}\r\n");
            }
            StoryItem::Block {
                kind: StoryBlockKind::Reply,
                text: reply,
                ..
            } => {
                for line in reply.split('\n').filter(|line| !line.trim().is_empty()) {
                    let _ = write!(text, "{line}\r\n");
                }
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
            StoryItem::Portrait { name, .. } => present.push(name.as_str()),
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
        match projection.prompt() {
            Some(prompt) if prompt.question => {
                text.push_str(
                    "Responde a la pregunta o escribe otra orden; «deshacer» (undo) o «salir» (leave).\r\n",
                );
            }
            Some(_) => {
                text.push_str(
                    "Escribe una orden (por ejemplo «mira», «inventario» o «coge la llave»), «deshacer» (undo) o «salir» (leave).\r\n",
                );
            }
            None => text.push_str(
                "Escribe un número para elegir, «deshacer» (undo) o «salir» (leave).\r\n",
            ),
        }
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
    /// `historia <id> [idioma]`/`story <id> [locale]`.
    Enter(String, Option<String>),
    Leave,
    /// `historia compartir <id> [idioma]`/`story host <id> [locale]`: enter
    /// and share.
    Host(String, Option<String>),
    /// `historia unirse <código>`/`story join <code>` (seated) or
    /// `historia mirar <código>`/`story watch <code>` (spectator).
    Join {
        code: String,
        spectate: bool,
    },
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
    /// `votar N`/`vote N`; `0` withdraws the vote, `None` is malformed.
    Vote(Option<usize>),
    /// `asientos`/`seats`: the lobby of a shared instance.
    Seats,
    /// `compartir`/`share`: shares the current instance.
    Share,
    /// `invitar <nombre>`/`invite <name>`.
    Invite(String),
    /// `expulsar <nombre>`/`kick <name>`.
    Kick(String),
    /// `modo <anfitrion|votacion|turnos> [segundos]`; `None` is malformed.
    Mode(Option<(StoryDecisionMode, Option<u16>)>),
    Other,
}

/// Parses `historias`/`stories`, `historia <id> [idioma]`/`story <id>
/// [locale]` and `salir historia`/`story leave`. Everything else stays a
/// world command.
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
        ["historia", "compartir", id] | ["story", "host", id] => {
            Some(StoryCommand::Host((*id).to_owned(), None))
        }
        ["historia", "compartir", id, locale] | ["story", "host", id, locale] => Some(
            StoryCommand::Host((*id).to_owned(), Some((*locale).to_owned())),
        ),
        ["historia", "unirse", code] | ["story", "join", code] => Some(StoryCommand::Join {
            code: (*code).to_owned(),
            spectate: false,
        }),
        ["historia", "mirar", code] | ["story", "watch", code] => Some(StoryCommand::Join {
            code: (*code).to_owned(),
            spectate: true,
        }),
        ["historia", id] | ["story", id] => Some(StoryCommand::Enter((*id).to_owned(), None)),
        ["historia", id, locale] | ["story", id, locale] => Some(StoryCommand::Enter(
            (*id).to_owned(),
            Some((*locale).to_owned()),
        )),
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
        ["votar" | "vote", rest @ ..] if rest.len() <= 1 => {
            return StoryInputCommand::Vote(
                rest.first()
                    .filter(|word| word.len() <= 3)
                    .and_then(|word| word.parse::<usize>().ok()),
            );
        }
        ["asientos" | "seats" | "sala" | "lobby"] => return StoryInputCommand::Seats,
        ["compartir" | "share"] => return StoryInputCommand::Share,
        ["invitar" | "invite", _] | ["expulsar" | "kick", _] => {
            // Keep the typed case of the name.
            let name = trimmed
                .split_whitespace()
                .nth(1)
                .unwrap_or_default()
                .to_owned();
            return if words[0] == "invitar" || words[0] == "invite" {
                StoryInputCommand::Invite(name)
            } else {
                StoryInputCommand::Kick(name)
            };
        }
        ["modo" | "mode", rest @ ..] if (1..=2).contains(&rest.len()) => {
            let mode = match rest[0] {
                "anfitrion" | "anfitrión" | "host" => Some(StoryDecisionMode::Host),
                "votacion" | "votación" | "vote" | "votar" => Some(StoryDecisionMode::Vote),
                "turnos" | "turns" => Some(StoryDecisionMode::Turns),
                _ => None,
            };
            let seconds = match rest.get(1) {
                None => Some(None),
                Some(word) => word
                    .parse::<u16>()
                    .ok()
                    .filter(|_| word.len() <= 3)
                    .map(Some),
            };
            return StoryInputCommand::Mode(mode.zip(seconds));
        }
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

/// Parses one line typed inside a parser story (StoryPack v3). The reserved
/// words of [`parse_story_input`] keep their meaning (numbers, `deshacer`,
/// `salir`, `guardar N`, `votar N`, `compartir`…), which matches the
/// parser's own `deshacer`/`salir` meta commands; `look`/`l` and a bare
/// `guardar`/`cargar` (no slot) go to the parser instead, as its look verb
/// and its save/restore meta commands. Everything else (`Other`) is a parser
/// command line.
pub fn parse_parser_story_input(input: &str) -> StoryInputCommand {
    let lower = input.trim().to_lowercase();
    match lower.as_str() {
        "look" | "l" | "guardar" | "save" | "cargar" | "load" | "restore" => {
            StoryInputCommand::Other
        }
        _ => parse_story_input(input),
    }
}

/// Help of a parser story.
pub const PARSER_STORY_HELP: &str = "Historia con parser: escribe órdenes como «mira», «inventario», «coge la llave», «n» o «abre la puerta con la llave». También «deshacer» (undo), «guardar N» (save), «cargar N» (load), «partidas» (saves) o «salir» (leave).";

pub const STORY_INPUT_HELP: &str = "Estás absorto en una historia: escribe un número para elegir, «ver» (look), «deshacer» (undo), «guardar N» (save), «cargar N» (load), «partidas» (saves), «borrar N» (delete), «compartir» (share) o «salir» (leave).";

/// Help of a shared instance.
pub const SHARED_STORY_HELP: &str = "Historia compartida: un número o «votar N» (vote N) elige o vota, «votar 0» retira tu voto, «asientos» (seats) muestra la sala; el anfitrión puede «invitar <nombre>» (invite), «expulsar <nombre>» (kick), «modo anfitrion|votacion|turnos [segundos]» (mode host|vote|turns), «deshacer» (undo) y guardar o cargar partidas. «salir» (leave) deja tu asiento.";

pub const fn mode_label(mode: StoryDecisionMode) -> &'static str {
    match mode {
        StoryDecisionMode::Host => "decide el anfitrión",
        StoryDecisionMode::Vote => "votación por mayoría",
        StoryDecisionMode::Turns => "turnos rotatorios",
    }
}

/// Text rendering of `story.lobby` for Telnet and `command.submit`.
pub fn render_lobby(lobby: &protocol::StoryLobby) -> String {
    let mut text = format!(
        "Historia compartida · {} · plazo {} s",
        mode_label(lobby.mode),
        lobby.vote_seconds
    );
    if let Some(code) = &lobby.code {
        let _ = write!(
            text,
            "\nCódigo para unirse: {code} (historia unirse {code} / story join {code})"
        );
    }
    let _ = write!(
        text,
        "\nAsientos ({}/{}):",
        lobby.seats.len(),
        lobby.max_seats
    );
    for (index, seat) in lobby.seats.iter().enumerate() {
        let _ = write!(text, "\n  {}. {}", index + 1, seat.name);
        if seat.role == StorySeatRole::Host {
            text.push_str(" (anfitrión)");
        }
        if !seat.connected {
            text.push_str(" (desconectado; asiento reservado)");
        }
        if seat.turn {
            text.push_str(" ← turno");
        }
    }
    let _ = write!(
        text,
        "\nEspectadores: {}/{}. Tú: {}.",
        lobby.spectators,
        lobby.max_spectators,
        match lobby.you {
            StorySeatRole::Host => "anfitrión",
            StorySeatRole::Player => "jugador",
            StorySeatRole::Spectator => "espectador",
        }
    );
    text
}

/// One-line text rendering of `story.tally` against the numbered actions of
/// `view`. `None` when there is nothing to report.
pub fn render_tally(tally: &protocol::StoryTally, view: &StoryProjection) -> Option<String> {
    if tally.view_rev != view.view_rev || view.ending.is_some() {
        return None;
    }
    let actions = view.actions();
    let number = |id: &str| {
        actions
            .iter()
            .position(|action| action.leaf_id() == id)
            .map(|index| index + 1)
    };
    let mut text = match tally.mode {
        StoryDecisionMode::Vote => format!("Votos ({}/{}):", tally.voted, tally.eligible),
        StoryDecisionMode::Host => {
            if tally.choices.is_empty() {
                return None;
            }
            "Votos orientativos (decide el anfitrión):".to_owned()
        }
        StoryDecisionMode::Turns => return None,
    };
    if tally.choices.is_empty() {
        text.push_str(" ninguno todavía");
    }
    for entry in &tally.choices {
        if let Some(number) = number(&entry.choice_id) {
            let _ = write!(text, " {number}) {}", entry.votes);
        }
    }
    if let Some(seconds) = tally.remaining_seconds {
        let _ = write!(text, " · cierra en {seconds} s");
    }
    if let Some(number) = tally.your_vote.as_deref().and_then(number) {
        let _ = write!(text, " · tu voto: {number}");
    }
    Some(text)
}

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

/// What a transport must do after a parser command besides showing its view.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandFollowUp {
    /// `guardar`/`cargar` without a slot: show the slot listing.
    Slots,
    /// The parser's quit command (`fin`, `salir`, `quit`): leave the story
    /// exactly like the reserved `salir`/`leave`, autosave included.
    Leave,
}

/// Answer to one parser command line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandReply {
    pub view: StoryProjection,
    pub follow_up: Option<CommandFollowUp>,
}

enum Command {
    View(oneshot::Sender<StoryProjection>),
    /// One line of free text for the parser (StoryPack v3).
    Text {
        from: AccountId,
        text: String,
        view_rev: u32,
        reply: oneshot::Sender<Result<CommandReply, StoryFailure>>,
    },
    Input {
        from: AccountId,
        request: StoryRequest,
        view_rev: u32,
        reply: oneshot::Sender<Result<StoryProjection, StoryFailure>>,
    },
    /// `story.vote`: a vote for (or, with `None`, a withdrawal from) the
    /// action whose leaf id is given. Shared instances only.
    Vote {
        from: AccountId,
        leaf: Option<String>,
        view_rev: u32,
        reply: oneshot::Sender<Result<StoryProjection, StoryFailure>>,
    },
    Undo {
        from: AccountId,
        reply: oneshot::Sender<Result<StoryProjection, StoryFailure>>,
    },
    /// The current state as the exact bytes a save slot stores.
    Snapshot {
        from: AccountId,
        reply: oneshot::Sender<Result<SavedState, StoryFailure>>,
    },
    /// Replaces the state with one already validated against the pack.
    Restore {
        from: AccountId,
        state: Box<StoryState>,
        reply: oneshot::Sender<Result<StoryProjection, StoryFailure>>,
    },
    /// Leaves the instance. A solo instance (or a shared one whose last seat
    /// leaves, or any instance closed by shutdown) closes and hands back its
    /// state when it has progress (turn > 0), for the autosave.
    Leave {
        from: AccountId,
        reason: CloseReason,
        reply: oneshot::Sender<Option<SavedState>>,
    },
    /// Turns a solo instance into a shared one hosted by its owner.
    Share {
        from: AccountId,
        name: String,
        reply: oneshot::Sender<Result<(), StoryFailure>>,
    },
    Join {
        from: AccountId,
        name: String,
        spectate: bool,
        reply: oneshot::Sender<Result<(), StoryFailure>>,
    },
    Kick {
        from: AccountId,
        name: String,
        reply: oneshot::Sender<Result<(), StoryFailure>>,
    },
    Mode {
        from: AccountId,
        mode: StoryDecisionMode,
        vote_seconds: Option<u16>,
        reply: oneshot::Sender<Result<(), StoryFailure>>,
    },
    /// Host check before an invitation: the host's name and the join code.
    InviteInfo {
        from: AccountId,
        reply: oneshot::Sender<Result<(String, String), StoryFailure>>,
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

/// The latest public state of a shared instance, followed by every
/// participant's session through a `watch` channel (newest value wins, so a
/// slow session coalesces instead of queueing). Per-recipient fields (own
/// role, own vote, the host-only join code) are derived by the session.
#[derive(Clone, Debug)]
pub struct SharedFrame {
    pub view: StoryProjection,
    pub table: Table,
    /// Set on the last frame of a closed instance.
    pub closed: Option<protocol::StoryLeftReason>,
}

impl SharedFrame {
    pub fn role(&self, me: AccountId) -> Option<StorySeatRole> {
        self.table.role(me)
    }

    /// `story.lobby` for `me`.
    pub fn lobby(&self, me: AccountId) -> protocol::StoryLobby {
        self.table.lobby(me)
    }

    /// `story.tally` for `me`, with the remaining window measured now.
    pub fn tally(&self, me: AccountId) -> protocol::StoryTally {
        self.tally_at(me, tokio::time::Instant::now())
    }

    pub fn tally_at(&self, me: AccountId, now: tokio::time::Instant) -> protocol::StoryTally {
        let leaves = self
            .view
            .actions()
            .iter()
            .map(|action| action.leaf_id().to_owned())
            .collect::<Vec<_>>();
        self.table.tally(me, self.view.view_rev, &leaves, now)
    }

    /// Why `me` is no longer part of this instance, if it is not.
    pub fn removal(&self, me: AccountId) -> Option<protocol::StoryLeftReason> {
        if let Some(reason) = self.closed {
            return Some(reason);
        }
        match self.table.role(me) {
            Some(_) => None,
            None if self.table.is_banned(me) => Some(protocol::StoryLeftReason::Kicked),
            None => Some(protocol::StoryLeftReason::Evicted),
        }
    }

    /// Only the host may undo; other participants see `canUndo: false`.
    pub fn view_for(&self, me: AccountId, mut view: StoryProjection) -> StoryProjection {
        view.can_undo &= self.table.is_host(me);
        view
    }
}

/// An account's membership in a live instance.
struct LiveEntry {
    instance: u64,
    /// Strong references live in the account's session links; a dead weak
    /// reference means no session of the account is connected.
    presence: Weak<()>,
}

struct Route {
    sender: mpsc::Sender<Command>,
    story: Arc<CatalogStory>,
    /// Language the instance plays in, fixed when it starts.
    locale: String,
    /// The account that started the instance.
    owner: AccountId,
    /// Present once the instance is shared.
    frames: Option<watch::Receiver<Arc<SharedFrame>>>,
    code: Option<String>,
}

/// Routing of accounts and join codes to live instances. The instance tasks
/// own the authoritative state; this only maps who may talk to which task.
#[derive(Default)]
struct Registry {
    accounts: HashMap<AccountId, LiveEntry>,
    routes: HashMap<u64, Route>,
    codes: HashMap<String, u64>,
}

type InboxSender = mpsc::Sender<protocol::StoryInvitation>;

struct ManagerInner {
    catalog: StoryCatalog,
    limits: StoryLimits,
    live: Mutex<Registry>,
    next_instance: AtomicU64,
    metrics: StoryMetrics,
    /// Character store holding the save slots; unset when none is attached.
    store: OnceLock<Arc<dyn CharacterStore>>,
    slot_writes: KeyedRateLimiter<AccountId>,
    slot_reads: KeyedRateLimiter<AccountId>,
    joins: KeyedRateLimiter<AccountId>,
    invites: KeyedRateLimiter<AccountId>,
    /// Parser command lines per account (StoryPack v3).
    commands: KeyedRateLimiter<AccountId>,
    /// Lowercase character name → invitation inboxes of its sessions.
    inboxes: Mutex<HashMap<String, Vec<(u64, InboxSender)>>>,
    next_inbox: AtomicU64,
    /// Set by graceful shutdown: no new instance starts afterwards.
    closing: AtomicBool,
}

impl ManagerInner {
    fn registry(&self) -> std::sync::MutexGuard<'_, Registry> {
        self.live.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// Unregisters an instance, its join code and every member account.
    fn close_instance(&self, instance: u64, reason: CloseReason) {
        let mut registry = self.registry();
        let Some(route) = registry.routes.remove(&instance) else {
            return;
        };
        if let Some(code) = &route.code {
            registry.codes.remove(code);
        }
        registry
            .accounts
            .retain(|_, entry| entry.instance != instance);
        self.metrics
            .live
            .store(registry.routes.len(), Ordering::Relaxed);
        if route.frames.is_some() {
            self.metrics
                .shared_instances
                .fetch_sub(1, Ordering::Relaxed);
        }
        match reason {
            CloseReason::Left => &self.metrics.closed_left,
            CloseReason::Evicted => &self.metrics.closed_evicted,
            CloseReason::Shutdown => &self.metrics.closed_shutdown,
        }
        .fetch_add(1, Ordering::Relaxed);
    }

    /// Removes one participant's routing entry (leave, kick, lapse).
    fn remove_member(&self, account: AccountId, instance: u64) {
        let mut registry = self.registry();
        if registry
            .accounts
            .get(&account)
            .is_some_and(|entry| entry.instance == instance)
        {
            registry.accounts.remove(&account);
        }
    }

    /// Accounts of `members` with at least one connected session.
    fn present(&self, instance: u64, members: &[AccountId]) -> BTreeSet<AccountId> {
        let registry = self.registry();
        members
            .iter()
            .copied()
            .filter(|account| {
                registry.accounts.get(account).is_some_and(|entry| {
                    entry.instance == instance && entry.presence.strong_count() > 0
                })
            })
            .collect()
    }

    /// Publishes a freshly shared instance; `false` on a code collision.
    fn register_shared(
        &self,
        instance: u64,
        code: &str,
        frames: watch::Receiver<Arc<SharedFrame>>,
    ) -> bool {
        let mut registry = self.registry();
        if registry.codes.contains_key(code) || !registry.routes.contains_key(&instance) {
            return false;
        }
        registry.codes.insert(code.to_owned(), instance);
        if let Some(route) = registry.routes.get_mut(&instance) {
            route.frames = Some(frames);
            route.code = Some(code.to_owned());
        }
        self.metrics
            .shared_instances
            .fetch_add(1, Ordering::Relaxed);
        self.metrics.shared_started.fetch_add(1, Ordering::Relaxed);
        true
    }

    /// Delivers an invitation to every connected session of `name`. Unknown
    /// or offline names and full inboxes are dropped silently: the host
    /// never learns whether a character is connected.
    fn deliver(&self, name: &str, invitation: &protocol::StoryInvitation) {
        let inboxes = self
            .inboxes
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(senders) = inboxes.get(&name.trim().to_lowercase()) {
            for (_, sender) in senders {
                let _ = sender.try_send(invitation.clone());
            }
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
                    && (summary.pack_sha256 == story.pack_sha256
                        || story.compatible.contains(&summary.pack_sha256)),
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

/// A session's handle to its account's live instance. While any link of an
/// account is alive the account counts as connected to a shared instance.
#[derive(Clone)]
pub struct StoryLink {
    account: AccountId,
    story: Arc<CatalogStory>,
    /// Language of the instance and the title in it.
    locale: String,
    title: String,
    /// True when the link joined an instance that already existed (resume,
    /// re-entry or joining a shared instance); only a fresh instance offers
    /// to continue the autosave.
    resumed: bool,
    sender: mpsc::Sender<Command>,
    inner: Arc<ManagerInner>,
    frames: Option<watch::Receiver<Arc<SharedFrame>>>,
    _presence: Arc<()>,
}

/// Result of leaving a story.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LeaveOutcome {
    /// The state was written to the autosave slot.
    pub autosaved: bool,
}

/// A session's invitation inbox; unregistered when dropped.
pub struct InvitationInbox {
    receiver: mpsc::Receiver<protocol::StoryInvitation>,
    key: String,
    token: u64,
    inner: Arc<ManagerInner>,
}

impl InvitationInbox {
    pub async fn recv(&mut self) -> Option<protocol::StoryInvitation> {
        self.receiver.recv().await
    }
}

impl Drop for InvitationInbox {
    fn drop(&mut self) {
        let mut inboxes = self
            .inner
            .inboxes
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(senders) = inboxes.get_mut(&self.key) {
            senders.retain(|(token, _)| *token != self.token);
            if senders.is_empty() {
                inboxes.remove(&self.key);
            }
        }
    }
}

impl StoryManager {
    pub fn new(catalog: StoryCatalog, limits: StoryLimits) -> Self {
        Self {
            inner: Arc::new(ManagerInner {
                catalog,
                limits,
                live: Mutex::new(Registry::default()),
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
                joins: KeyedRateLimiter::per_minute_tracking(
                    JOINS_PER_MINUTE,
                    JOIN_BURST,
                    MAX_TRACKED_SLOT_ACCOUNTS,
                ),
                commands: KeyedRateLimiter::per_minute_tracking(
                    COMMANDS_PER_MINUTE,
                    COMMAND_BURST,
                    MAX_TRACKED_SLOT_ACCOUNTS,
                ),
                invites: KeyedRateLimiter::per_minute_tracking(
                    INVITES_PER_MINUTE,
                    INVITE_BURST,
                    MAX_TRACKED_SLOT_ACCOUNTS,
                ),
                inboxes: Mutex::new(HashMap::new()),
                next_inbox: AtomicU64::new(1),
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
        self.inner.registry().routes.len()
    }

    pub fn prometheus(&self) -> String {
        self.inner.metrics.prometheus(self.inner.catalog.len())
    }

    /// Registers the invitation inbox of a connected character's session.
    /// `None` when the name is empty or too many inboxes are registered.
    pub fn register_inbox(&self, name: &str) -> Option<InvitationInbox> {
        let key = name.trim().to_lowercase();
        if key.is_empty() || key.len() > shared::MAX_MEMBER_NAME_BYTES {
            return None;
        }
        let mut inboxes = self
            .inner
            .inboxes
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let registered = inboxes.values().map(Vec::len).sum::<usize>();
        if registered >= MAX_INBOXES {
            return None;
        }
        let (sender, receiver) = mpsc::channel(INVITATION_INBOX);
        let token = self.inner.next_inbox.fetch_add(1, Ordering::Relaxed);
        inboxes
            .entry(key.clone())
            .or_default()
            .push((token, sender));
        Some(InvitationInbox {
            receiver,
            key,
            token,
            inner: self.inner.clone(),
        })
    }

    /// Re-attaches a session to its account's live instance, if any.
    pub fn attach(&self, account: AccountId) -> Option<StoryLink> {
        let mut registry = self.inner.registry();
        let Registry {
            accounts, routes, ..
        } = &mut *registry;
        let entry = accounts.get_mut(&account)?;
        let route = routes.get(&entry.instance)?;
        Some(self.link(account, entry, route, true))
    }

    fn link(
        &self,
        account: AccountId,
        entry: &mut LiveEntry,
        route: &Route,
        resumed: bool,
    ) -> StoryLink {
        let presence = entry.presence.upgrade().unwrap_or_else(|| {
            let presence = Arc::new(());
            entry.presence = Arc::downgrade(&presence);
            presence
        });
        StoryLink {
            account,
            story: route.story.clone(),
            locale: route.locale.clone(),
            title: route.story.title_in(&route.locale),
            resumed,
            sender: route.sender.clone(),
            inner: self.inner.clone(),
            frames: route.frames.clone(),
            _presence: presence,
        }
    }

    /// Starts (or resumes, for the same story) the account's instance in the
    /// story's default language.
    #[cfg(test)]
    pub fn enter(&self, account: AccountId, story_id: &str) -> Result<StoryLink, StoryFailure> {
        self.enter_in(account, story_id, None)
    }

    /// Starts (or resumes) the account's instance. A new instance plays in
    /// `locale` when the pack offers it; a resumed one keeps its language.
    pub fn enter_in(
        &self,
        account: AccountId,
        story_id: &str,
        locale: Option<&str>,
    ) -> Result<StoryLink, StoryFailure> {
        // The seed comes from the server's secure RNG and is recorded in the
        // instance; story-core is deterministic from that point on.
        self.enter_seeded_in(account, story_id, locale, OsRng.next_u64(), None)
    }

    /// Enters with an explicit story seed and, for reproducible fixtures only,
    /// a deterministic id sequence instead of the secure RNG.
    #[cfg(test)]
    pub(crate) fn enter_seeded(
        &self,
        account: AccountId,
        story_id: &str,
        seed: u64,
        id_seed: Option<u64>,
    ) -> Result<StoryLink, StoryFailure> {
        self.enter_seeded_in(account, story_id, None, seed, id_seed)
    }

    pub(crate) fn enter_seeded_in(
        &self,
        account: AccountId,
        story_id: &str,
        locale: Option<&str>,
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
        let locale = story.resolve_locale(locale)?;
        let mut registry = self.inner.registry();
        let Registry {
            accounts, routes, ..
        } = &mut *registry;
        if let Some(entry) = accounts.get_mut(&account) {
            if let Some(route) = routes.get(&entry.instance)
                && route.story.id == story.id
                && !route.sender.is_closed()
            {
                return Ok(self.link(account, entry, route, true));
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
        if routes.len() >= self.inner.limits.max_instances {
            self.inner
                .metrics
                .rejected_capacity
                .fetch_add(1, Ordering::Relaxed);
            return Err(StoryFailure::Capacity);
        }
        let instance = Instance::new(
            story.clone(),
            locale.clone(),
            seed,
            id_seed,
            &self.inner.catalog.media,
        )
        .map_err(|_| StoryFailure::Internal)?;
        let instance_id = self.inner.next_instance.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = mpsc::channel(INSTANCE_MAILBOX);
        let route = Route {
            sender,
            story,
            locale,
            owner: account,
            frames: None,
            code: None,
        };
        let mut entry = LiveEntry {
            instance: instance_id,
            presence: Weak::new(),
        };
        let link = self.link(account, &mut entry, &route, false);
        accounts.insert(account, entry);
        routes.insert(instance_id, route);
        self.inner
            .metrics
            .live
            .store(routes.len(), Ordering::Relaxed);
        self.inner.metrics.started.fetch_add(1, Ordering::Relaxed);
        drop(registry);
        tokio::spawn(run_instance(
            self.inner.clone(),
            account,
            instance_id,
            instance,
            receiver,
        ));
        Ok(link)
    }

    /// `historia compartir <id> [idioma]`/`story host <id> [locale]`: enters
    /// (or resumes) the story and shares it, hosted by `account`. Joiners
    /// play in the host's language.
    pub async fn host(
        &self,
        account: AccountId,
        name: &str,
        story_id: &str,
        locale: Option<&str>,
    ) -> Result<StoryLink, StoryFailure> {
        self.enter_in(account, story_id, locale)?.share(name).await
    }

    /// `historia unirse <código>`/`story join <code>` (or, spectating,
    /// `historia mirar`/`story watch`). Attempts are rate limited per account;
    /// an unknown code and a refused (kicked) account get the same answer.
    pub async fn join(
        &self,
        account: AccountId,
        name: &str,
        code: &str,
        spectate: bool,
    ) -> Result<StoryLink, StoryFailure> {
        let result = self.join_inner(account, name, code, spectate).await;
        match &result {
            Ok(_) => &self.inner.metrics.shared_joins_ok,
            Err(_) => &self.inner.metrics.shared_joins_rejected,
        }
        .fetch_add(1, Ordering::Relaxed);
        result
    }

    async fn join_inner(
        &self,
        account: AccountId,
        name: &str,
        code: &str,
        spectate: bool,
    ) -> Result<StoryLink, StoryFailure> {
        if !self.inner.joins.check(account, Instant::now()) {
            return Err(StoryFailure::RateLimited);
        }
        let code = shared::normalize_join_code(code).ok_or(StoryFailure::UnknownCode)?;
        let (link, instance) = {
            let mut registry = self.inner.registry();
            let Registry {
                accounts,
                routes,
                codes,
            } = &mut *registry;
            let instance = *codes.get(&code).ok_or(StoryFailure::UnknownCode)?;
            let route = routes.get(&instance).ok_or(StoryFailure::UnknownCode)?;
            if let Some(entry) = accounts.get_mut(&account) {
                if entry.instance == instance {
                    return Ok(self.link(account, entry, route, true));
                }
                return Err(StoryFailure::AlreadyInStory);
            }
            if self.inner.closing.load(Ordering::Acquire) {
                return Err(StoryFailure::Busy);
            }
            let mut entry = LiveEntry {
                instance,
                presence: Weak::new(),
            };
            let link = self.link(account, &mut entry, route, true);
            accounts.insert(account, entry);
            (link, instance)
        };
        let name = name.to_owned();
        let joined = link
            .request(|reply| Command::Join {
                from: account,
                name,
                spectate,
                reply,
            })
            .await
            .and_then(|result| result);
        match joined {
            Ok(()) => Ok(link),
            Err(failure) => {
                self.inner.remove_member(account, instance);
                Err(match failure {
                    StoryFailure::Gone | StoryFailure::NotShared => StoryFailure::UnknownCode,
                    failure => failure,
                })
            }
        }
    }

    /// Graceful shutdown: refuses new instances, closes every live instance
    /// and writes its autosave (to the host of a shared instance). Returns
    /// how many autosaves were written. The caller bounds the whole call;
    /// each store call is bounded too.
    pub async fn shutdown(&self) -> usize {
        self.inner.closing.store(true, Ordering::Release);
        let entries = {
            let registry = self.inner.registry();
            registry
                .routes
                .values()
                .map(|route| {
                    let account = route
                        .frames
                        .as_ref()
                        .map_or(route.owner, |frames| frames.borrow().table.host());
                    (account, route.sender.clone(), route.story.clone())
                })
                .collect::<Vec<_>>()
        };
        let mut tasks = tokio::task::JoinSet::new();
        for (account, sender, story) in entries {
            let inner = self.inner.clone();
            tasks.spawn(async move {
                let (reply, response) = oneshot::channel();
                let command = Command::Leave {
                    from: account,
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

    /// The story title in the instance language.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The language the instance plays in.
    pub fn locale(&self) -> &str {
        &self.locale
    }

    /// Every language the story can be played in, default first.
    pub fn locales(&self) -> &[String] {
        self.story.locales()
    }

    /// The shared frames this link follows; `None` for a solo instance.
    pub fn frames(&self) -> Option<watch::Receiver<Arc<SharedFrame>>> {
        self.frames.clone()
    }

    pub const fn is_shared(&self) -> bool {
        self.frames.is_some()
    }

    /// Role in a shared instance (`None` for a solo instance).
    pub fn role(&self) -> Option<StorySeatRole> {
        self.frames
            .as_ref()
            .and_then(|frames| frames.borrow().role(self.account))
    }

    /// Save slots of a shared instance belong to its host.
    fn require_slot_owner(&self) -> Result<(), StoryFailure> {
        if self.is_shared() && self.role() != Some(StorySeatRole::Host) {
            Err(StoryFailure::HostOnly)
        } else {
            Ok(())
        }
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

    /// Applies one input addressed by opaque ids of view `view_rev`. In a
    /// shared instance the input is routed by the decision mode: applied,
    /// counted as a vote, or refused.
    pub async fn input(
        &self,
        request: StoryRequest,
        view_rev: u32,
    ) -> Result<StoryProjection, StoryFailure> {
        let from = self.account;
        let result = self
            .request(|reply| Command::Input {
                from,
                request,
                view_rev,
                reply,
            })
            .await
            .and_then(|result| result);
        self.record(&result);
        result
    }

    /// `story.vote`: votes for the action named by its leaf id, or (`None`)
    /// withdraws the vote.
    pub async fn vote(
        &self,
        leaf: Option<String>,
        view_rev: u32,
    ) -> Result<StoryProjection, StoryFailure> {
        let from = self.account;
        let result = self
            .request(|reply| Command::Vote {
                from,
                leaf,
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

    /// True when the story accepts free-text parser commands (StoryPack v3).
    pub fn is_parser(&self) -> bool {
        self.story.is_parser()
    }

    /// Applies one line of free text typed against view `view_rev` of a
    /// parser story. Lines are bounded to 256 bytes and rate limited per
    /// account. In a shared instance only the host (host and vote modes:
    /// free text is never put to a vote) or the turn holder (turns mode)
    /// may type. The parser's meta commands map to the session's own
    /// operations: `deshacer` undoes, `reiniciar` restarts after a typed
    /// confirmation, `puntos`/`ayuda` answer in the view, and `guardar`,
    /// `cargar` and `fin` come back as a [`CommandFollowUp`].
    pub async fn command(&self, text: &str, view_rev: u32) -> Result<CommandReply, StoryFailure> {
        let result = self.command_inner(text, view_rev).await;
        let metrics = &self.inner.metrics;
        if result.is_ok() {
            &metrics.commands_accepted
        } else {
            &metrics.commands_rejected
        }
        .fetch_add(1, Ordering::Relaxed);
        result
    }

    async fn command_inner(&self, text: &str, view_rev: u32) -> Result<CommandReply, StoryFailure> {
        if !self.story.is_parser() {
            return Err(StoryFailure::NoParser);
        }
        if text.len() > MAX_COMMAND_BYTES {
            return Err(StoryFailure::InvalidCommand);
        }
        if !self.inner.commands.check(self.account, Instant::now()) {
            self.inner
                .metrics
                .commands_rate_limited
                .fetch_add(1, Ordering::Relaxed);
            return Err(StoryFailure::CommandsRateLimited);
        }
        let from = self.account;
        let text = text.to_owned();
        self.request(|reply| Command::Text {
            from,
            text,
            view_rev,
            reply,
        })
        .await
        .and_then(|result| result)
    }

    pub async fn undo(&self) -> Result<StoryProjection, StoryFailure> {
        let from = self.account;
        let result = self
            .request(|reply| Command::Undo { from, reply })
            .await
            .and_then(|result| result);
        self.record(&result);
        result
    }

    /// Leaves the story. When the instance closes with progress and a store
    /// is attached, its state is written to the autosave slot first.
    pub async fn leave(&self) -> Result<LeaveOutcome, StoryFailure> {
        let from = self.account;
        let saved = self
            .request(|reply| Command::Leave {
                from,
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

    /// Shares this solo instance; returns the link that follows its frames.
    /// Sharing an already shared instance is a no-op.
    pub async fn share(&self, name: &str) -> Result<StoryLink, StoryFailure> {
        let from = self.account;
        let name = name.to_owned();
        self.request(|reply| Command::Share { from, name, reply })
            .await
            .and_then(|result| result)?;
        StoryManager {
            inner: self.inner.clone(),
        }
        .attach(self.account)
        .ok_or(StoryFailure::Gone)
    }

    /// Host only: removes a participant by name.
    pub async fn kick(&self, name: &str) -> Result<(), StoryFailure> {
        let from = self.account;
        let name = name.to_owned();
        self.request(|reply| Command::Kick { from, name, reply })
            .await
            .and_then(|result| result)
    }

    /// Host only: changes the decision mode and, optionally, the window.
    pub async fn set_mode(
        &self,
        mode: StoryDecisionMode,
        vote_seconds: Option<u16>,
    ) -> Result<(), StoryFailure> {
        let from = self.account;
        self.request(|reply| Command::Mode {
            from,
            mode,
            vote_seconds,
            reply,
        })
        .await
        .and_then(|result| result)
    }

    /// Host only: sends the join code to a connected character. The answer
    /// never reveals whether the character exists or is connected.
    pub async fn invite(&self, name: &str) -> Result<(), StoryFailure> {
        let name = name.trim();
        if name.is_empty() || name.len() > shared::MAX_MEMBER_NAME_BYTES {
            return Err(StoryFailure::UnknownMember);
        }
        if !self.is_shared() {
            return Err(StoryFailure::NotShared);
        }
        if !self.inner.invites.check(self.account, Instant::now()) {
            return Err(StoryFailure::RateLimited);
        }
        let from = self.account;
        let (host, code) = self
            .request(|reply| Command::InviteInfo { from, reply })
            .await
            .and_then(|result| result)?;
        if !host.eq_ignore_ascii_case(name) {
            self.inner.deliver(
                name,
                &protocol::StoryInvitation {
                    host,
                    story_id: self.story.id.clone(),
                    title: self.title.clone(),
                    code,
                },
            );
            self.inner
                .metrics
                .shared_invitations
                .fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }

    /// `story.save`: writes the current state to manual slot `1..=9`.
    pub async fn save(&self, slot: u8) -> Result<protocol::StorySaved, StoryFailure> {
        let slot = StorySlot::new(slot)
            .filter(|slot| !slot.is_autosave())
            .ok_or(StoryFailure::InvalidSlot)?;
        self.require_slot_owner()?;
        self.inner.store()?;
        self.inner.admit(self.account, true)?;
        let from = self.account;
        let saved = self
            .request(|reply| Command::Snapshot { from, reply })
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
    /// the exact current pack revision, or of a revision the pack declares
    /// compatible (`compatible_revisions`), re-validates it against the
    /// current pack and replaces the instance state. A slot of a compatible
    /// revision that fails re-validation is `slot_incompatible`; one of the
    /// exact revision is `slot_corrupt`. The undo history starts empty.
    pub async fn restore(&self, slot: u8) -> Result<StoryProjection, StoryFailure> {
        let result = self.restore_inner(slot).await;
        self.inner.metrics.restore(result.is_ok());
        result
    }

    async fn restore_inner(&self, slot: u8) -> Result<StoryProjection, StoryFailure> {
        let slot = StorySlot::new(slot).ok_or(StoryFailure::InvalidSlot)?;
        self.require_slot_owner()?;
        let store = self.inner.store()?;
        self.inner.admit(self.account, false)?;
        let compatibility = StoryPackCompatibility {
            pack_id: self.story.pack.id.clone(),
            pack_sha256: self.story.pack_sha256.clone(),
            compatible_pack_sha256: self.story.compatible.clone(),
        };
        let record = self
            .inner
            .call(store.load_story_slot(self.account, &self.story.id, slot, compatibility))
            .await?
            .ok_or(StoryFailure::SlotEmpty)?;
        let exact = record.summary.pack_sha256 == self.story.pack_sha256;
        let state = StoryState::from_json_slice(record.state_json.as_bytes(), &self.story.pack)
            .map_err(|error| {
                // The reason may name authored ids; it stays in the server log.
                warn!(%error, exact, "stored story state failed validation against its pack");
                if exact {
                    StoryFailure::SlotCorrupt
                } else {
                    StoryFailure::SlotIncompatible
                }
            })?;
        let from = self.account;
        self.request(|reply| Command::Restore {
            from,
            state: Box::new(state),
            reply,
        })
        .await
        .and_then(|result| result)
    }

    /// `story.slots`: metadata of the occupied slots of this story.
    pub async fn slots(&self) -> Result<Vec<protocol::StorySlotSummary>, StoryFailure> {
        self.require_slot_owner()?;
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
        self.require_slot_owner()?;
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

/// Shared-mode state owned by an instance task.
struct SharedCore {
    table: Table,
    frames: watch::Sender<Arc<SharedFrame>>,
}

impl SharedCore {
    fn publish(&self, instance: &Instance) {
        self.frames.send_replace(Arc::new(SharedFrame {
            view: instance.current.clone(),
            table: self.table.clone(),
            closed: None,
        }));
    }

    fn close(&self, instance: &Instance, reason: protocol::StoryLeftReason) {
        self.frames.send_replace(Arc::new(SharedFrame {
            view: instance.current.clone(),
            table: self.table.clone(),
            closed: Some(reason),
        }));
    }

    /// Applies action `ordinal` of the current view (a closed vote, the
    /// host's or the turn holder's decision) and publishes the result.
    fn decide(
        &mut self,
        inner: &ManagerInner,
        instance: &mut Instance,
        ordinal: usize,
    ) -> Result<StoryProjection, StoryFailure> {
        let request = instance
            .current
            .actions()
            .get(ordinal)
            .map(StoryAction::request)
            .ok_or(StoryFailure::UnknownChoice)?;
        let view_rev = instance.view_rev;
        let result = instance.input(&request, view_rev, &inner.catalog.media);
        if result.is_ok() {
            self.table
                .on_new_view(instance.view_rev, true, tokio::time::Instant::now());
            inner
                .metrics
                .shared_decisions
                .fetch_add(1, Ordering::Relaxed);
        }
        self.publish(instance);
        result
    }

    fn input(
        &mut self,
        inner: &ManagerInner,
        instance: &mut Instance,
        from: AccountId,
        request: &StoryRequest,
        view_rev: u32,
    ) -> Result<StoryProjection, StoryFailure> {
        match self.table.route(from)? {
            shared::Route::Apply => {
                let ordinal = instance.ordinal_of(request, view_rev)?;
                self.decide(inner, instance, ordinal)
            }
            shared::Route::Vote => {
                let ordinal = instance.ordinal_of(request, view_rev)?;
                self.vote(inner, instance, from, Some(ordinal))
            }
        }
    }

    /// A parser command line. Free text is never votable: in host and vote
    /// modes only the host types, in turns mode only the turn holder. A
    /// line that executed world commands takes the turn; parse errors and
    /// questions keep it.
    fn command(
        &mut self,
        inner: &ManagerInner,
        instance: &mut Instance,
        from: AccountId,
        text: &str,
        view_rev: u32,
    ) -> Result<CommandReply, StoryFailure> {
        self.table.route_command(from)?;
        let host = self.table.is_host(from);
        let before = (instance.view_rev, instance.state.turn);
        let result = instance.command(text, view_rev, &inner.catalog.media, host);
        if instance.view_rev != before.0 {
            let taken = instance.state.turn > before.1;
            self.table
                .on_new_view(instance.view_rev, taken, tokio::time::Instant::now());
            if taken {
                inner
                    .metrics
                    .shared_decisions
                    .fetch_add(1, Ordering::Relaxed);
            }
            self.publish(instance);
        }
        result
    }

    fn vote_leaf(
        &mut self,
        inner: &ManagerInner,
        instance: &mut Instance,
        from: AccountId,
        leaf: Option<&str>,
        view_rev: u32,
    ) -> Result<StoryProjection, StoryFailure> {
        self.table.require_seat(from)?;
        if leaf.is_some_and(|leaf| !valid_choice_id(leaf)) {
            return Err(StoryFailure::UnknownChoice);
        }
        if view_rev != instance.view_rev {
            return Err(StoryFailure::StaleView);
        }
        if instance.state.ended.is_some() {
            return Err(StoryFailure::Ended);
        }
        let ordinal = leaf
            .map(|leaf| {
                instance
                    .current
                    .actions()
                    .iter()
                    .position(|action| action.leaf_id() == leaf)
                    .ok_or(StoryFailure::UnknownChoice)
            })
            .transpose()?;
        self.vote(inner, instance, from, ordinal)
    }

    fn vote(
        &mut self,
        inner: &ManagerInner,
        instance: &mut Instance,
        from: AccountId,
        ordinal: Option<usize>,
    ) -> Result<StoryProjection, StoryFailure> {
        let decided = self
            .table
            .vote(from, ordinal, tokio::time::Instant::now())?;
        inner.metrics.shared_votes.fetch_add(1, Ordering::Relaxed);
        match decided {
            Some(winner) => self.decide(inner, instance, winner),
            None => {
                self.publish(instance);
                Ok(instance.current.clone())
            }
        }
    }

    /// One clock tick: presence, seat lapses, vote windows and turns.
    /// Returns `false` when the last seat lapsed and the instance closes.
    fn tick(&mut self, inner: &ManagerInner, instance: &mut Instance, id: u64) -> bool {
        let now = tokio::time::Instant::now();
        let members = self
            .table
            .members()
            .map(|member| member.account)
            .collect::<Vec<_>>();
        let present = inner.present(id, &members);
        let tick = self.table.tick(now, |account| present.contains(&account));
        for account in &tick.removed {
            inner.remove_member(*account, id);
            inner
                .metrics
                .shared_seat_lapses
                .fetch_add(1, Ordering::Relaxed);
        }
        if tick.closed {
            return false;
        }
        match tick.winner {
            Some(winner) => {
                let _ = self.decide(inner, instance, winner);
            }
            None if tick.changed => self.publish(instance),
            None => {}
        }
        true
    }
}

/// Closes an instance: unregisters it, ends its shared frames and writes the
/// autosave (to the host of a shared instance) on this exiting task.
async fn close_instance_task(
    inner: &ManagerInner,
    id: u64,
    owner: AccountId,
    instance: &Instance,
    shared: Option<&SharedCore>,
    reason: CloseReason,
    public: protocol::StoryLeftReason,
) {
    inner.close_instance(id, reason);
    let account = shared.map_or(owner, |core| core.table.host());
    if let Some(core) = shared {
        core.close(instance, public);
    }
    if let Some(saved) = instance.progress() {
        let story = instance.story.clone();
        inner.autosave(account, &story, saved).await;
    }
}

async fn run_instance(
    inner: Arc<ManagerInner>,
    owner: AccountId,
    instance_id: u64,
    mut instance: Instance,
    mut receiver: mpsc::Receiver<Command>,
) {
    let idle = inner.limits.idle_timeout;
    let mut shared: Option<SharedCore> = None;
    let mut clock = tokio::time::interval(SHARED_TICK);
    clock.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_activity = tokio::time::Instant::now();
    loop {
        let command = if let Some(core) = shared.as_mut() {
            tokio::select! {
                command = receiver.recv() => command,
                _ = clock.tick() => {
                    let idle_for = last_activity.elapsed();
                    if idle_for >= idle {
                        info!(instance = instance_id, "idle shared story instance evicted");
                        close_instance_task(
                            &inner, instance_id, owner, &instance, Some(core),
                            CloseReason::Evicted, protocol::StoryLeftReason::Evicted,
                        ).await;
                        return;
                    }
                    if !core.tick(&inner, &mut instance, instance_id) {
                        info!(instance = instance_id, "shared story instance ended");
                        close_instance_task(
                            &inner, instance_id, owner, &instance, Some(core),
                            CloseReason::Left, protocol::StoryLeftReason::Ended,
                        ).await;
                        return;
                    }
                    continue;
                }
            }
        } else {
            tokio::time::timeout(idle, receiver.recv())
                .await
                .unwrap_or_default()
        };
        let Some(command) = command else {
            info!(instance = instance_id, "idle story instance evicted");
            // The instance is unregistered first; the autosave runs on this
            // exiting task and never delays a live instance.
            close_instance_task(
                &inner,
                instance_id,
                owner,
                &instance,
                shared.as_ref(),
                CloseReason::Evicted,
                protocol::StoryLeftReason::Evicted,
            )
            .await;
            return;
        };
        last_activity = tokio::time::Instant::now();
        let media = &inner.catalog.media;
        match command {
            Command::View(reply) => {
                let _ = reply.send(instance.resume_projection());
            }
            Command::Text {
                from,
                text,
                view_rev,
                reply,
            } => {
                let result = match shared.as_mut() {
                    None => instance.command(&text, view_rev, media, true),
                    Some(core) => core.command(&inner, &mut instance, from, &text, view_rev),
                };
                let _ = reply.send(result);
            }
            Command::Input {
                from,
                request,
                view_rev,
                reply,
            } => {
                let result = match shared.as_mut() {
                    None => instance.input(&request, view_rev, media),
                    Some(core) => core.input(&inner, &mut instance, from, &request, view_rev),
                };
                let _ = reply.send(result);
            }
            Command::Vote {
                from,
                leaf,
                view_rev,
                reply,
            } => {
                let result = match shared.as_mut() {
                    None => Err(StoryFailure::NotShared),
                    Some(core) => {
                        core.vote_leaf(&inner, &mut instance, from, leaf.as_deref(), view_rev)
                    }
                };
                let _ = reply.send(result);
            }
            Command::Undo { from, reply } => {
                let result = match shared.as_mut() {
                    None => instance.undo(media),
                    Some(core) => core.table.require_host(from).and_then(|()| {
                        let result = instance.undo(media);
                        if result.is_ok() {
                            core.table.on_new_view(
                                instance.view_rev,
                                false,
                                tokio::time::Instant::now(),
                            );
                            core.publish(&instance);
                        }
                        result
                    }),
                };
                let _ = reply.send(result);
            }
            Command::Snapshot { from, reply } => {
                let result = match shared.as_ref() {
                    Some(core) => core.table.require_host(from),
                    None => Ok(()),
                }
                .and_then(|()| SavedState::of(&instance.state));
                let _ = reply.send(result);
            }
            Command::Restore { from, state, reply } => {
                let result = match shared.as_mut() {
                    None => instance.restore(*state, media),
                    Some(core) => core.table.require_host(from).and_then(|()| {
                        let result = instance.restore(*state, media);
                        if result.is_ok() {
                            core.table.on_new_view(
                                instance.view_rev,
                                false,
                                tokio::time::Instant::now(),
                            );
                            core.publish(&instance);
                        }
                        result
                    }),
                };
                let _ = reply.send(result);
            }
            Command::Leave {
                from,
                reason,
                reply,
            } => {
                if let (Some(core), CloseReason::Left) = (shared.as_mut(), reason) {
                    let removal = core.table.leave(from, tokio::time::Instant::now());
                    inner.remove_member(from, instance_id);
                    if !removal.closed {
                        // Fewer seats may complete the vote of the view.
                        let _ = core.tick(&inner, &mut instance, instance_id);
                        core.publish(&instance);
                        let _ = reply.send(None);
                        continue;
                    }
                    inner.close_instance(instance_id, reason);
                    core.close(&instance, protocol::StoryLeftReason::Ended);
                    let _ = reply.send(instance.progress());
                    return;
                }
                // Unregister before acknowledging so the session can start
                // another story immediately.
                inner.close_instance(instance_id, reason);
                if let Some(core) = &shared {
                    core.close(&instance, protocol::StoryLeftReason::Evicted);
                }
                let _ = reply.send(instance.progress());
                return;
            }
            Command::Share { from, name, reply } => {
                let result = if let Some(core) = &shared {
                    core.table
                        .role(from)
                        .map(|_| ())
                        .ok_or(StoryFailure::NotInStory)
                } else if from != owner {
                    Err(StoryFailure::NotInStory)
                } else {
                    let core = share_instance(&inner, instance_id, from, &name, &instance);
                    let result = core.as_ref().map(|_| ()).ok_or(StoryFailure::Internal);
                    shared = core;
                    result
                };
                let _ = reply.send(result);
            }
            Command::Join {
                from,
                name,
                spectate,
                reply,
            } => {
                let result = match shared.as_mut() {
                    None => Err(StoryFailure::NotShared),
                    Some(core) => core.table.join(from, &name, spectate).map(|()| {
                        core.publish(&instance);
                    }),
                };
                let _ = reply.send(result);
            }
            Command::Kick { from, name, reply } => {
                let result = match shared.as_mut() {
                    None => Err(StoryFailure::NotShared),
                    Some(core) => core
                        .table
                        .kick(from, &name, tokio::time::Instant::now())
                        .map(|(target, _)| {
                            inner.remove_member(target, instance_id);
                            inner.metrics.shared_kicks.fetch_add(1, Ordering::Relaxed);
                            let _ = core.tick(&inner, &mut instance, instance_id);
                            core.publish(&instance);
                        }),
                };
                let _ = reply.send(result);
            }
            Command::Mode {
                from,
                mode,
                vote_seconds,
                reply,
            } => {
                let result = match shared.as_mut() {
                    None => Err(StoryFailure::NotShared),
                    Some(core) => core
                        .table
                        .set_mode(from, mode, vote_seconds, tokio::time::Instant::now())
                        .map(|()| core.publish(&instance)),
                };
                let _ = reply.send(result);
            }
            Command::InviteInfo { from, reply } => {
                let result = match shared.as_ref() {
                    None => Err(StoryFailure::NotShared),
                    Some(core) => core.table.require_host(from).map(|()| {
                        let host = core
                            .table
                            .seats()
                            .first()
                            .map(|seat| seat.name.clone())
                            .unwrap_or_default();
                        (host, core.table.code().to_owned())
                    }),
                };
                let _ = reply.send(result);
            }
        }
    }
}

/// Makes a solo instance shared: draws a unique join code, builds the table
/// and publishes the first frame. `None` if no unique code could be drawn.
fn share_instance(
    inner: &ManagerInner,
    id: u64,
    host: AccountId,
    name: &str,
    instance: &Instance,
) -> Option<SharedCore> {
    for _ in 0..8 {
        let code = shared::join_code(OsRng.next_u64());
        let table = Table::new(
            host,
            name,
            inner.limits.shared.clone(),
            code.clone(),
            instance.view_rev,
        );
        let (frames, receiver) = watch::channel(Arc::new(SharedFrame {
            view: instance.current.clone(),
            table: table.clone(),
            closed: None,
        }));
        if inner.register_shared(id, &code, receiver) {
            return Some(SharedCore { table, frames });
        }
    }
    warn!("could not draw a unique story join code");
    None
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
    /// Language of the views and of the parser lexicon.
    locale: String,
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
    /// Echo of the parser command the next projection answers.
    echo: Option<String>,
    /// Server notes (meta command hints) shown after the parser replies of
    /// the next projection.
    notes: Vec<&'static str>,
    /// View revision that showed the restart confirmation; a second
    /// `reiniciar` typed against exactly that view restarts the story.
    restart_armed: Option<u32>,
}

const RESTART_CONFIRM_NOTE: &str = "¿Seguro que quieres reiniciar la historia desde el principio? Escribe «reiniciar» otra vez para confirmarlo; cualquier otra orden lo cancela. Podrás «deshacer» el reinicio.";
const RESTARTED_NOTE: &str = "La historia vuelve a empezar.";
const SAVE_NOTE: &str =
    "Para guardar escribe «guardar N» (ranuras 1 a 9) o usa el panel de partidas guardadas.";
const RESTORE_NOTE: &str = "Para cargar escribe «cargar N» (la ranura 0 es el autoguardado) o usa el panel de partidas guardadas.";
const QUIT_NOTE: &str = "Sales de la historia.";

impl Instance {
    fn new(
        story: Arc<CatalogStory>,
        locale: String,
        seed: u64,
        id_state: Option<u64>,
        media: &StoryMedia,
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
            title: story.title_in(&locale),
            view_rev: 0,
            turn: 0,
            score: 0,
            ending: None,
            can_undo: false,
            items: Vec::new(),
        };
        let mut instance = Self {
            story,
            locale,
            seed,
            state,
            undo,
            view_rev: 0,
            ids: Vec::new(),
            music: None,
            current,
            id_state,
            echo: None,
            notes: Vec::new(),
            restart_armed: None,
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
        media: &StoryMedia,
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

    /// Applies one parser command line (StoryPack v3) typed against view
    /// `view_rev`. `manage` allows the meta commands that replace the game
    /// for every participant (undo, restart): always in a solo instance,
    /// only for the host of a shared one. Any failure leaves the state
    /// exactly as it was.
    fn command(
        &mut self,
        text: &str,
        view_rev: u32,
        media: &StoryMedia,
        manage: bool,
    ) -> Result<CommandReply, StoryFailure> {
        if text.len() > MAX_COMMAND_BYTES {
            return Err(StoryFailure::InvalidCommand);
        }
        if view_rev != self.view_rev {
            return Err(StoryFailure::StaleView);
        }
        if self.state.ended.is_some() {
            return Err(StoryFailure::Ended);
        }
        // Only the very next input may confirm a restart.
        let armed = self.restart_armed.take() == Some(self.view_rev);
        let stepped = step_command(
            &self.story.pack,
            &self.state,
            text,
            Some(self.locale.as_str()),
        )
        .map_err(|error| {
            use tiamud_story_core::StoryError as Core;
            match error {
                Core::Ended => StoryFailure::Ended,
                Core::NoParser => StoryFailure::NoParser,
                Core::InDialogue => StoryFailure::InvalidCommand,
                error => {
                    warn!(%error, "story command failed");
                    StoryFailure::Internal
                }
            }
        })?;
        let echo = echo_text(text);
        let advanced = stepped.state.turn != self.state.turn;
        let meta = stepped.meta.first().map(|meta| meta.action.clone());
        let reply = |view: StoryProjection, follow_up: Option<CommandFollowUp>| CommandReply {
            view,
            follow_up,
        };
        match meta.as_deref() {
            // A line such as «coge la llave. deshacer» first runs its
            // commands; undoing then pops exactly that line. A bare
            // «deshacer» undoes the last applied input, like `story.undo`.
            Some("undo") => {
                if !manage {
                    return Err(StoryFailure::HostOnly);
                }
                if advanced {
                    self.apply(stepped.state, &stepped.events, echo, Vec::new(), media)?;
                }
                self.undo(media).map(|view| reply(view, None))
            }
            Some("restart") => {
                if !manage {
                    return Err(StoryFailure::HostOnly);
                }
                if armed && !advanced {
                    return self.restart(echo, media).map(|view| reply(view, None));
                }
                let view = self.apply(
                    stepped.state,
                    &stepped.events,
                    echo,
                    vec![RESTART_CONFIRM_NOTE],
                    media,
                )?;
                self.restart_armed = Some(self.view_rev);
                Ok(reply(view, None))
            }
            Some("save") => self
                .apply(stepped.state, &stepped.events, echo, vec![SAVE_NOTE], media)
                .map(|view| reply(view, Some(CommandFollowUp::Slots))),
            Some("restore") => self
                .apply(
                    stepped.state,
                    &stepped.events,
                    echo,
                    vec![RESTORE_NOTE],
                    media,
                )
                .map(|view| reply(view, Some(CommandFollowUp::Slots))),
            Some("quit") => self
                .apply(stepped.state, &stepped.events, echo, vec![QUIT_NOTE], media)
                .map(|view| reply(view, Some(CommandFollowUp::Leave))),
            // `score` and `help` are already answered in the transcript.
            _ => self
                .apply(stepped.state, &stepped.events, echo, Vec::new(), media)
                .map(|view| reply(view, None)),
        }
    }

    /// Replaces the state with the result of a parser line and projects
    /// it. Only a line that executed world commands (the turn advanced) is
    /// an undo step; parse errors and questions are not.
    fn apply(
        &mut self,
        next: StoryState,
        events: &[StoryEvent],
        echo: Option<String>,
        notes: Vec<&'static str>,
        media: &StoryMedia,
    ) -> Result<StoryProjection, StoryFailure> {
        let advanced = next.turn != self.state.turn;
        let previous = std::mem::replace(&mut self.state, next);
        let music = self.music.clone();
        if advanced {
            self.undo.push(previous.clone());
        }
        self.echo = echo;
        self.notes = notes;
        if self.project(events, media).is_err() {
            self.state = previous;
            self.music = music;
            if advanced {
                self.undo.pop();
            }
            self.echo = None;
            self.notes.clear();
            return Err(StoryFailure::Internal);
        }
        Ok(self.current.clone())
    }

    /// Restarts the story from its start with a fresh seed. The previous
    /// state is an undo step, so `deshacer` brings the old game back.
    fn restart(
        &mut self,
        echo: Option<String>,
        media: &StoryMedia,
    ) -> Result<StoryProjection, StoryFailure> {
        let seed = next_id_word(&mut self.id_state);
        let fresh = StoryState::new(&self.story.pack, seed).map_err(|_| StoryFailure::Internal)?;
        let scene_cues = core_view(&self.story.pack, &fresh)
            .map_err(|_| StoryFailure::Internal)?
            .cues;
        let mut events = Vec::with_capacity(scene_cues.len() + 1);
        if self.music.is_some() {
            events.push(StoryEvent::Cue(Cue::StopMusic));
        }
        events.extend(scene_cues.into_iter().map(StoryEvent::Cue));
        let previous = std::mem::replace(&mut self.state, fresh);
        let music = self.music.clone();
        self.undo.push(previous.clone());
        self.echo = echo;
        self.notes = vec![RESTARTED_NOTE];
        if self.project(&events, media).is_err() {
            self.state = previous;
            self.music = music;
            self.undo.pop();
            self.echo = None;
            self.notes.clear();
            return Err(StoryFailure::Internal);
        }
        self.seed = seed;
        Ok(self.current.clone())
    }

    /// Ordinal (in [`StoryProjection::actions`] order) of the action that
    /// `request` addresses in view `view_rev`; validated like an input.
    fn ordinal_of(&self, request: &StoryRequest, view_rev: u32) -> Result<usize, StoryFailure> {
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
        let actions = self.current.actions();
        let addressed = |action: &StoryAction| match (request, action) {
            (StoryRequest::Choose { id }, StoryAction::Choose { id: own, .. })
            | (
                StoryRequest::Choose { id } | StoryRequest::Dialogue { id },
                StoryAction::Dialogue { id: own, .. },
            ) => id == own,
            (
                StoryRequest::Menu { verb, target },
                StoryAction::Menu {
                    verb: own_verb,
                    target: own_target,
                    ..
                },
            ) => verb == own_verb && target == own_target,
            _ => false,
        };
        actions
            .iter()
            .position(addressed)
            .ok_or(StoryFailure::UnknownChoice)
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
        media: &StoryMedia,
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

    fn undo(&mut self, media: &StoryMedia) -> Result<StoryProjection, StoryFailure> {
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
        media: &StoryMedia,
    ) -> Result<(), tiamud_story_core::StoryError> {
        let story = self.story.clone();
        let pack = &story.pack;
        let locale = self.locale.clone();
        // Any new view cancels a pending restart confirmation; the command
        // that shows the confirmation arms it again after projecting.
        self.restart_armed = None;
        let echo = self.echo.take();
        let notes = std::mem::take(&mut self.notes);
        let pages = view_pages(pack, &self.state, &locale)?;
        let event_items = events
            .iter()
            .filter_map(|event| render_event(pack, event, &locale))
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
        let mut title = story.title_in(&locale);
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
                        prompt,
                        question,
                        background,
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
                            prompt: prompt.map(|text| protocol::StoryPrompt { text, question }),
                            background: background.and_then(|id| media.image(&id, &locale)),
                        });
                        if let Some(echo) = &echo {
                            items.push(StoryItem::Block {
                                kind: StoryBlockKind::Command,
                                speaker: None,
                                text: echo.clone(),
                            });
                        }
                        if let Some((events, cues)) = extras.take() {
                            items.extend(cues);
                            items.extend(events);
                        }
                    }
                    // Scene cues are emitted as events on entry; resending
                    // them with every view would restart music.
                    PageItem::Cue(_) => {}
                    PageItem::Portrait { name, asset } => items.push(StoryItem::Portrait {
                        name,
                        image: asset.and_then(|id| media.image(&id, &locale)),
                    }),
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
        // The parser transcript is rendered as the last speakerless blocks
        // of the view (after menu and dialogue text): mark them as replies.
        let replies = self.state.transcript.len();
        let mut marked = 0;
        let mut last_block = None;
        for (index, item) in items.iter_mut().enumerate().rev() {
            let StoryItem::Block { kind, speaker, .. } = item else {
                continue;
            };
            last_block = last_block.or(Some(index));
            if *kind == StoryBlockKind::Line && speaker.is_none() && marked < replies {
                *kind = StoryBlockKind::Reply;
                marked += 1;
            }
        }
        // Server notes (meta command hints) follow the parser replies.
        let at = last_block.map_or_else(
            || {
                items
                    .iter()
                    .position(|item| {
                        !matches!(
                            item,
                            StoryItem::Scene { .. }
                                | StoryItem::Cue { .. }
                                | StoryItem::Portrait { .. }
                        )
                    })
                    .unwrap_or(items.len())
            },
            |index| index + 1,
        );
        items.splice(
            at..at,
            notes.into_iter().map(|note| StoryItem::Block {
                kind: StoryBlockKind::Reply,
                speaker: None,
                text: note.to_owned(),
            }),
        );
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

    fn resolve_cue(&mut self, cue: &Cue, media: &StoryMedia) -> Option<StoryItem> {
        match cue {
            Cue::Music { asset } => match media.audio.get(asset) {
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
            Cue::Sfx { asset } => media.audio.get(asset).map(|url| StoryItem::Cue {
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

fn resolve(text: &tiamud_story_core::LocalizedText, locale: &str) -> String {
    text.resolve(locale).unwrap_or_default().to_owned()
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

/// The echo of a typed command: control characters removed, whitespace
/// collapsed, at most [`MAX_COMMAND_BYTES`]. `None` for a blank line.
fn echo_text(text: &str) -> Option<String> {
    let cleaned = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut cut = collapsed.len().min(MAX_COMMAND_BYTES);
    while !collapsed.is_char_boundary(cut) {
        cut -= 1;
    }
    let echo = &collapsed[..cut];
    (!echo.is_empty()).then(|| echo.to_owned())
}

/// Renders the player-visible part of one event. Hidden variables, node
/// entries, menu bookkeeping and media cues produce no text (menu responses
/// and dialogue lines arrive as `line` blocks of the view itself).
fn render_event(pack: &StoryPack, event: &StoryEvent, locale: &str) -> Option<String> {
    match event {
        StoryEvent::VariableChanged { var, from, to, .. } => {
            let decl = pack
                .variables
                .iter()
                .find(|decl| &decl.id == var && decl.visible)?;
            let label = decl
                .label
                .as_ref()
                .map(|label| resolve(label, locale))
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
            .map(|decl| format!("Obtienes: {}", resolve(&decl.name, locale))),
        StoryEvent::ItemLost { item } => pack
            .items
            .iter()
            .find(|decl| &decl.id == item)
            .map(|decl| format!("Pierdes: {}", resolve(&decl.name, locale))),
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

#[cfg(test)]
mod shared_tests;

#[cfg(test)]
mod parser_tests;

#[cfg(test)]
mod media_tests;
