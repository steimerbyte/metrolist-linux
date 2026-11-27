//! Generates `$OUT_DIR/modules.rs` from `crates/*/module.meta`.

#[path = "src/gen.rs"]
mod gen;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    // A new workspace member (and so a new module.meta) changes the root manifest.
    println!("cargo:rerun-if-changed=../../Cargo.toml");

    let manifest_dir = match env::var("CARGO_MANIFEST_DIR") {
        Ok(dir) => dir,
        Err(e) => panic!("CARGO_MANIFEST_DIR is not set: {e}"),
    };
    let crates_dir = Path::new(&manifest_dir)
        .join("..")
        .join("..")
        .join("crates");

    let mut paths: Vec<PathBuf> = Vec::new();
    let entries = match fs::read_dir(&crates_dir) {
        Ok(entries) => entries,
        Err(e) => panic!("cannot read {}: {e}", crates_dir.display()),
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => panic!("cannot read {}: {e}", crates_dir.display()),
        };
        let path = entry.path().join("module.meta");
        if path.is_file() {
            paths.push(path);
        }
    }
    paths.sort();

    let mut metas = Vec::with_capacity(paths.len());
    for path in &paths {
        println!("cargo:rerun-if-changed={}", path.display());
        let name = match path.parent().and_then(|p| p.file_name()) {
            Some(name) => name.to_string_lossy().into_owned(),
            None => panic!("cannot derive module name from {}", path.display()),
        };
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) => panic!("cannot read {}: {e}", path.display()),
        };
        match gen::parse_meta(&name, &text) {
            Ok(meta) => metas.push(meta),
            Err(reason) => panic!("invalid {}: {reason}", path.display()),
        }
    }

    let rendered = match gen::render(&metas) {
        Ok(rendered) => rendered,
        Err(reason) => panic!("cannot render module registry: {reason}"),
    };
    let out_dir = match env::var("OUT_DIR") {
        Ok(dir) => dir,
        Err(e) => panic!("OUT_DIR is not set: {e}"),
    };
    let out_path = Path::new(&out_dir).join("modules.rs");
    let current = fs::read_to_string(&out_path).ok();
    // Keep mtime stable when nothing changed; cargo would rebuild the crate otherwise.
    if current.as_deref() != Some(rendered.as_str()) {
        if let Err(e) = fs::write(&out_path, &rendered) {
            panic!("cannot write {}: {e}", out_path.display());
        }
    }
}
