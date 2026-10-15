//! Offline authoring tools for StoryPacks (phase S1 tooling, ADR 0025).
//!
//! This crate is the library behind the `tiamud-story` binary. It only reads
//! local files and standard input; it never opens sockets, spawns processes or
//! touches persistence. Every input is bounded: pack and script bytes, script
//! steps, interactive line length and interactive command count.
//!
//! The narrative rules themselves live in `tiamud-story-core`; these tools
//! only drive [`tiamud_story_core::step`] and [`tiamud_story_core::view`] and
//! add static authoring lints on top of the core validator.

#![forbid(unsafe_code)]

pub mod format;
pub mod lint;
pub mod play;
pub mod render;
pub mod script;

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use tiamud_story_core::{
    MAX_PACK_BYTES, StoryError, StoryPack, ValidationErrors, ValidationReport,
};

/// Published JSON Schema (draft 2020-12) for StoryPack format v1.
///
/// The schema describes the canonical document shape. The validator in
/// `tiamud-story-core` remains authoritative for references, byte limits,
/// nesting budgets and lints.
pub const STORY_PACK_SCHEMA: &str = include_str!("../schema/story-pack.v1.schema.json");

/// Errors raised while loading authoring inputs.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("could not read {path:?}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path:?} is not a regular file")]
    NotAFile { path: PathBuf },
    #[error("{path:?} exceeds the {limit}-byte limit")]
    TooLarge { path: PathBuf, limit: usize },
    #[error("could not load story pack {path:?}: {source}")]
    Story { path: PathBuf, source: StoryError },
    #[error("story pack {path:?} is invalid:\n{errors}")]
    Invalid {
        path: PathBuf,
        errors: ValidationErrors,
    },
}

/// Reads a regular file of at most `limit` bytes without trusting metadata.
pub fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>, ToolError> {
    let read_error = |source| ToolError::Read {
        path: path.to_path_buf(),
        source,
    };
    let file = File::open(path).map_err(read_error)?;
    let metadata = file.metadata().map_err(read_error)?;
    if !metadata.is_file() {
        return Err(ToolError::NotAFile {
            path: path.to_path_buf(),
        });
    }
    let too_large = || ToolError::TooLarge {
        path: path.to_path_buf(),
        limit,
    };
    if metadata.len() > u64::try_from(limit).unwrap_or(u64::MAX) {
        return Err(too_large());
    }
    let mut bytes = Vec::new();
    file.take(u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(read_error)?;
    if bytes.len() > limit {
        return Err(too_large());
    }
    Ok(bytes)
}

/// Parses a pack without validating it.
pub fn parse_pack(path: &Path) -> Result<StoryPack, ToolError> {
    let bytes = read_bounded(path, MAX_PACK_BYTES)?;
    StoryPack::from_json_slice(&bytes).map_err(|source| ToolError::Story {
        path: path.to_path_buf(),
        source,
    })
}

/// Parses and validates a pack. Validation errors are fatal; the returned
/// report carries the core lint warnings.
pub fn load_pack(path: &Path) -> Result<(StoryPack, ValidationReport), ToolError> {
    let pack = parse_pack(path)?;
    let report = pack.validate().map_err(|errors| ToolError::Invalid {
        path: path.to_path_buf(),
        errors,
    })?;
    Ok((pack, report))
}
