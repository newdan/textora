# Edit+

A desktop text and Markdown editor built with Rust and [wgpu](https://github.com/gfx-rs/wgpu).

> **Platform support:** The application currently targets **macOS** only.
> Cross-platform support is not yet available.

## Getting Started

### Prerequisites

- Rust toolchain — version is locked in [`rust-toolchain.toml`](rust-toolchain.toml) (currently **1.93.0**).

### Build

```bash
cargo build -p textora-app
```

### Verify

Run the full baseline check (formatting, lints, tests):

```bash
./scripts/verify.sh
```

## Architecture

See [`AGENTS.md`](AGENTS.md) for the crate layout, dependency hierarchy, and UI module overview.

## Plans

Project plans and phase tracking live in [`docs/README.md`](docs/README.md).

## License

This project is licensed under the [MIT License](LICENSE).
