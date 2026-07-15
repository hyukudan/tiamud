use std::{env, path::PathBuf};

fn main() {
    println!("cargo::rustc-check-cfg=cfg(web_dist)");
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let web_dist = manifest_dir.join("../../apps/web/dist");
    println!("cargo::rerun-if-changed={}", web_dist.display());
    if web_dist.join("index.html").is_file() {
        println!("cargo::rustc-cfg=web_dist");
    }
}
