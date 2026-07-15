//! Offline compiler for bounded, content-addressed atmospheric scene assets.

use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView, ImageEncoder, ImageFormat, ImageReader, RgbImage};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const TARGET_WIDTHS: [u32; 2] = [640, 1280];
const SCHEMA_VERSION: u8 = 2;
const JPEG_QUALITY: u8 = 84;
const AVIF_QUALITY: f32 = 64.0;
const AVIF_SPEED: u8 = 8;
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
    /// Publish only deterministic PNG variants, even when the source is opaque.
    pub lossless: bool,
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
    pub kind: String,
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
    #[error("source dimensions {width}x{height} exceed the configured limits")]
    InvalidDimensions { width: u32, height: u32 },
    #[error("unsupported source format; use PNG, JPEG, or WebP")]
    UnsupportedSourceFormat,
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
    fn formats(&self) -> Vec<AssetFormat>;

    fn encode(&self, format: AssetFormat, image: &RgbImage) -> Result<Vec<u8>, PipelineError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultEncoders;

impl VariantEncoders for DefaultEncoders {
    fn formats(&self) -> Vec<AssetFormat> {
        let mut formats = vec![AssetFormat::Avif];
        #[cfg(feature = "webp-encoding")]
        formats.push(AssetFormat::Webp);
        formats.push(AssetFormat::Jpeg);
        formats
    }

    fn encode(&self, format: AssetFormat, image: &RgbImage) -> Result<Vec<u8>, PipelineError> {
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
    request.lossless |= record.has_transparency;
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

    if source_width < *TARGET_WIDTHS.last().expect("target widths are non-empty") {
        return Err(PipelineError::SourceTooNarrow {
            actual: source_width,
        });
    }
    let has_transparency = has_transparency(&source);
    let formats = if request.lossless || has_transparency {
        vec![AssetFormat::Png]
    } else {
        let formats = encoders.formats();
        for required in [AssetFormat::Avif, AssetFormat::Jpeg] {
            if !formats.contains(&required) {
                return Err(PipelineError::MissingRequiredEncoder(required));
            }
        }
        formats
    };

    let source_rgb = source.to_rgb8();
    let source_rgba = (request.lossless || has_transparency).then(|| source.to_rgba8());
    let dominant_color = dominant_color(&source_rgb);
    let mut encoded_variants = Vec::with_capacity(TARGET_WIDTHS.len() * formats.len());
    for width in TARGET_WIDTHS {
        let height = scaled_height(source_width, source_height, width);
        let resized = image::imageops::resize(&source_rgb, width, height, FilterType::Lanczos3);
        let resized_rgba = source_rgba
            .as_ref()
            .map(|image| image::imageops::resize(image, width, height, FilterType::Lanczos3));

        for format in &formats {
            let bytes = if *format == AssetFormat::Png {
                encode_png_rgba(resized_rgba.as_ref().expect("PNG source prepared"))?
            } else {
                encoders.encode(*format, &resized)?
            };
            if bytes.len() > request.limits.max_variant_bytes {
                return Err(PipelineError::VariantTooLarge {
                    format: *format,
                    width,
                    actual: bytes.len(),
                    maximum: request.limits.max_variant_bytes,
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
        source_width,
        source_height,
        &metadata,
        &dominant_color,
        &encoded_variants,
    );
    let asset_id = format!("sha256:{asset_hash}");
    let bundle_dir = request.output_dir.join(&asset_hash);
    let mut variants = Vec::with_capacity(encoded_variants.len());

    fs::create_dir_all(&bundle_dir).map_err(|error| PipelineError::io(&bundle_dir, error))?;
    for variant in &encoded_variants {
        let file_name = format!("scene-{}.{}", variant.width, variant.format.extension());
        let file_path = bundle_dir.join(&file_name);
        write_immutable(&file_path, &variant.bytes)?;
        variants.push(PublishedVariant {
            format: variant.format,
            media_type: variant.format.media_type().to_owned(),
            width: variant.width,
            height: variant.height,
            bytes: variant.bytes.len(),
            sha256: variant.sha256.clone(),
            path: format!("{asset_hash}/{file_name}"),
        });
    }

    let manifest = AssetManifest {
        schema: SCHEMA_VERSION,
        asset_id: asset_id.clone(),
        revision: metadata.revision,
        kind: "atmosphere".to_owned(),
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
    let manifest_sha256 = sha256_hex(&manifest_bytes);
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
    source_width: u32,
    source_height: u32,
    metadata: &AuthoringMetadata,
    dominant_color: &str,
    variants: &[EncodedVariant],
) -> String {
    let mut digest = Sha256::new();
    digest.update(b"tiamud-atmospheric-asset-v2\0");
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

        if path.exists() {
            let existing = fs::read(path).map_err(|error| PipelineError::io(path, error))?;
            if existing != bytes {
                return Err(PipelineError::ImmutableCollision(path.to_owned()));
            }
            return Ok(());
        }
        fs::rename(&temporary, path).map_err(|error| PipelineError::io(path, error))
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
        fn formats(&self) -> Vec<AssetFormat> {
            vec![AssetFormat::Avif, AssetFormat::Webp, AssetFormat::Jpeg]
        }

        fn encode(&self, format: AssetFormat, image: &RgbImage) -> Result<Vec<u8>, PipelineError> {
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

    #[test]
    fn default_encoders_emit_the_advertised_formats() {
        let image = RgbImage::from_pixel(4, 4, image::Rgb([24, 48, 72]));
        let encoders = DefaultEncoders;

        let avif = encoders.encode(AssetFormat::Avif, &image).unwrap();
        assert!(avif.windows(4).any(|bytes| bytes == b"ftyp"));

        #[cfg(feature = "webp-encoding")]
        {
            let webp = encoders.encode(AssetFormat::Webp, &image).unwrap();
            assert_eq!(&webp[..4], b"RIFF");
            assert_eq!(&webp[8..12], b"WEBP");
        }

        let jpeg = encoders.encode(AssetFormat::Jpeg, &image).unwrap();
        assert_eq!(&jpeg[..2], &[0xff, 0xd8]);
    }
}
