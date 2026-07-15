#![no_main]

use envy_import::{TextEncoding, audit, parse_import};
use libfuzzer_sys::fuzz_target;

const MAX_INPUT_BYTES: usize = 1024 * 1024;

fuzz_target!(|input: &[u8]| {
    let Some(&selector) = input.first() else {
        return;
    };
    if input.len() > MAX_INPUT_BYTES {
        return;
    }

    if selector & 1 == 0 {
        let _ = audit(input);
    } else {
        let encoding = if selector & 2 == 0 {
            TextEncoding::Ascii
        } else {
            TextEncoding::Utf8
        };
        let _ = parse_import(input, "fuzz", None, encoding);
    }
});
