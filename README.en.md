# Textora

[简体中文](README.md)

This repository contains two native desktop apps built with Rust and [wgpu](https://github.com/gfx-rs/wgpu): **Textora**, a plain-text and Markdown editor, and **Notora**, a notes app with a local library. They share an editor kernel and a desktop host.

> **Project status:** The project is under active development.
>
> **Platform support:** Both desktop apps target **macOS 12 or later** for running and packaging.

New to the repository? The [onboarding guide (Chinese)](docs/onboarding.md) traces a click, an edit, and a save through the code.

## Textora Features

- Plain-text editing with tabs, sidebar navigation, word wrapping, line numbers, and status information.
- Markdown editing with source, WYSIWYG, and preview-oriented views.
- Automatic view routing for Markdown files, `.mmap.md` mind maps, and `.txt` reading mode.
- Search and replace with Unicode-aware text handling and regular-expression support in the text buffer.
- Syntax highlighting, light/dark themes, configurable typography, and user theme loading.
- Workspace persistence, recent files, dirty-document recovery, and external file-change monitoring.
- GPU-accelerated rendering and text shaping through `wgpu` and `cosmic-text`.

## Getting Started

### Prerequisites

- macOS 12 or later
- Rust toolchain specified by [`rust-toolchain.toml`](rust-toolchain.toml), currently **1.93.0**

### Run

Run these commands from the repository root. `textora-app` is the default Cargo workspace member; select `notora-app` explicitly to start Notora.

```bash
# Textora
cargo run -p textora-app --bin textora
cargo run -p textora-app --bin textora -- path/to/file.md

# Notora (a path argument opens an external file)
cargo run -p notora-app --bin notora
cargo run -p notora-app --bin notora -- path/to/file.md
```

The startup entry points are [`crates/app/src/main.rs`](crates/app/src/main.rs) and [`crates/notora-app/src/main.rs`](crates/notora-app/src/main.rs).

Initialize Textora's GPU backend without creating a window:

```bash
cargo run -p textora-app -- --headless
```

### Build

Build both apps in debug mode:

```bash
cargo build -p textora-app
cargo build -p notora-app
```

Build the smaller executable with basic Markdown and mind maps, without LaTeX, Mermaid diagrams, or HTML rich paste:

```bash
cargo lite
```

The binary is written to `target/lite/textora`. Math and HTML tags remain visible as source text, while `mermaid` fences render as ordinary code blocks.

Build macOS release bundles:

```bash
./scripts/bundle-mac.sh
./scripts/bundle-notora-mac.sh
```

The default app bundle paths are `target/textora.app` and `target/notora.app`.

### Verify

Run the repository's standard verification script:

```bash
./scripts/verify.sh
```

[`scripts/verify.sh`](scripts/verify.sh) covers:

| Check | Scope |
| --- | --- |
| Architecture | Runs [`scripts/check_architecture.sh`](scripts/check_architecture.sh) unless `TEXTORA_ARCHITECTURE_MIGRATION` is nonempty |
| Formatting and Clippy | Workspace-wide format check; `cargo clippy --workspace --all-targets -- -D warnings` |
| Default tests | Runs `notora-app` separately with one test thread; tests the remaining workspace packages except `textora-sync` |

Clippy still checks `textora-sync`. Run its tests separately with `cargo test -p textora-sync` (some tests use local loopback ports). The default test runs do not execute `#[ignore]` cases or drive the desktop UI. See [`scripts/verify-editing.sh`](scripts/verify-editing.sh) for the focused editing checks.

## Textora File Views

| File pattern | Default view |
| --- | --- |
| `.md`, `.markdown` | Markdown editor |
| `.mmap.md` | Mind map |
| `.txt` | Plain-text editor, with reading view available |
| Other files | Plain-text editor |

Markdown documents can be switched between editing and preview-oriented views. Mind maps use Markdown files with the `.mmap.md` suffix.

## Workspace Layout

| Directory | Cargo package | Responsibility |
| --- | --- | --- |
| [`crates/app`](crates/app) | `textora-app` | Textora product layer: windows, command routing, file workspaces, and sync integration |
| [`crates/notora-app`](crates/notora-app) | `notora-app` | Notora product layer: library UI, note actions, autosave, and editor integration |
| [`crates/notora-core`](crates/notora-core) | `notora-core` | Notes domain: library directories, SQLite index, note commands, and file monitoring |
| [`crates/appkit-core`](crates/appkit-core) | `textora-appkit-core` | Headless `DocumentModel`, tab workspace model, session persistence, and file safety |
| [`crates/appkit-shell`](crates/appkit-shell) | `textora-appkit-shell` | Shared desktop host: `EditorRuntime`, tab runtime, input and painting pipelines, and save jobs |
| [`crates/core`](crates/core) | `textora-core` | Text buffer, undo history, Unicode navigation, search, highlighting, and file I/O primitives |
| [`crates/ui`](crates/ui) | `textora-ui` | Reusable widgets, themes, viewport geometry, and the editor view plugin protocol |
| [`crates/markdown`](crates/markdown) | `textora-markdown` | Markdown parsing, source-to-visual projection, edit policy, preview, and reading |
| [`crates/mindmap`](crates/mindmap) | `textora-mindmap` | MMF parsing and editing, mind-map layout, and interactive view |
| [`crates/render`](crates/render) | `textora-render` | GPU drawing primitives and glyph atlases |
| [`crates/shaping`](crates/shaping) | `textora-shaping` | Font selection, text shaping, glyph rasterization, and caches |
| [`crates/sync`](crates/sync) | `textora-sync` | Textora's Syncthing client, sync commands, and state |
| [`crates/encryption`](crates/encryption) | `textora-encryption` | Encrypted Markdown format, encryption and decryption, and unlock sessions |
| [`crates/lsh`](crates/lsh) | `lsh` | Syntax highlighting definitions, compiler, and runtime |
| [`crates/stdext`](crates/stdext) | `stdext` | Low-level utilities for allocation, containers, Unicode, and system primitives |

The UI layer accepts pure data inputs from the application layer; it does not depend on application state types. See [`AGENTS.md`](AGENTS.md) for the detailed architecture rules and [`CONTRIBUTING.md`](CONTRIBUTING.md) for development conventions.

## Project Documentation

- [Onboarding guide (Chinese)](docs/onboarding.md): entry points and tests for edit commits, save completion, and click mapping.
- [Plans and documentation index](docs/README.md): historical remediation plans and design background.

## License

Textora is distributed under the [MIT License](LICENSE).
