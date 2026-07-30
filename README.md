# Textora

Textora is a native desktop editor for plain text and Markdown, built with Rust and [wgpu](https://github.com/gfx-rs/wgpu).

> **Project status:** Textora is under active development.
>
> **Platform support:** The application currently targets **macOS 12 or later**. The reusable workspace crates are designed to remain portable, but the desktop application is not yet cross-platform.

## Features

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

Run Textora without an initial file:

```bash
cargo run -p textora-app
```

Open a file on startup:

```bash
cargo run -p textora-app -- path/to/file.md
```

Initialize the GPU backend without creating a window:

```bash
cargo run -p textora-app -- --headless
```

### Build

Build the application in debug mode:

```bash
cargo build -p textora-app
```

Build a release `.app` bundle for macOS:

```bash
./scripts/bundle-mac.sh
```

The bundle is written to `target/textora.app`.

### Verify

Run the full project verification suite:

```bash
./scripts/verify.sh
```

This checks architecture boundaries, formatting, Clippy, and the workspace test suite.

## File Views

| File pattern | Default view |
| --- | --- |
| `.md`, `.markdown` | Markdown editor |
| `.mmap.md` | Mind map |
| `.txt` | Plain-text editor, with reading view available |
| Other files | Plain-text editor |

Markdown documents can be switched between editing and preview-oriented views. Mind maps use Markdown files with the `.mmap.md` suffix.

## Workspace Layout

| Crate | Responsibility |
| --- | --- |
| [`crates/app`](crates/app) | Desktop application, product behavior, and platform integration |
| [`crates/ui`](crates/ui) | Reusable UI components, layout, themes, and widgets |
| [`crates/core`](crates/core) | Text buffer, document model, editing, and search primitives |
| [`crates/markdown`](crates/markdown) | Markdown editing, preview, reading, and mind-map views |
| [`crates/render`](crates/render) | GPU rendering primitives |
| [`crates/shaping`](crates/shaping) | Text shaping and glyph layout |
| [`crates/appkit-core`](crates/appkit-core) / [`crates/appkit-shell`](crates/appkit-shell) | Shared application-model and desktop-shell infrastructure |

The UI layer accepts pure data inputs from the application layer; it does not depend on application state types. See [`AGENTS.md`](AGENTS.md) for the detailed architecture rules and [`CONTRIBUTING.md`](CONTRIBUTING.md) for development conventions.

## Project Documentation

Plans and project documentation are indexed in [`docs/README.md`](docs/README.md).

## License

Textora is distributed under the [MIT License](LICENSE).
