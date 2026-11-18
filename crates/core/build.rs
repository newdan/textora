use std::env;
use std::fs;
use std::path::Path;

use lsh::compiler::{Generator, builtin_definitions_path};
use stdext::arena::scratch_arena;

fn main() {
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
