//! Byte-exact golden tests for every CellFrame output mode.
//!
//! Regenerate after an intentional encoder change with
//! `TIAMUD_UPDATE_GOLDEN=1 cargo test --locked -p tiamud-cell-frame --test golden`
//! and review the diff of `tests/golden/`.

use std::path::PathBuf;

use tiamud_cell_frame::{
    AnsiProfile, Cell, CellAttributes, CellFrame, CellFrameError, MAX_HEIGHT, MAX_WIDTH,
    NativeProtocol, RgbColor,
};

const BUDGET: usize = 64 * 1024;

fn rgb(red: u8, green: u8, blue: u8) -> RgbColor {
    RgbColor { red, green, blue }
}

/// 3x2 frame: asymmetric, distinct backgrounds, grey and chromatic colours.
fn fixture() -> CellFrame {
    let palette = vec![
        rgb(0, 0, 0),
        rgb(255, 160, 32),
        rgb(128, 128, 128),
        rgb(10, 90, 200),
    ];
    let spec = [
        ('A', 1, 0, true, false, false),
        ('b', 2, 3, false, true, false),
        ('C', 3, 1, false, false, true),
        ('d', 0, 2, true, true, true),
        ('E', 1, 3, false, false, false),
        ('f', 2, 0, false, false, false),
    ];
    let cells = spec
        .iter()
        .map(
            |&(glyph, foreground, background, bold, underline, inverse)| Cell {
                glyph,
                foreground,
                background,
                attributes: CellAttributes {
                    bold,
                    underline,
                    inverse,
                },
            },
        )
        .collect();
    CellFrame {
        width: 3,
        height: 2,
        palette,
        cells,
        alt: "Golden fixture.".to_owned(),
    }
}

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name)
}

fn assert_golden(name: &str, actual: &[u8]) {
    let path = golden_path(name);
    if std::env::var_os("TIAMUD_UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read(&path).unwrap_or_else(|error| {
        panic!("missing golden {path:?} ({error}); run with TIAMUD_UPDATE_GOLDEN=1")
    });
    assert!(
        expected == actual,
        "golden {name} differs ({} expected bytes, {} actual). If the change is intentional, \
         regenerate with TIAMUD_UPDATE_GOLDEN=1 and review the diff. For PNG modes the pixel \
         test (`*_pixels_*`) tells whether only the encoding changed.\nexpected: {:?}\nactual:   {:?}",
        expected.len(),
        actual.len(),
        String::from_utf8_lossy(&expected),
        String::from_utf8_lossy(actual),
    );
}

fn decode_base64(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut accumulator = 0_u32;
    let mut bits = 0;
    for byte in text.bytes().filter(|byte| *byte != b'=') {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            other => panic!("invalid base64 byte {other}"),
        };
        accumulator = (accumulator << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((accumulator >> bits) as u8);
            accumulator &= (1 << bits) - 1;
        }
    }
    out
}

fn decode_png(png_bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    let mut reader = png::Decoder::new(std::io::Cursor::new(png_bytes))
        .read_info()
        .unwrap();
    let mut buffer = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buffer).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgb);
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    buffer.truncate(info.buffer_size());
    (info.width, info.height, buffer)
}

/// Expected raster derived independently from the frame: each cell is a
/// 4x6 block of its background colour.
fn expected_pixels(frame: &CellFrame) -> (u32, u32, Vec<u8>) {
    let (cw, ch) = (4_usize, 6_usize);
    let width = usize::from(frame.width) * cw;
    let height = usize::from(frame.height) * ch;
    let mut pixels = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let cell = &frame.cells[(y / ch) * usize::from(frame.width) + x / cw];
            let color = frame.palette[usize::from(cell.background)];
            pixels.extend([color.red, color.green, color.blue]);
        }
    }
    (width as u32, height as u32, pixels)
}

fn assert_pixels_match(png_bytes: &[u8], frame: &CellFrame, what: &str) {
    let (width, height, pixels) = decode_png(png_bytes);
    let (ew, eh, epixels) = expected_pixels(frame);
    assert_eq!((width, height), (ew, eh), "{what}: PNG dimensions changed");
    assert!(
        pixels == epixels,
        "{what}: decoded pixels differ from the cell backgrounds"
    );
}

#[test]
fn ansi_modes_match_goldens() {
    let frame = fixture();
    for (profile, name) in [
        (AnsiProfile::Plain, "ansi-plain.bin"),
        (AnsiProfile::Ansi16, "ansi-16.bin"),
        (AnsiProfile::Ansi256, "ansi-256.bin"),
        (AnsiProfile::TrueColor, "ansi-truecolor.bin"),
    ] {
        let output = frame.render_ansi_bounded(profile, BUDGET).unwrap();
        assert_golden(name, output.as_bytes());
    }
}

#[test]
fn truecolor_and_256_colour_fallback_are_distinct_and_exact() {
    let frame = fixture();
    let truecolor = frame.render_ansi(AnsiProfile::TrueColor).unwrap();
    let ansi256 = frame.render_ansi(AnsiProfile::Ansi256).unwrap();
    let ansi16 = frame.render_ansi(AnsiProfile::Ansi16).unwrap();
    assert!(truecolor.contains(";38;2;255;160;32;48;2;0;0;0m"));
    assert!(!truecolor.contains(";38;5;"));
    assert!(ansi256.contains(";38;5;") && !ansi256.contains(";38;2;"));
    assert!(!ansi16.contains(";38;") && !ansi16.contains(";48;"));
    for output in [&truecolor, &ansi256, &ansi16] {
        for glyph in "AbCdEf".chars() {
            assert!(output.contains(glyph));
        }
        assert!(output.ends_with("\x1b[0m\r\n"));
    }
}

#[test]
fn plain_profile_emits_no_escape_bytes() {
    let output = fixture().render_ansi(AnsiProfile::Plain).unwrap();
    assert_eq!(output, "AbC\r\ndEf\r\n");
}

#[test]
fn native_modes_match_goldens() {
    let frame = fixture();
    for (protocol, name) in [
        (NativeProtocol::Kitty, "kitty.bin"),
        (NativeProtocol::Iterm2, "iterm2.bin"),
        (NativeProtocol::Sixel, "sixel.bin"),
    ] {
        let output = frame.render_native(protocol, true, BUDGET).unwrap();
        assert_golden(name, &output);
    }
}

#[test]
fn kitty_pixels_are_the_cell_backgrounds() {
    let frame = fixture();
    let output = frame
        .render_native(NativeProtocol::Kitty, true, BUDGET)
        .unwrap();
    let text = String::from_utf8(output).unwrap();
    let payload = text
        .strip_prefix("\x1b_Ga=T,f=100,m=0;")
        .and_then(|rest| rest.strip_suffix("\x1b\\"))
        .expect("kitty envelope changed");
    assert_pixels_match(&decode_base64(payload), &frame, "kitty");
}

#[test]
fn iterm2_pixels_are_the_cell_backgrounds() {
    let frame = fixture();
    let output = frame
        .render_native(NativeProtocol::Iterm2, true, BUDGET)
        .unwrap();
    let text = String::from_utf8(output).unwrap();
    let rest = text
        .strip_prefix("\x1b]1337;File=inline=1;")
        .expect("iterm2 envelope changed");
    let rest = rest
        .strip_suffix('\x07')
        .expect("iterm2 terminator changed");
    let (header, payload) = rest.split_once(':').unwrap();
    assert_eq!(header, "width=12px;height=12px;preserveAspectRatio=1");
    assert_pixels_match(&decode_base64(payload), &frame, "iterm2");
}

#[test]
fn sixel_decodes_to_the_cell_backgrounds() {
    let frame = fixture();
    let output = frame
        .render_native(NativeProtocol::Sixel, true, BUDGET)
        .unwrap();
    let text = String::from_utf8(output).unwrap();
    let body = text
        .strip_prefix("\x1bPq")
        .unwrap()
        .strip_suffix("\x1b\\")
        .unwrap();
    let (width, height, expected) = expected_pixels(&frame);
    // Minimal decoder for the emitted subset: palette defs, `#n` selects,
    // data bytes, `$` carriage return and `-` next band.
    let mut canvas = vec![None::<usize>; (width * height) as usize];
    let (mut color, mut x, mut band) = (0_usize, 0_u32, 0_u32);
    let mut chars = body.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '#' => {
                let mut number = String::new();
                while let Some(d) = chars.peek().filter(|c| c.is_ascii_digit()) {
                    number.push(*d);
                    chars.next();
                }
                color = number.parse().unwrap();
                if chars.peek() == Some(&';') {
                    while chars
                        .peek()
                        .is_some_and(|c| *c == ';' || c.is_ascii_digit())
                    {
                        chars.next();
                    }
                }
            }
            '$' => x = 0,
            '-' => {
                x = 0;
                band += 1;
            }
            '?'..='~' => {
                let bits = ch as u8 - 63;
                for bit in 0..6 {
                    let y = band * 6 + bit;
                    if bits & (1 << bit) != 0 && y < height {
                        canvas[(y * width + x) as usize] = Some(color);
                    }
                }
                x += 1;
            }
            other => panic!("unexpected sixel char {other:?}"),
        }
    }
    let mut decoded = Vec::new();
    for slot in canvas {
        let c = frame.palette[slot.expect("pixel left unpainted")];
        decoded.extend([c.red, c.green, c.blue]);
    }
    assert!(
        decoded == expected,
        "sixel pixels differ from the cell backgrounds"
    );
}

#[test]
fn native_output_requires_consent() {
    let frame = fixture();
    for protocol in [
        NativeProtocol::Kitty,
        NativeProtocol::Iterm2,
        NativeProtocol::Sixel,
    ] {
        assert!(matches!(
            frame.render_native(protocol, false, BUDGET),
            Err(CellFrameError::ConsentRequired)
        ));
    }
}

#[test]
fn size_bounds_are_enforced_for_every_mode() {
    let blank = |width: u16, height: u16| CellFrame {
        width,
        height,
        palette: vec![rgb(1, 2, 3)],
        cells: vec![
            Cell {
                glyph: ' ',
                foreground: 0,
                background: 0,
                attributes: CellAttributes::default(),
            };
            usize::from(width) * usize::from(height)
        ],
        alt: "bounds".to_owned(),
    };
    let profiles = [
        AnsiProfile::Plain,
        AnsiProfile::Ansi16,
        AnsiProfile::Ansi256,
        AnsiProfile::TrueColor,
    ];
    let protocols = [
        NativeProtocol::Kitty,
        NativeProtocol::Iterm2,
        NativeProtocol::Sixel,
    ];
    let max = blank(MAX_WIDTH, MAX_HEIGHT);
    max.validate().unwrap();
    assert!(max.render_ansi(AnsiProfile::Plain).is_ok());
    let kitty = max
        .render_native(NativeProtocol::Kitty, true, usize::MAX)
        .unwrap();
    assert!(kitty.starts_with(b"\x1b_G"));
    for oversized in [
        blank(MAX_WIDTH + 1, 1),
        blank(1, MAX_HEIGHT + 1),
        blank(0, 1),
    ] {
        assert!(oversized.validate().is_err());
        for profile in profiles {
            assert!(oversized.render_ansi(profile).is_err());
        }
        for protocol in protocols {
            assert!(oversized.render_native(protocol, true, usize::MAX).is_err());
        }
    }
    // Byte budgets are exact: one byte short fails, an exact fit succeeds.
    let frame = fixture();
    let len = frame.render_ansi(AnsiProfile::TrueColor).unwrap().len();
    assert!(
        frame
            .render_ansi_bounded(AnsiProfile::TrueColor, len)
            .is_ok()
    );
    assert!(matches!(
        frame.render_ansi_bounded(AnsiProfile::TrueColor, len - 1),
        Err(CellFrameError::OutputTooLarge)
    ));
    for protocol in protocols {
        let len = frame
            .render_native(protocol, true, usize::MAX)
            .unwrap()
            .len();
        assert!(frame.render_native(protocol, true, len).is_ok());
        assert!(matches!(
            frame.render_native(protocol, true, len - 1),
            Err(CellFrameError::OutputTooLarge)
        ));
    }
}
