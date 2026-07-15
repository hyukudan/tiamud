#![no_main]

use libfuzzer_sys::fuzz_target;
use tiamud_core::parse_action;

const MAX_INPUT_BYTES: usize = 64 * 1024;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT_BYTES {
        return;
    }

    let Ok(input) = std::str::from_utf8(data) else {
        return;
    };

    let parsed = parse_action(input);

    // The command language deliberately ignores surrounding whitespace. This
    // invariant exercises both successful commands and every error path while
    // guarding against later changes that accidentally preserve untrusted
    // padding in targets or diagnostic values.
    assert_eq!(parsed, parse_action(input.trim()));
});
