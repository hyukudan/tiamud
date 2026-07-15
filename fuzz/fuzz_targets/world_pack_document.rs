#![no_main]

use libfuzzer_sys::fuzz_target;
use world_pack::WorldPackDocument;

const MAX_INPUT_BYTES: usize = 1024 * 1024;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT_BYTES {
        return;
    }

    if let Ok(document) = WorldPackDocument::from_json(data) {
        let _ = document.validate();
        let _ = document.deterministic_artifact();
        let _ = document.run_content_tests();
    }
});
