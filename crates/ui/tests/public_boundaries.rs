use std::fs;
use std::path::{Path, PathBuf};

fn rust_files(root: &Path) -> Vec<PathBuf> {
    fn visit(dir: &Path, out: &mut Vec<PathBuf>) {
        let mut entries: Vec<_> =
            fs::read_dir(dir).unwrap().map(|entry| entry.unwrap().path()).collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                visit(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    visit(root, &mut files);
    files
}

/// Strip line comments (//...) and block comments (/* ... */) from Rust source
/// so that forbidden-pattern checks only match actual code, not documentation.
fn strip_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            // line comment: skip until newline
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
        } else if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            // block comment: skip until */
            i += 2;
            while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i += 2; // skip */
        } else if bytes[i] == b'"' {
            // string literal — keep but skip contents to avoid false hits inside strings
            out.push(bytes[i] as char);
            i += 1;
            while i < len && bytes[i] != b'"' {
                if bytes[i] == b'\\' && i + 1 < len {
                    out.push(bytes[i] as char);
                    i += 1;
                }
                out.push(bytes[i] as char);
                i += 1;
            }
            if i < len {
                out.push(bytes[i] as char);
                i += 1;
            }
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

fn joined_sources_stripped(root: &Path) -> String {
    rust_files(root)
        .into_iter()
        .map(|path| {
            let raw = fs::read_to_string(path).unwrap();
            strip_comments(&raw)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn joined_sources(root: &Path) -> String {
    rust_files(root)
        .into_iter()
        .map(|path| fs::read_to_string(path).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn implementation_modules_are_not_public() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let lib = fs::read_to_string(root.join("src/lib.rs")).unwrap();
    for declaration in
        ["pub mod widgets;", "pub mod theme_file;", "pub mod hex_color;", "pub mod text_renderer;"]
    {
        assert!(!lib.contains(declaration), "public implementation module: {declaration}");
    }
}

#[test]
fn ui_crate_has_no_high_risk_blanket_lint_suppression() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let lib = fs::read_to_string(root.join("src/lib.rs")).unwrap();
    for forbidden in [
        "#![allow(unused_must_use)]",
        "#![allow(dead_code)]",
        "#![allow(unused_mut)]",
        "#![allow(clippy::type_complexity)]",
        "#![allow(clippy::too_many_arguments)]",
    ] {
        assert!(!lib.contains(forbidden), "high-risk crate lint suppression: {forbidden}");
    }
}

#[test]
fn ui_has_no_app_types_or_production_filesystem_access() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let source = joined_sources_stripped(&root.join("src"));
    for forbidden in [
        "DocumentView",
        "Workspace",
        "AppAction",
        "AppCommand",
        "textora_sync",
        "SyncthingClient",
        "Keychain",
    ] {
        assert!(!source.contains(forbidden), "ui depends on app type {forbidden}");
    }
    for forbidden in ["std::fs", "read_dir(", "read_to_string("] {
        assert!(!source.contains(forbidden), "ui production filesystem use: {forbidden}");
    }
}

#[test]
fn ui_has_no_platform_accessibility_implementation_dependency() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let source = joined_sources_stripped(&root.join("src"));
    let manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
    for forbidden in ["accesskit", "NSAccessibility", "UIAutomation", "atk::"] {
        assert!(!source.contains(forbidden), "ui source depends on platform type {forbidden}");
        assert!(!manifest.contains(forbidden), "ui manifest depends on platform type {forbidden}");
    }
}

#[test]
fn theme_registry_does_not_log_or_load_lazily() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let registry = fs::read_to_string(root.join("src/theme_registry.rs")).unwrap();
    let registry_stripped = strip_comments(&registry);
    for forbidden in ["eprintln!", "pending", "load_pending", "std::io"] {
        assert!(!registry_stripped.contains(forbidden), "theme registry leak: {forbidden}");
    }
}

#[test]
fn app_uses_only_semantic_widget_paths() {
    let ui_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let app_source = joined_sources(&ui_root.parent().unwrap().join("app/src"));
    assert!(!app_source.contains("ui::widgets::"));
    assert!(!app_source.contains("use ui::widgets"));
}
