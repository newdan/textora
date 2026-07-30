use std::env;
use std::fs;
use std::path::Path;

use lsh::compiler::{Generator, builtin_definitions_path};
use stdext::arena::scratch_arena;

fn main() {
    // ── ICU defaults ──────────────────────────────────────────────
    set_if_unset("EDIT_CFG_ICUUC_SONAME", "libicucore.dylib");
    set_if_unset("EDIT_CFG_ICUI18N_SONAME", "libicucore.dylib");
    set_if_unset("EDIT_CFG_ICU_EXPORT_PREFIX", "");
    set_if_unset("EDIT_CFG_ICU_EXPORT_SUFFIX", "");

    println!("cargo::rustc-check-cfg=cfg(edit_icu_renaming_auto_detect)");

    // ── LSH definitions ──────────────────────────────────────────
    let arena = scratch_arena(None);
    let mut generator = Generator::new(&arena);

    let defs_path = builtin_definitions_path();
    println!("cargo:rerun-if-changed={}", defs_path.display());

    if let Err(e) = generator.read_directory(defs_path) {
        panic!("failed to compile LSH definitions from {}: {e}", defs_path.display(),);
    }

    let rust_code = generator.generate_rust().unwrap_or_else(|e| {
        panic!("failed to generate Rust code for LSH definitions: {e}");
    });

    let out_dir = env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("lsh_definitions.rs");
    fs::write(&dest_path, rust_code).unwrap_or_else(|e| {
        panic!("failed to write lsh_definitions.rs: {e}");
    });
}

fn set_if_unset(key: &str, fallback: &str) {
    if std::env::var(key).unwrap_or_default().is_empty() {
        println!("cargo::rerun-if-env-changed={key}");
        println!("cargo::rustc-env={key}={fallback}");
    }
}
