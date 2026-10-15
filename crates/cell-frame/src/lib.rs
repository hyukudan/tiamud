//! Bounded, transport-neutral cell art with explicit-consent renderers.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MAX_WIDTH: u16 = 160;
pub const MAX_HEIGHT: u16 = 90;
pub const MAX_CELLS: usize = MAX_WIDTH as usize * MAX_HEIGHT as usize;
pub const MAX_PALETTE: usize = 256;
const CELL_PIXEL_WIDTH: u32 = 4;
const CELL_PIXEL_HEIGHT: u32 = 6;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RgbColor {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CellAttributes {
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub underline: bool,
    #[serde(default)]
    pub inverse: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cell {
    pub glyph: char,
    pub foreground: u16,
    pub background: u16,
    #[serde(default)]
    pub attributes: CellAttributes,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CellFrame {
    pub width: u16,
    pub height: u16,
    pub palette: Vec<RgbColor>,
    pub cells: Vec<Cell>,
    /// Complete text alternative; the graphic never owns playable facts.
    pub alt: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnsiProfile {
    Plain,
    Ansi16,
    Ansi256,
    TrueColor,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeProtocol {
    Kitty,
    Sixel,
    Iterm2,
}

#[derive(Debug, Error)]
pub enum CellFrameError {
    #[error("cell frame dimensions must be within {MAX_WIDTH}x{MAX_HEIGHT}")]
    InvalidDimensions,
    #[error("cell frame palette must contain 1 to {MAX_PALETTE} colors")]
    InvalidPalette,
    #[error("cell frame contains {actual} cells but dimensions require {expected}")]
    InvalidCellCount { actual: usize, expected: usize },
    #[error("cell references a color outside the palette")]
    InvalidColorIndex,
    #[error("cell glyph or alternative text contains forbidden controls")]
    InvalidText,
    #[error("native terminal graphics require explicit per-session consent")]
    ConsentRequired,
    #[error("native renderer output exceeded its bounded size")]
    OutputTooLarge,
    #[error("cell frame exceeds the caller's {maximum}-cell limit")]
    CellLimitExceeded { maximum: usize },
    #[error("PNG encoding failed: {0}")]
    Png(String),
}

impl CellFrame {
    pub fn validate(&self) -> Result<(), CellFrameError> {
        let expected = usize::from(self.width).saturating_mul(usize::from(self.height));
        if self.width == 0
            || self.height == 0
            || self.width > MAX_WIDTH
            || self.height > MAX_HEIGHT
            || expected > MAX_CELLS
        {
            return Err(CellFrameError::InvalidDimensions);
        }
        if self.palette.is_empty() || self.palette.len() > MAX_PALETTE {
            return Err(CellFrameError::InvalidPalette);
        }
        if self.cells.len() != expected {
            return Err(CellFrameError::InvalidCellCount {
                actual: self.cells.len(),
                expected,
            });
        }
        if self.alt.trim().is_empty()
            || self.alt.len() > 2048
            || self.alt.chars().any(forbidden_text)
            || self.cells.iter().any(|cell| forbidden_text(cell.glyph))
        {
            return Err(CellFrameError::InvalidText);
        }
        if self.cells.iter().any(|cell| {
            usize::from(cell.foreground) >= self.palette.len()
                || usize::from(cell.background) >= self.palette.len()
        }) {
            return Err(CellFrameError::InvalidColorIndex);
        }
        Ok(())
    }

    /// Apply a transport-specific cell budget in addition to the crate-wide
    /// hard limits. Servers should choose a much smaller value than
    /// [`MAX_CELLS`] for interactive output queues.
    pub fn validate_with_max_cells(&self, maximum: usize) -> Result<(), CellFrameError> {
        self.validate()?;
        if self.cells.len() > maximum {
            return Err(CellFrameError::CellLimitExceeded { maximum });
        }
        Ok(())
    }

    pub fn render_text(&self) -> Result<String, CellFrameError> {
        self.validate()?;
        let mut output = String::with_capacity(self.cells.len() + usize::from(self.height) * 2);
        for row in self.cells.chunks_exact(usize::from(self.width)) {
            output.extend(row.iter().map(|cell| cell.glyph));
            output.push_str("\r\n");
        }
        Ok(output)
    }

    pub fn render_ansi(&self, profile: AnsiProfile) -> Result<String, CellFrameError> {
        if profile == AnsiProfile::Plain {
            return self.render_text();
        }
        self.validate()?;
        let mut output = String::new();
        for row in self.cells.chunks_exact(usize::from(self.width)) {
            for cell in row {
                let foreground = self.palette[usize::from(cell.foreground)];
                let background = self.palette[usize::from(cell.background)];
                output.push_str("\x1b[0");
                if cell.attributes.bold {
                    output.push_str(";1");
                }
                if cell.attributes.underline {
                    output.push_str(";4");
                }
                if cell.attributes.inverse {
                    output.push_str(";7");
                }
                match profile {
                    AnsiProfile::Plain => unreachable!(),
                    AnsiProfile::Ansi16 => {
                        let _ = write!(
                            output,
                            ";{};{}",
                            30 + nearest_ansi16(foreground),
                            40 + nearest_ansi16(background)
                        );
                    }
                    AnsiProfile::Ansi256 => {
                        let _ = write!(
                            output,
                            ";38;5;{};48;5;{}",
                            ansi256(foreground),
                            ansi256(background)
                        );
                    }
                    AnsiProfile::TrueColor => {
                        let _ = write!(
                            output,
                            ";38;2;{};{};{};48;2;{};{};{}",
                            foreground.red,
                            foreground.green,
                            foreground.blue,
                            background.red,
                            background.green,
                            background.blue
                        );
                    }
                }
                output.push('m');
                output.push(cell.glyph);
            }
            output.push_str("\x1b[0m\r\n");
        }
        Ok(output)
    }

    /// Render ANSI with an explicit byte budget. The ordinary renderer remains
    /// useful for offline tooling; network transports should use this method.
    pub fn render_ansi_bounded(
        &self,
        profile: AnsiProfile,
        max_output_bytes: usize,
    ) -> Result<String, CellFrameError> {
        let output = self.render_ansi(profile)?;
        if output.len() > max_output_bytes {
            return Err(CellFrameError::OutputTooLarge);
        }
        Ok(output)
    }

    /// Render a native terminal payload only after an explicit session opt-in.
    /// The raster is deliberately decorative; callers must send `alt` as text.
    pub fn render_native(
        &self,
        protocol: NativeProtocol,
        consent: bool,
        max_output_bytes: usize,
    ) -> Result<Vec<u8>, CellFrameError> {
        self.validate()?;
        if !consent {
            return Err(CellFrameError::ConsentRequired);
        }
        let output = match protocol {
            NativeProtocol::Kitty => {
                let png = self.background_png()?;
                format!("\x1b_Ga=T,f=100,m=0;{}\x1b\\", base64(&png)).into_bytes()
            }
            NativeProtocol::Iterm2 => {
                let png = self.background_png()?;
                format!(
                    "\x1b]1337;File=inline=1;width={}px;height={}px;preserveAspectRatio=1:{}\x07",
                    u32::from(self.width) * CELL_PIXEL_WIDTH,
                    u32::from(self.height) * CELL_PIXEL_HEIGHT,
                    base64(&png)
                )
                .into_bytes()
            }
            NativeProtocol::Sixel => self.sixel_background(),
        };
        if output.len() > max_output_bytes {
            return Err(CellFrameError::OutputTooLarge);
        }
        Ok(output)
    }

    fn background_raster(&self) -> BackgroundRaster {
        let width = u32::from(self.width) * CELL_PIXEL_WIDTH;
        let height = u32::from(self.height) * CELL_PIXEL_HEIGHT;
        // Bounded by MAX_WIDTH x MAX_HEIGHT cells of 4x6 pixels after validation.
        let mut pixels = vec![0_u8; width as usize * height as usize * 3];
        for (index, cell) in self.cells.iter().enumerate() {
            let color = self.palette[usize::from(cell.background)];
            let cell_x = (index % usize::from(self.width)) as u32 * CELL_PIXEL_WIDTH;
            let cell_y = (index / usize::from(self.width)) as u32 * CELL_PIXEL_HEIGHT;
            for y in cell_y..cell_y + CELL_PIXEL_HEIGHT {
                for x in cell_x..cell_x + CELL_PIXEL_WIDTH {
                    let offset = (y as usize * width as usize + x as usize) * 3;
                    pixels[offset..offset + 3].copy_from_slice(&[
                        color.red,
                        color.green,
                        color.blue,
                    ]);
                }
            }
        }
        BackgroundRaster {
            width,
            height,
            pixels,
        }
    }

    /// Encodes the decorative background as an 8-bit RGB PNG with the same
    /// fast-compression, adaptive-filter settings the general-purpose `image`
    /// crate used, without linking its decoders into network servers.
    fn background_png(&self) -> Result<Vec<u8>, CellFrameError> {
        let raster = self.background_raster();
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, raster.width, raster.height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        encoder.set_filter(png::Filter::Adaptive);
        let mut writer = encoder
            .write_header()
            .map_err(|error| CellFrameError::Png(error.to_string()))?;
        writer
            .write_image_data(&raster.pixels)
            .map_err(|error| CellFrameError::Png(error.to_string()))?;
        writer
            .finish()
            .map_err(|error| CellFrameError::Png(error.to_string()))?;
        Ok(bytes)
    }

    fn sixel_background(&self) -> Vec<u8> {
        let image = self.background_raster();
        let mut output = b"\x1bPq".to_vec();
        for (index, color) in self.palette.iter().enumerate() {
            let _ = write!(
                StringWriter(&mut output),
                "#{};2;{};{};{}",
                index,
                percent(color.red),
                percent(color.green),
                percent(color.blue)
            );
        }
        for band_y in (0..image.height).step_by(6) {
            for (palette_index, color) in self.palette.iter().enumerate() {
                let _ = write!(StringWriter(&mut output), "#{palette_index}");
                for x in 0..image.width {
                    let mut bits = 0_u8;
                    for bit in 0..6 {
                        let y = band_y + bit;
                        if y < image.height
                            && image.pixel(x, y) == [color.red, color.green, color.blue]
                        {
                            bits |= 1 << bit;
                        }
                    }
                    output.push(63 + bits);
                }
                output.push(b'$');
            }
            output.push(b'-');
        }
        output.extend_from_slice(b"\x1b\\");
        output
    }
}

/// Row-major 8-bit RGB pixels for the decorative cell background.
struct BackgroundRaster {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl BackgroundRaster {
    fn pixel(&self, x: u32, y: u32) -> [u8; 3] {
        let offset = (y as usize * self.width as usize + x as usize) * 3;
        [
            self.pixels[offset],
            self.pixels[offset + 1],
            self.pixels[offset + 2],
        ]
    }
}

struct StringWriter<'a>(&'a mut Vec<u8>);

impl std::fmt::Write for StringWriter<'_> {
    fn write_str(&mut self, value: &str) -> std::fmt::Result {
        self.0.extend_from_slice(value.as_bytes());
        Ok(())
    }
}

fn forbidden_text(character: char) -> bool {
    character == '\u{1b}' || (character.is_control() && character != '\t')
}

fn percent(value: u8) -> u16 {
    (u16::from(value) * 100 + 127) / 255
}

fn nearest_ansi16(color: RgbColor) -> u8 {
    let bright = u16::from(color.red) + u16::from(color.green) + u16::from(color.blue) > 510;
    let mut index = 0;
    if color.red > 96 {
        index |= 1;
    }
    if color.green > 96 {
        index |= 2;
    }
    if color.blue > 96 {
        index |= 4;
    }
    if bright && index == 0 { 7 } else { index }
}

fn ansi256(color: RgbColor) -> u8 {
    let cube = |value: u8| -> u8 { ((u16::from(value) * 5 + 127) / 255) as u8 };
    16 + 36 * cube(color.red) + 6 * cube(color.green) + cube(color.blue)
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let a = chunk[0];
        let b = *chunk.get(1).unwrap_or(&0);
        let c = *chunk.get(2).unwrap_or(&0);
        output.push(ALPHABET[usize::from(a >> 2)] as char);
        output.push(ALPHABET[usize::from((a & 0x03) << 4 | b >> 4)] as char);
        output.push(if chunk.len() > 1 {
            ALPHABET[usize::from((b & 0x0f) << 2 | c >> 6)] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            ALPHABET[usize::from(c & 0x3f)] as char
        } else {
            '='
        });
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> CellFrame {
        CellFrame {
            width: 2,
            height: 1,
            palette: vec![
                RgbColor {
                    red: 0,
                    green: 0,
                    blue: 0,
                },
                RgbColor {
                    red: 255,
                    green: 160,
                    blue: 32,
                },
            ],
            cells: vec![
                Cell {
                    glyph: 'T',
                    foreground: 1,
                    background: 0,
                    attributes: CellAttributes {
                        bold: true,
                        ..CellAttributes::default()
                    },
                },
                Cell {
                    glyph: 'M',
                    foreground: 1,
                    background: 0,
                    attributes: CellAttributes::default(),
                },
            ],
            alt: "The letters T and M in amber.".to_owned(),
        }
    }

    #[test]
    fn validates_bounds_and_plain_text_is_complete() {
        assert_eq!(frame().render_text().unwrap(), "TM\r\n");
        let mut invalid = frame();
        invalid.cells.pop();
        assert!(matches!(
            invalid.validate(),
            Err(CellFrameError::InvalidCellCount { .. })
        ));
    }

    #[test]
    fn ansi_profiles_are_explicit_and_controls_cannot_be_injected() {
        assert_eq!(frame().render_ansi(AnsiProfile::Plain).unwrap(), "TM\r\n");
        assert!(
            frame()
                .render_ansi(AnsiProfile::Ansi16)
                .unwrap()
                .contains(";33;40m")
        );
        assert!(
            frame()
                .render_ansi(AnsiProfile::Ansi256)
                .unwrap()
                .contains(";38;5;")
        );
        assert!(
            frame()
                .render_ansi(AnsiProfile::TrueColor)
                .unwrap()
                .contains("38;2;255;160;32")
        );
        let mut invalid = frame();
        invalid.cells[0].glyph = '\u{1b}';
        assert!(matches!(
            invalid.validate(),
            Err(CellFrameError::InvalidText)
        ));
    }

    #[test]
    fn kitty_sixel_and_iterm_require_consent_and_are_bounded() {
        for (protocol, prefix, suffix) in [
            (
                NativeProtocol::Kitty,
                b"\x1b_G".as_slice(),
                b"\x1b\\".as_slice(),
            ),
            (
                NativeProtocol::Sixel,
                b"\x1bPq".as_slice(),
                b"\x1b\\".as_slice(),
            ),
            (
                NativeProtocol::Iterm2,
                b"\x1b]1337".as_slice(),
                b"\x07".as_slice(),
            ),
        ] {
            assert!(matches!(
                frame().render_native(protocol, false, 1_000_000),
                Err(CellFrameError::ConsentRequired)
            ));
            let rendered = frame().render_native(protocol, true, 1_000_000).unwrap();
            assert!(rendered.starts_with(prefix));
            assert!(rendered.ends_with(suffix));
            assert!(matches!(
                frame().render_native(protocol, true, 1),
                Err(CellFrameError::OutputTooLarge)
            ));
        }
    }

    #[test]
    fn caller_specific_cell_and_ansi_byte_limits_are_enforced() {
        assert!(matches!(
            frame().validate_with_max_cells(1),
            Err(CellFrameError::CellLimitExceeded { maximum: 1 })
        ));
        frame().validate_with_max_cells(2).unwrap();
        assert!(matches!(
            frame().render_ansi_bounded(AnsiProfile::Ansi16, 1),
            Err(CellFrameError::OutputTooLarge)
        ));
        let output = frame()
            .render_ansi_bounded(AnsiProfile::Ansi16, 1024)
            .unwrap();
        assert!(output.contains('T') && output.contains('M'));
    }

    #[test]
    fn base64_vectors_are_rfc_4648_compatible() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
    }

    #[test]
    fn native_raster_is_a_png_of_cell_backgrounds() {
        let mut frame = frame();
        frame.cells[1].background = 1;
        let kitty = frame
            .render_native(NativeProtocol::Kitty, true, 1_000_000)
            .unwrap();
        // Base64 of the eight-byte PNG signature plus the IHDR length prefix.
        assert!(kitty.starts_with(b"\x1b_Ga=T,f=100,m=0;iVBORw0KGgo"));
        let raster = frame.background_raster();
        assert_eq!((raster.width, raster.height), (8, 6));
        assert_eq!(raster.pixel(3, 5), [0, 0, 0]);
        assert_eq!(raster.pixel(4, 0), [255, 160, 32]);
        assert_eq!(raster.pixel(7, 5), [255, 160, 32]);
    }
}
