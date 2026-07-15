//! Offline compiler for bounded, content-addressed atmospheric scene assets.

use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView, ImageEncoder, ImageFormat, ImageReader, RgbImage};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const TARGET_WIDTHS: [u32; 2] = [640, 1280];
const SCHEMA_VERSION: u8 = 1;
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
            max_variant_bytes: 8 * 1024 * 1024,
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
}

impl AssetFormat {
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Avif => "avif",
            Self::Webp => "webp",
            Self::Jpeg => "jpg",
        }
    }

    pub const fn media_type(self) -> &'static str {
        match self {
            Self::Avif => "image/avif",
            Self::Webp => "image/webp",
            Self::Jpeg => "image/jpeg",
        }
    }
}

impl fmt::Display for AssetFormat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Avif => "AVIF",
            Self::Webp => "WebP",
            Self::Jpeg => "JPEG",
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetManifest {
    pub schema: u8,
    pub asset_id: String,
    pub kind: String,
    pub alt: String,
    pub source: SourceDescriptor,
    pub variants: Vec<PublishedVariant>,
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

#[derive(Debug, Error)]
pub enum PipelineError {
    #[error("alternative text must not be empty")]
    EmptyAlt,
    #[error("alternative text exceeds the {maximum}-character limit")]
    AltTooLong { maximum: usize },
    #[error("alternative text must be a single line without control characters")]
    InvalidAltCharacters,
    #[error("source is {actual} bytes, exceeding the {maximum}-byte limit")]
    SourceTooLarge { actual: u64, maximum: u64 },
    #[error("source width {actual} is below the required 1280 pixels; upscaling is disabled")]
    SourceTooNarrow { actual: u32 },
    #[error("source dimensions {width}x{height} exceed the configured limits")]
    InvalidDimensions { width: u32, height: u32 },
    #[error("source contains transparent pixels; atmospheric backdrops must be opaque")]
    SourceHasTransparency,
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
        }
    }
}

pub fn build_asset(request: &BuildRequest) -> Result<BuildResult, PipelineError> {
    build_asset_with(request, &DefaultEncoders)
}

pub fn build_asset_with(
    request: &BuildRequest,
    encoders: &dyn VariantEncoders,
) -> Result<BuildResult, PipelineError> {
    let alt = validate_alt(&request.alt, request.limits.max_alt_chars)?;
    let source_bytes = read_bounded_source(&request.source, request.limits.max_source_bytes)?;
    let source_sha256 = sha256_hex(&source_bytes);
    let source = decode_bounded_source(&source_bytes, request.limits)?;
    let (source_width, source_height) = source.dimensions();

    if source_width < *TARGET_WIDTHS.last().expect("target widths are non-empty") {
        return Err(PipelineError::SourceTooNarrow {
            actual: source_width,
        });
    }
    reject_transparency(&source)?;

    let formats = encoders.formats();
    for required in [AssetFormat::Avif, AssetFormat::Jpeg] {
        if !formats.contains(&required) {
            return Err(PipelineError::MissingRequiredEncoder(required));
        }
    }

    let source_rgb = source.to_rgb8();
    let mut encoded_variants = Vec::with_capacity(TARGET_WIDTHS.len() * formats.len());
    for width in TARGET_WIDTHS {
        let height = scaled_height(source_width, source_height, width);
        let resized = image::imageops::resize(&source_rgb, width, height, FilterType::Lanczos3);

        for format in &formats {
            let bytes = encoders.encode(*format, &resized)?;
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
        kind: "atmosphere".to_owned(),
        alt,
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

fn reject_transparency(source: &DynamicImage) -> Result<(), PipelineError> {
    if source.color().has_alpha()
        && source
            .to_rgba8()
            .pixels()
            .any(|pixel| pixel.0[3] != u8::MAX)
    {
        return Err(PipelineError::SourceHasTransparency);
    }
    Ok(())
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

fn asset_digest(
    alt: &str,
    source_sha256: &str,
    source_width: u32,
    source_height: u32,
    variants: &[EncodedVariant],
) -> String {
    let mut digest = Sha256::new();
    digest.update(b"envy-atmospheric-asset-v1\0");
    digest.update((alt.len() as u64).to_be_bytes());
    digest.update(alt.as_bytes());
    digest.update(source_sha256.as_bytes());
    digest.update(source_width.to_be_bytes());
    digest.update(source_height.to_be_bytes());
    for variant in variants {
        digest.update([match variant.format {
            AssetFormat::Avif => 1,
            AssetFormat::Webp => 2,
            AssetFormat::Jpeg => 3,
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

    #[test]
    fn validates_alt_before_touching_the_source() {
        let request = BuildRequest {
            source: PathBuf::from("does-not-exist.png"),
            alt: " \n ".to_owned(),
            output_dir: PathBuf::from("unused"),
            limits: BuildLimits::default(),
        };
        assert!(matches!(
            build_asset_with(&request, &FakeEncoders),
            Err(PipelineError::EmptyAlt)
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
        };
        assert!(matches!(
            build_asset_with(&request, &FakeEncoders),
            Err(PipelineError::SourceTooLarge { .. })
        ));
    }

    #[test]
    fn rejects_upscaling_and_transparency() {
        let temp = tempfile::tempdir().unwrap();
        let narrow = temp.path().join("narrow.png");
        write_png(&narrow, 1279, 4, None);
        let narrow_request = BuildRequest {
            source: narrow,
            alt: "A narrow backdrop.".to_owned(),
            output_dir: temp.path().join("narrow-out"),
            limits: BuildLimits::default(),
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
        };
        assert!(matches!(
            build_asset_with(&transparent_request, &FakeEncoders),
            Err(PipelineError::SourceHasTransparency)
        ));
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
