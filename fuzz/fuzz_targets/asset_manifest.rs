#![no_main]

use libfuzzer_sys::fuzz_target;
use tiamud_asset_pipeline::AssetManifest;

const MAX_INPUT_BYTES: usize = 1024 * 1024;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT_BYTES {
        return;
    }

    if let Ok(manifest) = serde_json::from_slice::<AssetManifest>(data) {
        let encoded = serde_json::to_vec(&manifest).expect("a parsed manifest must serialize");
        let decoded: AssetManifest =
            serde_json::from_slice(&encoded).expect("a serialized manifest must parse");
        assert_eq!(manifest, decoded);
    }
});
