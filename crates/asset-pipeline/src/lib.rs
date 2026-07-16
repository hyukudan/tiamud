//! Offline compiler for bounded, content-addressed scene and entity assets.

use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::imageops::FilterType;
use image::{
    DynamicImage, GenericImageView, ImageEncoder, ImageFormat, ImageReader, RgbImage, RgbaImage,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const TARGET_WIDTHS: [u32; 2] = [640, 1280];
pub const SPRITE_TARGET_WIDTHS: [u32; 2] = [320, 640];
const ATMOSPHERE_SCHEMA_VERSION: u8 = 2;
const ENTITY_SPRITE_SCHEMA_VERSION: u8 = 3;
const JPEG_QUALITY: u8 = 84;
const AVIF_QUALITY: f32 = 64.0;
const AVIF_SPEED: u8 = 8;
const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const DEFAULT_SPRITE_MAX_VARIANT_BYTES: usize = 512 * 1024;
#[cfg(feature = "webp-encoding")]
const WEBP_QUALITY: f32 = 82.0;

static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug)]
pub struct BuildRequest {
    pub source: PathBuf,
    pub alt: String,
    pub output_dir: PathBuf,
    pub limits: BuildLimits,
    pub metadata: AuthoringMetadata,
    /// Explicit publication profile. Sprite poses are metadata, never inferred
    /// from a source filename.
    pub profile: AssetProfile,
    /// Publish only deterministic PNG variants, even when the source is opaque.
    /// This option applies only to atmospheric assets.
    pub lossless: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AssetProfile {
    #[default]
    Atmosphere,
    EntitySprite {
        pose: SpritePose,
    },
}

impl AssetProfile {
    const fn kind(self) -> AssetKind {
        match self {
            Self::Atmosphere => AssetKind::Atmosphere,
            Self::EntitySprite { .. } => AssetKind::EntitySprite,
        }
    }

    const fn pose(self) -> Option<SpritePose> {
        match self {
            Self::Atmosphere => None,
            Self::EntitySprite { pose } => Some(pose),
        }
    }

    const fn target_widths(self) -> &'static [u32] {
        match self {
            Self::Atmosphere => &TARGET_WIDTHS,
            Self::EntitySprite { .. } => &SPRITE_TARGET_WIDTHS,
        }
    }

    const fn minimum_source_width(self) -> u32 {
        match self {
            Self::Atmosphere => 1280,
            Self::EntitySprite { .. } => 640,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AssetKind {
    Atmosphere,
    EntitySprite,
}

impl fmt::Display for AssetKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Atmosphere => "atmosphere",
            Self::EntitySprite => "entity-sprite",
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SpritePose {
    Idle,
    Attack,
    Hit,
    Defeated,
}

impl SpritePose {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Attack => "attack",
            Self::Hit => "hit",
            Self::Defeated => "defeated",
        }
    }
}

impl fmt::Display for SpritePose {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BuildLimits {
    pub max_source_bytes: u64,
    pub max_dimension: u32,
    pub max_pixels: u64,
    pub max_decoded_bytes: u64,
    pub max_variant_bytes: usize,
    pub max_alt_chars: usize,
}

impl Default for BuildLimits {
    fn default() -> Self {
        Self {
            max_source_bytes: 64 * 1024 * 1024,
            max_dimension: 8192,
            max_pixels: 40_000_000,
            max_decoded_bytes: 256 * 1024 * 1024,
            max_variant_bytes: 2 * 1024 * 1024,
            max_alt_chars: 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AssetFormat {
    Avif,
    Webp,
    Jpeg,
    Png,
}

impl AssetFormat {
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Avif => "avif",
            Self::Webp => "webp",
            Self::Jpeg => "jpg",
            Self::Png => "png",
        }
    }

    pub const fn media_type(self) -> &'static str {
        match self {
            Self::Avif => "image/avif",
            Self::Webp => "image/webp",
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
        }
    }
}

impl fmt::Display for AssetFormat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Avif => "AVIF",
            Self::Webp => "WebP",
            Self::Jpeg => "JPEG",
            Self::Png => "PNG",
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetManifest {
    pub schema: u8,
    pub asset_id: String,
    pub revision: u32,
    pub kind: AssetKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pose: Option<SpritePose>,
    /// Fallback only. WorldPack scenes provide the contextual/localized alt.
    pub alt: String,
    pub focal_point: FocalPoint,
    pub dominant_color: String,
    pub author: String,
    pub license: String,
    pub provenance: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generator: Option<String>,
    pub content_classification: String,
    pub human_reviewed_at: String,
    pub source: SourceDescriptor,
    pub variants: Vec<PublishedVariant>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthoringMetadata {
    pub revision: u32,
    pub focal_point: FocalPoint,
    pub author: String,
    pub license: String,
    pub provenance: String,
    pub generator: Option<String>,
    pub content_classification: String,
    pub human_reviewed_at: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FocalPoint {
    /// Horizontal position in thousandths, from 0 (left) to 1000 (right).
    pub x_per_mille: u16,
    /// Vertical position in thousandths, from 0 (top) to 1000 (bottom).
    pub y_per_mille: u16,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDescriptor {
    pub sha256: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishedVariant {
    pub format: AssetFormat,
    pub media_type: String,
    pub width: u32,
    pub height: u32,
    pub bytes: usize,
    pub sha256: String,
    pub path: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildResult {
    pub asset_id: String,
    pub manifest_sha256: String,
    pub manifest_path: PathBuf,
    pub manifest: AssetManifest,
}

#[derive(Clone, Debug)]
pub struct QuarantineRequest {
    pub source: PathBuf,
    pub quarantine_dir: PathBuf,
    pub limits: BuildLimits,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuarantineReview {
    pub reviewer: String,
    pub reviewed_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuarantineRecord {
    pub schema: u8,
    pub original_sha256: String,
    pub sanitized_sha256: String,
    pub width: u32,
    pub height: u32,
    pub has_transparency: bool,
    pub sanitized_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review: Option<QuarantineReview>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuarantineResult {
    pub record_path: PathBuf,
    pub record: QuarantineRecord,
}

#[derive(Debug, Error)]
pub enum PipelineError {
    #[error("alternative text must not be empty")]
    EmptyAlt,
    #[error("alternative text exceeds the {maximum}-character limit")]
    AltTooLong { maximum: usize },
    #[error("alternative text must be a single line without control characters")]
    InvalidAltCharacters,
    #[error("asset metadata field {field} must be non-empty, bounded, and contain no controls")]
    InvalidMetadata { field: &'static str },
    #[error("asset focal point must be between 0 and 1000 per mille on each axis")]
    InvalidFocalPoint,
    #[error("asset revision must be greater than zero")]
    InvalidRevision,
    #[error("invalid quarantine record: {0}")]
    InvalidQuarantine(String),
    #[error("quarantined source has not been reviewed")]
    QuarantineNotReviewed,
    #[error("source is {actual} bytes, exceeding the {maximum}-byte limit")]
    SourceTooLarge { actual: u64, maximum: u64 },
    #[error("source width {actual} is below the required 1280 pixels; upscaling is disabled")]
    SourceTooNarrow { actual: u32 },
    #[error(
        "{kind} source width {actual} is below the required {minimum} pixels; upscaling is disabled"
    )]
    SourceTooNarrowForProfile {
        kind: AssetKind,
        actual: u32,
        minimum: u32,
    },
    #[error("source dimensions {width}x{height} exceed the configured limits")]
    InvalidDimensions { width: u32, height: u32 },
    #[error("unsupported source format; use PNG, JPEG, or WebP")]
    UnsupportedSourceFormat,
    #[error("animated image sources are unsupported; publish one reviewed static pose at a time")]
    UnsupportedAnimation,
    #[error("entity-sprite assets require a source with non-opaque alpha pixels")]
    SpriteRequiresTransparency,
    #[error("the lossless option is valid only for atmosphere assets")]
    LosslessSpriteProfile,
    #[error("required {0} encoder is unavailable")]
    MissingRequiredEncoder(AssetFormat),
    #[error("{format} encoder failed: {message}")]
    Encoder {
        format: AssetFormat,
        message: String,
    },
    #[error(
        "encoded {format} {width}px variant is {actual} bytes, exceeding the {maximum}-byte limit"
    )]
    VariantTooLarge {
        format: AssetFormat,
        width: u32,
        actual: usize,
        maximum: usize,
    },
    #[error("asset manifest is {actual} bytes, exceeding the {maximum}-byte client limit")]
    ManifestTooLarge { actual: usize, maximum: usize },
    #[error("immutable output already exists with different contents: {0}")]
    ImmutableCollision(PathBuf),
    #[error("image decode failed: {0}")]
    Image(#[from] image::ImageError),
    #[error("JSON serialization failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("I/O failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl PipelineError {
    fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

/// Encoder boundary kept separate from validation, resizing, hashing, and publication.
///
/// Tests and alternate platforms can supply a deterministic implementation without
/// changing the manifest contract. Implementations must return the exact bytes for
/// the advertised format; substituting formats is forbidden.
pub trait VariantEncoders {
    fn rgb_formats(&self) -> Vec<AssetFormat>;

    fn rgba_formats(&self) -> Vec<AssetFormat>;

    fn encode_rgb(&self, format: AssetFormat, image: &RgbImage) -> Result<Vec<u8>, PipelineError>;

    fn encode_rgba(&self, format: AssetFormat, image: &RgbaImage)
    -> Result<Vec<u8>, PipelineError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultEncoders;

impl VariantEncoders for DefaultEncoders {
    fn rgb_formats(&self) -> Vec<AssetFormat> {
        let mut formats = vec![AssetFormat::Avif];
        #[cfg(feature = "webp-encoding")]
        formats.push(AssetFormat::Webp);
        formats.push(AssetFormat::Jpeg);
        formats
    }

    fn rgba_formats(&self) -> Vec<AssetFormat> {
        #[cfg(feature = "webp-encoding")]
        return vec![AssetFormat::Webp];
        #[cfg(not(feature = "webp-encoding"))]
        Vec::new()
    }

    fn encode_rgb(&self, format: AssetFormat, image: &RgbImage) -> Result<Vec<u8>, PipelineError> {
        match format {
            AssetFormat::Avif => encode_avif(image),
            #[cfg(feature = "webp-encoding")]
            AssetFormat::Webp => encode_webp(image),
            #[cfg(not(feature = "webp-encoding"))]
            AssetFormat::Webp => Err(PipelineError::MissingRequiredEncoder(AssetFormat::Webp)),
            AssetFormat::Jpeg => encode_jpeg(image),
            AssetFormat::Png => encode_png_rgb(image),
        }
    }

    fn encode_rgba(
        &self,
        format: AssetFormat,
        image: &RgbaImage,
    ) -> Result<Vec<u8>, PipelineError> {
        match format {
            #[cfg(feature = "webp-encoding")]
            AssetFormat::Webp => encode_webp_rgba(image),
            #[cfg(not(feature = "webp-encoding"))]
            AssetFormat::Webp => Err(PipelineError::MissingRequiredEncoder(AssetFormat::Webp)),
            AssetFormat::Png => encode_png_rgba(image),
            other => Err(PipelineError::MissingRequiredEncoder(other)),
        }
    }
}

pub fn build_asset(request: &BuildRequest) -> Result<BuildResult, PipelineError> {
    build_asset_with(request, &DefaultEncoders)
}

/// Decode and re-encode an untrusted local upload into a metadata-free,
/// content-addressed PNG. The returned record remains unpublished and pending
/// review; this function has no network access and is never used by the server.
pub fn quarantine_source(request: &QuarantineRequest) -> Result<QuarantineResult, PipelineError> {
    let original = read_bounded_source(&request.source, request.limits.max_source_bytes)?;
    let original_sha256 = sha256_hex(&original);
    let decoded = decode_bounded_source(&original, request.limits)?;
    let (width, height) = decoded.dimensions();
    let transparent = has_transparency(&decoded);
    let sanitized = if transparent {
        encode_png_rgba(&decoded.to_rgba8())?
    } else {
        encode_png_rgb(&decoded.to_rgb8())?
    };
    if sanitized.len() as u64 > request.limits.max_source_bytes {
        return Err(PipelineError::SourceTooLarge {
            actual: sanitized.len() as u64,
            maximum: request.limits.max_source_bytes,
        });
    }
    let sanitized_sha256 = sha256_hex(&sanitized);
    let bundle_dir = request.quarantine_dir.join(&sanitized_sha256);
    let source_path = bundle_dir.join("source.png");
    write_immutable(&source_path, &sanitized)?;

    let pending = QuarantineRecord {
        schema: 1,
        original_sha256,
        sanitized_sha256,
        width,
        height,
        has_transparency: transparent,
        sanitized_path: "source.png".to_owned(),
        review: None,
    };
    let record_path = bundle_dir.join("quarantine.json");
    let record = if record_path.exists() {
        let existing = read_quarantine_record(&record_path, request.limits.max_source_bytes)?;
        if existing.original_sha256 != pending.original_sha256 {
            return Err(PipelineError::InvalidQuarantine(
                "sanitized collision has different original bytes".to_owned(),
            ));
        }
        existing
    } else {
        write_json_immutable(&record_path, &pending)?;
        pending
    };
    validate_quarantine_record(&record_path, &record, request.limits.max_source_bytes)?;
    Ok(QuarantineResult {
        record_path,
        record,
    })
}

/// Record the explicit human decision that makes a quarantined source eligible
/// for publication. Re-review is idempotent only with identical metadata.
pub fn review_quarantine(
    record_path: &Path,
    reviewer: &str,
    reviewed_at: &str,
    max_bytes: u64,
) -> Result<QuarantineResult, PipelineError> {
    let reviewer = validate_quarantine_field(reviewer, "reviewer")?;
    let reviewed_at = validate_quarantine_field(reviewed_at, "reviewed_at")?;
    let mut record = read_quarantine_record(record_path, max_bytes)?;
    validate_quarantine_record(record_path, &record, max_bytes)?;
    let review = QuarantineReview {
        reviewer,
        reviewed_at,
    };
    if let Some(existing) = &record.review {
        if existing != &review {
            return Err(PipelineError::InvalidQuarantine(
                "an existing review cannot be silently replaced".to_owned(),
            ));
        }
    } else {
        record.review = Some(review);
        write_json_atomic(record_path, &record)?;
    }
    Ok(QuarantineResult {
        record_path: record_path.to_owned(),
        record,
    })
}

/// Publish only the sanitized bytes named by a reviewed quarantine record.
pub fn build_quarantined(
    record_path: &Path,
    mut request: BuildRequest,
) -> Result<BuildResult, PipelineError> {
    let record = read_quarantine_record(record_path, request.limits.max_source_bytes)?;
    validate_quarantine_record(record_path, &record, request.limits.max_source_bytes)?;
    let review = record
        .review
        .as_ref()
        .ok_or(PipelineError::QuarantineNotReviewed)?;
    if request.metadata.human_reviewed_at.trim() != review.reviewed_at {
        return Err(PipelineError::InvalidQuarantine(
            "publication review timestamp does not match quarantine review".to_owned(),
        ));
    }
    request.source = record_path
        .parent()
        .ok_or_else(|| PipelineError::InvalidQuarantine("record has no parent".to_owned()))?
        .join("source.png");
    if matches!(request.profile, AssetProfile::Atmosphere) {
        request.lossless |= record.has_transparency;
    }
    build_asset(&request)
}

pub fn build_asset_with(
    request: &BuildRequest,
    encoders: &dyn VariantEncoders,
) -> Result<BuildResult, PipelineError> {
    let alt = validate_alt(&request.alt, request.limits.max_alt_chars)?;
    let metadata = validate_metadata(&request.metadata)?;
    let source_bytes = read_bounded_source(&request.source, request.limits.max_source_bytes)?;
    let source_sha256 = sha256_hex(&source_bytes);
    let source = decode_bounded_source(&source_bytes, request.limits)?;
    let (source_width, source_height) = source.dimensions();

    let minimum_source_width = request.profile.minimum_source_width();
    if source_width < minimum_source_width {
        return if matches!(request.profile, AssetProfile::Atmosphere) {
            Err(PipelineError::SourceTooNarrow {
                actual: source_width,
            })
        } else {
            Err(PipelineError::SourceTooNarrowForProfile {
                kind: request.profile.kind(),
                actual: source_width,
                minimum: minimum_source_width,
            })
        };
    }
    let has_transparency = has_transparency(&source);
    let formats = match request.profile {
        AssetProfile::Atmosphere if request.lossless || has_transparency => {
            vec![AssetFormat::Png]
        }
        AssetProfile::Atmosphere => {
            let formats = encoders.rgb_formats();
            for required in [AssetFormat::Avif, AssetFormat::Jpeg] {
                if !formats.contains(&required) {
                    return Err(PipelineError::MissingRequiredEncoder(required));
                }
            }
            formats
        }
        AssetProfile::EntitySprite { .. } if request.lossless => {
            return Err(PipelineError::LosslessSpriteProfile);
        }
        AssetProfile::EntitySprite { .. } if !has_transparency => {
            return Err(PipelineError::SpriteRequiresTransparency);
        }
        AssetProfile::EntitySprite { .. } => {
            let mut formats = encoders.rgba_formats();
            if !formats.contains(&AssetFormat::Webp) {
                return Err(PipelineError::MissingRequiredEncoder(AssetFormat::Webp));
            }
            formats.retain(|format| *format == AssetFormat::Webp);
            formats.push(AssetFormat::Png);
            formats
        }
    };

    let source_rgb = source.to_rgb8();
    let source_rgba = (request.lossless || has_transparency).then(|| source.to_rgba8());
    let dominant_color = dominant_color(&source_rgb);
    let target_widths = request.profile.target_widths();
    let mut encoded_variants = Vec::with_capacity(target_widths.len() * formats.len());
    let maximum_variant_bytes = if matches!(request.profile, AssetProfile::EntitySprite { .. }) {
        request
            .limits
            .max_variant_bytes
            .min(DEFAULT_SPRITE_MAX_VARIANT_BYTES)
    } else {
        request.limits.max_variant_bytes
    };
    for &width in target_widths {
        let height = scaled_height(source_width, source_height, width);
        let resized = image::imageops::resize(&source_rgb, width, height, FilterType::Lanczos3);
        let resized_rgba = source_rgba
            .as_ref()
            .map(|image| image::imageops::resize(image, width, height, FilterType::Lanczos3));

        for format in &formats {
            let bytes = if matches!(request.profile, AssetProfile::EntitySprite { .. })
                && *format == AssetFormat::Png
            {
                encode_png_rgba(resized_rgba.as_ref().expect("sprite RGBA source prepared"))?
            } else if matches!(request.profile, AssetProfile::EntitySprite { .. }) {
                encoders.encode_rgba(
                    *format,
                    resized_rgba.as_ref().expect("sprite RGBA source prepared"),
                )?
            } else if *format == AssetFormat::Png {
                encode_png_rgba(resized_rgba.as_ref().expect("PNG source prepared"))?
            } else {
                encoders.encode_rgb(*format, &resized)?
            };
            if bytes.len() > maximum_variant_bytes {
                return Err(PipelineError::VariantTooLarge {
                    format: *format,
                    width,
                    actual: bytes.len(),
                    maximum: maximum_variant_bytes,
                });
            }
            encoded_variants.push(EncodedVariant {
                format: *format,
                width,
                height,
                sha256: sha256_hex(&bytes),
                bytes,
            });
        }
    }

    let asset_hash = asset_digest(
        &alt,
        &source_sha256,
        (source_width, source_height),
        &metadata,
        &dominant_color,
        request.profile,
        &encoded_variants,
    );
    let asset_id = format!("sha256:{asset_hash}");
    let bundle_dir = request.output_dir.join(&asset_hash);
    let variants = encoded_variants
        .iter()
        .map(|variant| {
            let file_name = variant_file_name(request.profile, variant.width, variant.format);
            PublishedVariant {
                format: variant.format,
                media_type: variant.format.media_type().to_owned(),
                width: variant.width,
                height: variant.height,
                bytes: variant.bytes.len(),
                sha256: variant.sha256.clone(),
                path: format!("{asset_hash}/{file_name}"),
            }
        })
        .collect();

    let manifest = AssetManifest {
        schema: match request.profile {
            AssetProfile::Atmosphere => ATMOSPHERE_SCHEMA_VERSION,
            AssetProfile::EntitySprite { .. } => ENTITY_SPRITE_SCHEMA_VERSION,
        },
        asset_id: asset_id.clone(),
        revision: metadata.revision,
        kind: request.profile.kind(),
        pose: request.profile.pose(),
        alt,
        focal_point: metadata.focal_point,
        dominant_color,
        author: metadata.author,
        license: metadata.license,
        provenance: metadata.provenance,
        generator: metadata.generator,
        content_classification: metadata.content_classification,
        human_reviewed_at: metadata.human_reviewed_at,
        source: SourceDescriptor {
            sha256: source_sha256,
            width: source_width,
            height: source_height,
        },
        variants,
    };
    let mut manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
    manifest_bytes.push(b'\n');
    if manifest_bytes.len() > MAX_MANIFEST_BYTES {
        return Err(PipelineError::ManifestTooLarge {
            actual: manifest_bytes.len(),
            maximum: MAX_MANIFEST_BYTES,
        });
    }
    let manifest_sha256 = sha256_hex(&manifest_bytes);

    // Validate the complete publish marker before writing any part of its
    // bundle. A rejected manifest must not leave an orphaned asset directory.
    fs::create_dir_all(&bundle_dir).map_err(|error| PipelineError::io(&bundle_dir, error))?;
    for variant in &encoded_variants {
        let file_name = variant_file_name(request.profile, variant.width, variant.format);
        write_immutable(&bundle_dir.join(file_name), &variant.bytes)?;
    }
    let manifest_path = bundle_dir.join("manifest.json");
    write_immutable(&manifest_path, &manifest_bytes)?;

    Ok(BuildResult {
        asset_id,
        manifest_sha256,
        manifest_path,
        manifest,
    })
}

#[derive(Debug)]
struct EncodedVariant {
    format: AssetFormat,
    width: u32,
    height: u32,
    sha256: String,
    bytes: Vec<u8>,
}

fn validate_alt(alt: &str, max_chars: usize) -> Result<String, PipelineError> {
    let alt = alt.trim();
    if alt.is_empty() {
        return Err(PipelineError::EmptyAlt);
    }
    if alt.chars().count() > max_chars {
        return Err(PipelineError::AltTooLong { maximum: max_chars });
    }
    if alt.chars().any(char::is_control) {
        return Err(PipelineError::InvalidAltCharacters);
    }
    Ok(alt.to_owned())
}

fn validate_metadata(metadata: &AuthoringMetadata) -> Result<AuthoringMetadata, PipelineError> {
    if metadata.revision == 0 {
        return Err(PipelineError::InvalidRevision);
    }
    if metadata.focal_point.x_per_mille > 1000 || metadata.focal_point.y_per_mille > 1000 {
        return Err(PipelineError::InvalidFocalPoint);
    }

    fn field(value: &str, name: &'static str) -> Result<String, PipelineError> {
        let value = value.trim();
        if value.is_empty() || value.len() > 1024 || value.chars().any(char::is_control) {
            return Err(PipelineError::InvalidMetadata { field: name });
        }
        Ok(value.to_owned())
    }

    Ok(AuthoringMetadata {
        revision: metadata.revision,
        focal_point: metadata.focal_point,
        author: field(&metadata.author, "author")?,
        license: field(&metadata.license, "license")?,
        provenance: field(&metadata.provenance, "provenance")?,
        generator: metadata
            .generator
            .as_deref()
            .map(|value| field(value, "generator"))
            .transpose()?,
        content_classification: field(&metadata.content_classification, "content_classification")?,
        human_reviewed_at: field(&metadata.human_reviewed_at, "human_reviewed_at")?,
    })
}

fn dominant_color(image: &RgbImage) -> String {
    let (red, green, blue, count) = image.pixels().fold(
        (0_u64, 0_u64, 0_u64, 0_u64),
        |(red, green, blue, count), pixel| {
            (
                red + u64::from(pixel.0[0]),
                green + u64::from(pixel.0[1]),
                blue + u64::from(pixel.0[2]),
                count + 1,
            )
        },
    );
    format!(
        "#{:02x}{:02x}{:02x}",
        red / count.max(1),
        green / count.max(1),
        blue / count.max(1)
    )
}

fn read_bounded_source(path: &Path, max_bytes: u64) -> Result<Vec<u8>, PipelineError> {
    let metadata = fs::metadata(path).map_err(|error| PipelineError::io(path, error))?;
    if metadata.len() > max_bytes {
        return Err(PipelineError::SourceTooLarge {
            actual: metadata.len(),
            maximum: max_bytes,
        });
    }
    fs::read(path).map_err(|error| PipelineError::io(path, error))
}

fn read_quarantine_record(path: &Path, max_bytes: u64) -> Result<QuarantineRecord, PipelineError> {
    let bytes = read_bounded_source(path, max_bytes.min(64 * 1024))?;
    serde_json::from_slice(&bytes).map_err(|error| {
        PipelineError::InvalidQuarantine(format!("record JSON is invalid: {error}"))
    })
}

fn validate_quarantine_record(
    record_path: &Path,
    record: &QuarantineRecord,
    max_bytes: u64,
) -> Result<(), PipelineError> {
    if record.schema != 1
        || record.sanitized_path != "source.png"
        || record.width == 0
        || record.height == 0
        || record.original_sha256.len() != 64
        || record.sanitized_sha256.len() != 64
        || !record
            .original_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || !record
            .sanitized_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(PipelineError::InvalidQuarantine(
            "record fields violate schema 1".to_owned(),
        ));
    }
    if let Some(review) = &record.review {
        validate_quarantine_field(&review.reviewer, "reviewer")?;
        validate_quarantine_field(&review.reviewed_at, "reviewed_at")?;
    }
    let parent = record_path
        .parent()
        .ok_or_else(|| PipelineError::InvalidQuarantine("record has no parent".to_owned()))?;
    if parent.file_name().and_then(|name| name.to_str()) != Some(record.sanitized_sha256.as_str()) {
        return Err(PipelineError::InvalidQuarantine(
            "record directory does not match sanitized digest".to_owned(),
        ));
    }
    let sanitized_path = parent.join("source.png");
    let sanitized = read_bounded_source(&sanitized_path, max_bytes)?;
    if sha256_hex(&sanitized) != record.sanitized_sha256 {
        return Err(PipelineError::InvalidQuarantine(
            "sanitized source digest mismatch".to_owned(),
        ));
    }
    let decoded = decode_bounded_source(
        &sanitized,
        BuildLimits {
            max_source_bytes: max_bytes,
            ..BuildLimits::default()
        },
    )?;
    if decoded.dimensions() != (record.width, record.height)
        || has_transparency(&decoded) != record.has_transparency
    {
        return Err(PipelineError::InvalidQuarantine(
            "sanitized source properties mismatch".to_owned(),
        ));
    }
    Ok(())
}

fn validate_quarantine_field(value: &str, name: &'static str) -> Result<String, PipelineError> {
    let value = value.trim();
    if value.is_empty() || value.len() > 1024 || value.chars().any(char::is_control) {
        return Err(PipelineError::InvalidMetadata { field: name });
    }
    Ok(value.to_owned())
}

fn decode_bounded_source(bytes: &[u8], limits: BuildLimits) -> Result<DynamicImage, PipelineError> {
    let format = image::guess_format(bytes)?;
    if !matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP
    ) {
        return Err(PipelineError::UnsupportedSourceFormat);
    }
    if is_animated_source(bytes, format) {
        return Err(PipelineError::UnsupportedAnimation);
    }

    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut decode_limits = image::Limits::default();
    decode_limits.max_image_width = Some(limits.max_dimension);
    decode_limits.max_image_height = Some(limits.max_dimension);
    decode_limits.max_alloc = Some(limits.max_decoded_bytes);
    reader.limits(decode_limits);
    let decoded = reader.decode()?;
    let (width, height) = decoded.dimensions();
    let pixels = u64::from(width).saturating_mul(u64::from(height));
    if width == 0
        || height == 0
        || width > limits.max_dimension
        || height > limits.max_dimension
        || pixels > limits.max_pixels
    {
        return Err(PipelineError::InvalidDimensions { width, height });
    }
    Ok(decoded)
}

fn is_animated_source(bytes: &[u8], format: ImageFormat) -> bool {
    match format {
        ImageFormat::Png => png_has_animation_control(bytes),
        ImageFormat::WebP => webp_has_animation_chunks(bytes),
        _ => false,
    }
}

fn png_has_animation_control(bytes: &[u8]) -> bool {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return false;
    }
    let mut offset = 8_usize;
    while offset.saturating_add(12) <= bytes.len() {
        let length = u32::from_be_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]) as usize;
        let chunk_end = match offset
            .checked_add(12)
            .and_then(|base| base.checked_add(length))
        {
            Some(end) if end <= bytes.len() => end,
            _ => return false,
        };
        if &bytes[offset + 4..offset + 8] == b"acTL" {
            return true;
        }
        if &bytes[offset + 4..offset + 8] == b"IEND" {
            return false;
        }
        offset = chunk_end;
    }
    false
}

fn webp_has_animation_chunks(bytes: &[u8]) -> bool {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return false;
    }
    let mut offset = 12_usize;
    while offset.saturating_add(8) <= bytes.len() {
        let chunk = &bytes[offset..offset + 4];
        let length = u32::from_le_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]) as usize;
        if chunk == b"ANIM" || chunk == b"ANMF" {
            return true;
        }
        let padded = match length.checked_add(length & 1) {
            Some(value) => value,
            None => return false,
        };
        offset = match offset
            .checked_add(8)
            .and_then(|base| base.checked_add(padded))
        {
            Some(value) if value <= bytes.len() => value,
            _ => return false,
        };
    }
    false
}

fn has_transparency(source: &DynamicImage) -> bool {
    source.color().has_alpha()
        && source
            .to_rgba8()
            .pixels()
            .any(|pixel| pixel.0[3] != u8::MAX)
}

fn scaled_height(source_width: u32, source_height: u32, target_width: u32) -> u32 {
    let numerator = u64::from(source_height) * u64::from(target_width);
    ((numerator + u64::from(source_width) / 2) / u64::from(source_width)).max(1) as u32
}

fn encode_avif(image: &RgbImage) -> Result<Vec<u8>, PipelineError> {
    let pixels: Vec<ravif::RGB8> = image
        .pixels()
        .map(|pixel| ravif::RGB8::new(pixel.0[0], pixel.0[1], pixel.0[2]))
        .collect();
    let encoded = ravif::Encoder::new()
        .with_quality(AVIF_QUALITY)
        .with_speed(AVIF_SPEED)
        .with_num_threads(Some(1))
        .encode_rgb(ravif::Img::new(
            &pixels,
            image.width() as usize,
            image.height() as usize,
        ))
        .map_err(|error| PipelineError::Encoder {
            format: AssetFormat::Avif,
            message: error.to_string(),
        })?;
    Ok(encoded.avif_file)
}

#[cfg(feature = "webp-encoding")]
fn encode_webp(image: &RgbImage) -> Result<Vec<u8>, PipelineError> {
    webp::Encoder::from_rgb(image.as_raw(), image.width(), image.height())
        .encode_simple(false, WEBP_QUALITY)
        .map(|memory| memory.to_vec())
        .map_err(|error| PipelineError::Encoder {
            format: AssetFormat::Webp,
            message: format!("{error:?}"),
        })
}

#[cfg(feature = "webp-encoding")]
fn encode_webp_rgba(image: &RgbaImage) -> Result<Vec<u8>, PipelineError> {
    webp::Encoder::from_rgba(image.as_raw(), image.width(), image.height())
        .encode_simple(false, WEBP_QUALITY)
        .map(|memory| memory.to_vec())
        .map_err(|error| PipelineError::Encoder {
            format: AssetFormat::Webp,
            message: format!("{error:?}"),
        })
}

fn encode_jpeg(image: &RgbImage) -> Result<Vec<u8>, PipelineError> {
    let mut output = Vec::new();
    JpegEncoder::new_with_quality(&mut output, JPEG_QUALITY)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|error| PipelineError::Encoder {
            format: AssetFormat::Jpeg,
            message: error.to_string(),
        })?;
    Ok(output)
}

fn encode_png_rgb(image: &RgbImage) -> Result<Vec<u8>, PipelineError> {
    let mut output = Vec::new();
    PngEncoder::new(&mut output)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|error| PipelineError::Encoder {
            format: AssetFormat::Png,
            message: error.to_string(),
        })?;
    Ok(output)
}

fn encode_png_rgba(image: &image::RgbaImage) -> Result<Vec<u8>, PipelineError> {
    let mut output = Vec::new();
    PngEncoder::new(&mut output)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|error| PipelineError::Encoder {
            format: AssetFormat::Png,
            message: error.to_string(),
        })?;
    Ok(output)
}

fn asset_digest(
    alt: &str,
    source_sha256: &str,
    source_dimensions: (u32, u32),
    metadata: &AuthoringMetadata,
    dominant_color: &str,
    profile: AssetProfile,
    variants: &[EncodedVariant],
) -> String {
    let (source_width, source_height) = source_dimensions;
    let mut digest = Sha256::new();
    match profile {
        AssetProfile::Atmosphere => digest.update(b"tiamud-atmospheric-asset-v2\0"),
        AssetProfile::EntitySprite { pose } => {
            digest.update(b"tiamud-entity-sprite-v1\0");
            digest.update(pose.as_str().as_bytes());
            digest.update(b"\0");
        }
    }
    digest.update((alt.len() as u64).to_be_bytes());
    digest.update(alt.as_bytes());
    digest.update(source_sha256.as_bytes());
    digest.update(source_width.to_be_bytes());
    digest.update(source_height.to_be_bytes());
    digest.update(metadata.revision.to_be_bytes());
    digest.update(metadata.focal_point.x_per_mille.to_be_bytes());
    digest.update(metadata.focal_point.y_per_mille.to_be_bytes());
    for value in [
        metadata.author.as_str(),
        metadata.license.as_str(),
        metadata.provenance.as_str(),
        metadata.generator.as_deref().unwrap_or_default(),
        metadata.content_classification.as_str(),
        metadata.human_reviewed_at.as_str(),
        dominant_color,
    ] {
        digest.update((value.len() as u64).to_be_bytes());
        digest.update(value.as_bytes());
    }
    for variant in variants {
        digest.update([match variant.format {
            AssetFormat::Avif => 1,
            AssetFormat::Webp => 2,
            AssetFormat::Jpeg => 3,
            AssetFormat::Png => 4,
        }]);
        digest.update(variant.width.to_be_bytes());
        digest.update(variant.height.to_be_bytes());
        digest.update(variant.sha256.as_bytes());
    }
    hex::encode(digest.finalize())
}

fn variant_file_name(profile: AssetProfile, width: u32, format: AssetFormat) -> String {
    match profile {
        AssetProfile::Atmosphere => format!("scene-{width}.{}", format.extension()),
        AssetProfile::EntitySprite { pose } => {
            format!("sprite-{pose}-{width}.{}", format.extension())
        }
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn write_immutable(path: &Path, bytes: &[u8]) -> Result<(), PipelineError> {
    if path.exists() {
        let existing = fs::read(path).map_err(|error| PipelineError::io(path, error))?;
        return if existing == bytes {
            Ok(())
        } else {
            Err(PipelineError::ImmutableCollision(path.to_owned()))
        };
    }

    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| PipelineError::io(parent, error))?;
    let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("asset");
    let temporary = parent.join(format!(
        ".{file_name}.{}.{}.part",
        std::process::id(),
        sequence
    ));

    let result = (|| {
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| PipelineError::io(&temporary, error))?;
        output
            .write_all(bytes)
            .map_err(|error| PipelineError::io(&temporary, error))?;
        output
            .sync_all()
            .map_err(|error| PipelineError::io(&temporary, error))?;

        match fs::hard_link(&temporary, path) {
            Ok(()) => {
                // Persist the directory entry as well as the file contents.
                fs::File::open(parent)
                    .and_then(|directory| directory.sync_all())
                    .map_err(|error| PipelineError::io(parent, error))?;
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let existing = fs::read(path).map_err(|error| PipelineError::io(path, error))?;
                if existing == bytes {
                    Ok(())
                } else {
                    Err(PipelineError::ImmutableCollision(path.to_owned()))
                }
            }
            Err(error) => Err(PipelineError::io(path, error)),
        }
    })();

    if temporary.exists() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn write_json_immutable<T: Serialize>(path: &Path, value: &T) -> Result<(), PipelineError> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    write_immutable(path, &bytes)
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), PipelineError> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| PipelineError::io(parent, error))?;
    let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = parent.join(format!(
        ".quarantine.{}.{}.part",
        std::process::id(),
        sequence
    ));
    let result = (|| {
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| PipelineError::io(&temporary, error))?;
        output
            .write_all(&bytes)
            .map_err(|error| PipelineError::io(&temporary, error))?;
        output
            .sync_all()
            .map_err(|error| PipelineError::io(&temporary, error))?;
        fs::rename(&temporary, path).map_err(|error| PipelineError::io(path, error))
    })();
    if temporary.exists() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::codecs::png::PngEncoder;

    #[derive(Debug)]
    struct FakeEncoders;

    impl VariantEncoders for FakeEncoders {
        fn rgb_formats(&self) -> Vec<AssetFormat> {
            vec![AssetFormat::Avif, AssetFormat::Webp, AssetFormat::Jpeg]
        }

        fn rgba_formats(&self) -> Vec<AssetFormat> {
            vec![AssetFormat::Webp]
        }

        fn encode_rgb(
            &self,
            format: AssetFormat,
            image: &RgbImage,
        ) -> Result<Vec<u8>, PipelineError> {
            let mut output = format!("{format}:{}x{}:", image.width(), image.height()).into_bytes();
            output.extend_from_slice(Sha256::digest(image.as_raw()).as_slice());
            Ok(output)
        }

        fn encode_rgba(
            &self,
            format: AssetFormat,
            image: &RgbaImage,
        ) -> Result<Vec<u8>, PipelineError> {
            let mut output = format!("{format}:{}x{}:", image.width(), image.height()).into_bytes();
            output.extend_from_slice(Sha256::digest(image.as_raw()).as_slice());
            Ok(output)
        }
    }

    fn write_png(path: &Path, width: u32, height: u32, alpha: Option<u8>) {
        let mut bytes = Vec::new();
        if let Some(alpha) = alpha {
            let pixels = vec![32_u8, 64, 96, alpha]
                .into_iter()
                .cycle()
                .take(width as usize * height as usize * 4)
                .collect::<Vec<_>>();
            PngEncoder::new(&mut bytes)
                .write_image(&pixels, width, height, image::ExtendedColorType::Rgba8)
                .unwrap();
        } else {
            let pixels = vec![32_u8, 64, 96]
                .into_iter()
                .cycle()
                .take(width as usize * height as usize * 3)
                .collect::<Vec<_>>();
            PngEncoder::new(&mut bytes)
                .write_image(&pixels, width, height, image::ExtendedColorType::Rgb8)
                .unwrap();
        }
        fs::write(path, bytes).unwrap();
    }

    fn reviewed_metadata() -> AuthoringMetadata {
        AuthoringMetadata {
            revision: 1,
            focal_point: FocalPoint {
                x_per_mille: 500,
                y_per_mille: 500,
            },
            author: "Tiamud test suite".to_owned(),
            license: "LicenseRef-Test".to_owned(),
            provenance: "Generated test fixture".to_owned(),
            generator: None,
            content_classification: "general".to_owned(),
            human_reviewed_at: "2026-07-15T00:00:00Z".to_owned(),
        }
    }

    #[test]
    fn validates_alt_before_touching_the_source() {
        let request = BuildRequest {
            source: PathBuf::from("does-not-exist.png"),
            alt: " \n ".to_owned(),
            output_dir: PathBuf::from("unused"),
            limits: BuildLimits::default(),
            metadata: reviewed_metadata(),
            profile: AssetProfile::Atmosphere,
            lossless: false,
        };
        assert!(matches!(
            build_asset_with(&request, &FakeEncoders),
            Err(PipelineError::EmptyAlt)
        ));
    }

    #[test]
    fn requires_reviewed_metadata_before_touching_the_source() {
        let mut metadata = reviewed_metadata();
        metadata.revision = 0;
        let request = BuildRequest {
            source: PathBuf::from("does-not-exist.png"),
            alt: "A reviewed scene.".to_owned(),
            output_dir: PathBuf::from("unused"),
            limits: BuildLimits::default(),
            metadata,
            profile: AssetProfile::Atmosphere,
            lossless: false,
        };
        assert!(matches!(
            build_asset_with(&request, &FakeEncoders),
            Err(PipelineError::InvalidRevision)
        ));

        let mut request = request;
        request.metadata.revision = 1;
        request.metadata.focal_point.x_per_mille = 1001;
        assert!(matches!(
            build_asset_with(&request, &FakeEncoders),
            Err(PipelineError::InvalidFocalPoint)
        ));
    }

    #[test]
    fn rejects_encoded_sources_over_the_bound() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("oversized.bin");
        fs::write(&source, [0_u8; 9]).unwrap();
        let request = BuildRequest {
            source,
            alt: "A bounded test image.".to_owned(),
            output_dir: temp.path().join("out"),
            limits: BuildLimits {
                max_source_bytes: 8,
                ..BuildLimits::default()
            },
            metadata: reviewed_metadata(),
            profile: AssetProfile::Atmosphere,
            lossless: false,
        };
        assert!(matches!(
            build_asset_with(&request, &FakeEncoders),
            Err(PipelineError::SourceTooLarge { .. })
        ));
    }

    #[test]
    fn rejects_upscaling_and_preserves_transparency_as_png() {
        let temp = tempfile::tempdir().unwrap();
        let narrow = temp.path().join("narrow.png");
        write_png(&narrow, 1279, 4, None);
        let narrow_request = BuildRequest {
            source: narrow,
            alt: "A narrow backdrop.".to_owned(),
            output_dir: temp.path().join("narrow-out"),
            limits: BuildLimits::default(),
            metadata: reviewed_metadata(),
            profile: AssetProfile::Atmosphere,
            lossless: false,
        };
        assert!(matches!(
            build_asset_with(&narrow_request, &FakeEncoders),
            Err(PipelineError::SourceTooNarrow { actual: 1279 })
        ));

        let transparent = temp.path().join("transparent.png");
        write_png(&transparent, 1280, 4, Some(128));
        let transparent_request = BuildRequest {
            source: transparent,
            alt: "A translucent backdrop.".to_owned(),
            output_dir: temp.path().join("transparent-out"),
            limits: BuildLimits::default(),
            metadata: reviewed_metadata(),
            profile: AssetProfile::Atmosphere,
            lossless: false,
        };
        let result = build_asset_with(&transparent_request, &FakeEncoders).unwrap();
        assert_eq!(result.manifest.variants.len(), 2);
        assert!(
            result
                .manifest
                .variants
                .iter()
                .all(|variant| variant.format == AssetFormat::Png)
        );
        let published = fs::read(
            result
                .manifest_path
                .parent()
                .unwrap()
                .join("scene-1280.png"),
        )
        .unwrap();
        let decoded = image::load_from_memory_with_format(&published, ImageFormat::Png).unwrap();
        assert!(has_transparency(&decoded));
    }

    #[test]
    fn repeated_builds_are_byte_for_byte_deterministic() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.png");
        write_png(&source, 1280, 8, None);
        let request_one = BuildRequest {
            source: source.clone(),
            alt: "  An empty stone plaza under a quiet blue dusk.  ".to_owned(),
            output_dir: temp.path().join("one"),
            limits: BuildLimits::default(),
            metadata: reviewed_metadata(),
            profile: AssetProfile::Atmosphere,
            lossless: false,
        };
        let request_two = BuildRequest {
            output_dir: temp.path().join("two"),
            ..request_one.clone()
        };

        let first = build_asset_with(&request_one, &FakeEncoders).unwrap();
        let second = build_asset_with(&request_two, &FakeEncoders).unwrap();

        assert_eq!(first.asset_id, second.asset_id);
        assert_eq!(first.manifest_sha256, second.manifest_sha256);
        assert_eq!(first.manifest, second.manifest);
        assert_eq!(
            fs::read(first.manifest_path).unwrap(),
            fs::read(second.manifest_path).unwrap()
        );
        assert_eq!(
            first.manifest.alt,
            "An empty stone plaza under a quiet blue dusk."
        );
    }

    #[test]
    fn oversized_manifest_is_rejected_before_creating_a_bundle() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.png");
        let output_dir = temp.path().join("published");
        write_png(&source, 1280, 8, None);
        let limits = BuildLimits {
            max_alt_chars: 70_000,
            ..BuildLimits::default()
        };
        let request = BuildRequest {
            source,
            alt: "a".repeat(70_000),
            output_dir: output_dir.clone(),
            limits,
            metadata: reviewed_metadata(),
            profile: AssetProfile::Atmosphere,
            lossless: false,
        };

        assert!(matches!(
            build_asset_with(&request, &FakeEncoders),
            Err(PipelineError::ManifestTooLarge { .. })
        ));
        assert!(!output_dir.exists());
    }

    #[test]
    fn concurrent_immutable_writers_never_replace_the_winner() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("asset.bin");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let handles = [b"first".as_slice(), b"second".as_slice()].map(|payload| {
            let path = path.clone();
            let barrier = std::sync::Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                write_immutable(&path, payload)
            })
        });
        barrier.wait();
        let outcomes = handles.map(|handle| handle.join().unwrap());
        assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| matches!(outcome, Err(PipelineError::ImmutableCollision(_))))
                .count(),
            1
        );
        let published = fs::read(path).unwrap();
        assert!(published == b"first" || published == b"second");
    }

    #[test]
    fn quarantine_requires_review_and_detects_tampering_before_publication() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("upload.png");
        write_png(&source, 1280, 4, Some(128));
        let staged = quarantine_source(&QuarantineRequest {
            source,
            quarantine_dir: temp.path().join("quarantine"),
            limits: BuildLimits::default(),
        })
        .unwrap();
        assert!(staged.record.review.is_none());
        assert!(staged.record.has_transparency);

        let publication = BuildRequest {
            source: PathBuf::new(),
            alt: "A translucent sigil over dark stone.".to_owned(),
            output_dir: temp.path().join("published"),
            limits: BuildLimits::default(),
            metadata: reviewed_metadata(),
            profile: AssetProfile::Atmosphere,
            lossless: false,
        };
        assert!(matches!(
            build_quarantined(&staged.record_path, publication.clone()),
            Err(PipelineError::QuarantineNotReviewed)
        ));

        review_quarantine(
            &staged.record_path,
            "Tiamud reviewer",
            "2026-07-15T00:00:00Z",
            BuildLimits::default().max_source_bytes,
        )
        .unwrap();
        let result = build_quarantined(&staged.record_path, publication.clone()).unwrap();
        assert!(
            result
                .manifest
                .variants
                .iter()
                .all(|variant| variant.format == AssetFormat::Png)
        );

        let sanitized = staged.record_path.parent().unwrap().join("source.png");
        fs::write(sanitized, b"tampered").unwrap();
        assert!(matches!(
            build_quarantined(&staged.record_path, publication),
            Err(PipelineError::InvalidQuarantine(_))
                | Err(PipelineError::Image(_))
                | Err(PipelineError::UnsupportedSourceFormat)
        ));
    }

    fn sprite_request(source: PathBuf, output_dir: PathBuf, pose: SpritePose) -> BuildRequest {
        BuildRequest {
            source,
            alt: "A translucent training automaton.".to_owned(),
            output_dir,
            limits: BuildLimits::default(),
            metadata: reviewed_metadata(),
            profile: AssetProfile::EntitySprite { pose },
            lossless: false,
        }
    }

    #[cfg(feature = "webp-encoding")]
    #[test]
    fn static_sprite_manifest_and_variants_are_bounded_deterministic_and_preserve_alpha() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("sprite.png");
        write_png(&source, 640, 8, Some(96));
        let first_request = sprite_request(
            source.clone(),
            temp.path().join("published-one"),
            SpritePose::Hit,
        );
        let second_request =
            sprite_request(source, temp.path().join("published-two"), SpritePose::Hit);

        let first = build_asset(&first_request).unwrap();
        let second = build_asset(&second_request).unwrap();

        assert_eq!(first.asset_id, second.asset_id);
        assert_eq!(first.manifest_sha256, second.manifest_sha256);
        assert_eq!(first.manifest, second.manifest);
        assert_eq!(first.manifest.schema, ENTITY_SPRITE_SCHEMA_VERSION);
        assert_eq!(first.manifest.kind, AssetKind::EntitySprite);
        assert_eq!(first.manifest.pose, Some(SpritePose::Hit));
        assert_eq!(first.manifest.variants.len(), 4);
        assert!(
            first
                .manifest
                .variants
                .iter()
                .all(|variant| variant.bytes <= DEFAULT_SPRITE_MAX_VARIANT_BYTES)
        );
        assert_eq!(
            first
                .manifest
                .variants
                .iter()
                .map(|variant| (variant.format, variant.width))
                .collect::<Vec<_>>(),
            vec![
                (AssetFormat::Webp, 320),
                (AssetFormat::Png, 320),
                (AssetFormat::Webp, 640),
                (AssetFormat::Png, 640),
            ]
        );

        let asset_hash = first.asset_id.strip_prefix("sha256:").unwrap();
        for variant in &first.manifest.variants {
            assert_eq!(
                variant.path,
                format!(
                    "{asset_hash}/sprite-hit-{}.{}",
                    variant.width,
                    variant.format.extension()
                )
            );
            assert_ne!(variant.format, AssetFormat::Jpeg);
            assert_ne!(variant.format, AssetFormat::Avif);
            let bytes = fs::read(
                first
                    .manifest_path
                    .parent()
                    .unwrap()
                    .join(variant.path.rsplit('/').next().unwrap()),
            )
            .unwrap();
            let decoded = image::load_from_memory_with_format(
                &bytes,
                match variant.format {
                    AssetFormat::Webp => ImageFormat::WebP,
                    AssetFormat::Png => ImageFormat::Png,
                    other => panic!("unexpected sprite format {other}"),
                },
            )
            .unwrap()
            .to_rgba8();
            assert!(decoded.pixels().all(|pixel| pixel.0[3] == 96));

            let first_bytes = fs::read(
                first
                    .manifest_path
                    .parent()
                    .unwrap()
                    .join(variant.path.rsplit('/').next().unwrap()),
            )
            .unwrap();
            let second_bytes = fs::read(
                second
                    .manifest_path
                    .parent()
                    .unwrap()
                    .join(variant.path.rsplit('/').next().unwrap()),
            )
            .unwrap();
            assert_eq!(first_bytes, second_bytes);
        }
        assert_eq!(
            fs::read(first.manifest_path).unwrap(),
            fs::read(second.manifest_path).unwrap()
        );
    }

    #[test]
    fn atmosphere_profile_keeps_schema_two_names_widths_and_format_policy() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("atmosphere.png");
        write_png(&source, 1280, 8, None);
        let result = build_asset_with(
            &BuildRequest {
                source,
                alt: "A quiet atmospheric backdrop.".to_owned(),
                output_dir: temp.path().join("published"),
                limits: BuildLimits::default(),
                metadata: reviewed_metadata(),
                profile: AssetProfile::Atmosphere,
                lossless: false,
            },
            &FakeEncoders,
        )
        .unwrap();

        assert_eq!(result.manifest.schema, 2);
        assert_eq!(result.manifest.kind, AssetKind::Atmosphere);
        assert_eq!(result.manifest.pose, None);
        assert_eq!(
            result
                .manifest
                .variants
                .iter()
                .map(|variant| (variant.format, variant.width))
                .collect::<Vec<_>>(),
            vec![
                (AssetFormat::Avif, 640),
                (AssetFormat::Webp, 640),
                (AssetFormat::Jpeg, 640),
                (AssetFormat::Avif, 1280),
                (AssetFormat::Webp, 1280),
                (AssetFormat::Jpeg, 1280),
            ]
        );
        assert!(
            result
                .manifest
                .variants
                .iter()
                .all(|variant| { variant.path.contains(&format!("/scene-{}.", variant.width)) })
        );
        let manifest_bytes = fs::read(result.manifest_path).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
        assert!(json.get("pose").is_none());
        let reparsed: AssetManifest = serde_json::from_slice(&manifest_bytes).unwrap();
        assert_eq!(reparsed, result.manifest);
    }

    #[derive(Debug)]
    struct OversizedSpriteEncoders;

    impl VariantEncoders for OversizedSpriteEncoders {
        fn rgb_formats(&self) -> Vec<AssetFormat> {
            vec![AssetFormat::Avif, AssetFormat::Jpeg]
        }

        fn rgba_formats(&self) -> Vec<AssetFormat> {
            vec![AssetFormat::Webp]
        }

        fn encode_rgb(
            &self,
            _format: AssetFormat,
            _image: &RgbImage,
        ) -> Result<Vec<u8>, PipelineError> {
            unreachable!("sprite test must not use RGB encoding")
        }

        fn encode_rgba(
            &self,
            _format: AssetFormat,
            _image: &RgbaImage,
        ) -> Result<Vec<u8>, PipelineError> {
            Ok(vec![0; DEFAULT_SPRITE_MAX_VARIANT_BYTES + 1])
        }
    }

    #[test]
    fn sprite_profile_rejects_upscaling_opaque_sources_lossless_mode_and_oversized_renditions() {
        let temp = tempfile::tempdir().unwrap();
        let narrow = temp.path().join("narrow.png");
        write_png(&narrow, 639, 4, Some(128));
        assert!(matches!(
            build_asset_with(
                &sprite_request(narrow, temp.path().join("narrow-out"), SpritePose::Idle),
                &FakeEncoders
            ),
            Err(PipelineError::SourceTooNarrowForProfile {
                kind: AssetKind::EntitySprite,
                actual: 639,
                minimum: 640
            })
        ));

        let opaque = temp.path().join("opaque.png");
        write_png(&opaque, 640, 4, None);
        assert!(matches!(
            build_asset_with(
                &sprite_request(opaque, temp.path().join("opaque-out"), SpritePose::Idle),
                &FakeEncoders
            ),
            Err(PipelineError::SpriteRequiresTransparency)
        ));

        let transparent = temp.path().join("transparent.png");
        write_png(&transparent, 640, 4, Some(128));
        let mut lossless = sprite_request(
            transparent.clone(),
            temp.path().join("lossless-out"),
            SpritePose::Idle,
        );
        lossless.lossless = true;
        assert!(matches!(
            build_asset_with(&lossless, &FakeEncoders),
            Err(PipelineError::LosslessSpriteProfile)
        ));

        let oversized_output = temp.path().join("oversized-out");
        let oversized = sprite_request(transparent, oversized_output.clone(), SpritePose::Attack);
        assert!(matches!(
            build_asset_with(&oversized, &OversizedSpriteEncoders),
            Err(PipelineError::VariantTooLarge {
                format: AssetFormat::Webp,
                width: 320,
                maximum: DEFAULT_SPRITE_MAX_VARIANT_BYTES,
                ..
            })
        ));
        assert!(!oversized_output.exists());
    }

    #[test]
    fn pose_metadata_cannot_be_used_for_path_injection_and_animated_sources_are_rejected() {
        assert!(serde_json::from_str::<SpritePose>(r#""../../idle""#).is_err());

        let mut animated_webp = b"RIFF\x0c\x00\x00\x00WEBPANIM".to_vec();
        animated_webp.extend_from_slice(&0_u32.to_le_bytes());
        assert!(is_animated_source(&animated_webp, ImageFormat::WebP));

        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("animated.png");
        write_png(&source, 640, 4, Some(128));
        let mut bytes = fs::read(&source).unwrap();
        let mut animation_chunk = Vec::new();
        animation_chunk.extend_from_slice(&8_u32.to_be_bytes());
        animation_chunk.extend_from_slice(b"acTL");
        animation_chunk.extend_from_slice(&1_u32.to_be_bytes());
        animation_chunk.extend_from_slice(&0_u32.to_be_bytes());
        animation_chunk.extend_from_slice(&0_u32.to_be_bytes());
        bytes.splice(8..8, animation_chunk);
        fs::write(&source, bytes).unwrap();

        assert!(matches!(
            build_asset_with(
                &sprite_request(source, temp.path().join("animated-out"), SpritePose::Attack),
                &FakeEncoders
            ),
            Err(PipelineError::UnsupportedAnimation)
        ));
    }

    #[cfg(not(feature = "webp-encoding"))]
    #[test]
    fn sprite_profile_fails_closed_when_the_required_webp_alpha_encoder_is_unavailable() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("sprite.png");
        write_png(&source, 640, 4, Some(128));
        assert!(matches!(
            build_asset(&sprite_request(
                source,
                temp.path().join("published"),
                SpritePose::Idle
            )),
            Err(PipelineError::MissingRequiredEncoder(AssetFormat::Webp))
        ));
    }

    #[cfg(feature = "webp-encoding")]
    #[test]
    fn reviewed_quarantine_can_publish_a_static_alpha_sprite_without_forcing_png_only() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("incoming.png");
        write_png(&source, 640, 4, Some(128));
        let staged = quarantine_source(&QuarantineRequest {
            source,
            quarantine_dir: temp.path().join("quarantine"),
            limits: BuildLimits::default(),
        })
        .unwrap();
        review_quarantine(
            &staged.record_path,
            "Tiamud reviewer",
            "2026-07-15T00:00:00Z",
            BuildLimits::default().max_source_bytes,
        )
        .unwrap();

        let result = build_quarantined(
            &staged.record_path,
            sprite_request(
                PathBuf::new(),
                temp.path().join("published"),
                SpritePose::Idle,
            ),
        )
        .unwrap();
        assert_eq!(
            result
                .manifest
                .variants
                .iter()
                .map(|variant| variant.format)
                .collect::<Vec<_>>(),
            vec![
                AssetFormat::Webp,
                AssetFormat::Png,
                AssetFormat::Webp,
                AssetFormat::Png
            ]
        );
    }

    #[test]
    fn default_encoders_emit_the_advertised_formats() {
        let image = RgbImage::from_pixel(4, 4, image::Rgb([24, 48, 72]));
        let encoders = DefaultEncoders;

        let avif = encoders.encode_rgb(AssetFormat::Avif, &image).unwrap();
        assert!(avif.windows(4).any(|bytes| bytes == b"ftyp"));
        assert_eq!(
            avif,
            encoders.encode_rgb(AssetFormat::Avif, &image).unwrap(),
            "the pinned single-threaded AVIF encoder must be byte-deterministic"
        );

        #[cfg(feature = "webp-encoding")]
        {
            let webp = encoders.encode_rgb(AssetFormat::Webp, &image).unwrap();
            assert_eq!(&webp[..4], b"RIFF");
            assert_eq!(&webp[8..12], b"WEBP");
        }

        let jpeg = encoders.encode_rgb(AssetFormat::Jpeg, &image).unwrap();
        assert_eq!(&jpeg[..2], &[0xff, 0xd8]);
    }
}
