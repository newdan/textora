# Contributing

## Before You Commit

Always run the verification script before committing:

```bash
./scripts/verify.sh
```

This checks formatting (`cargo fmt`), lints (`cargo clippy`), and tests (`cargo test`) across the entire workspace.

## Bug Fixes

1. Write a **reproduction test** that demonstrates the bug first.
2. Fix the code until the test passes.
3. Commit the fix.

## Commit Hygiene

- **Logic changes and formatting changes go in separate commits.**
  Do not mix functional edits with whitespace or style reformatting.

## Task Scope

- Keep each atomic task to **at most 3 files**.
  If a change touches more files, break it into smaller tasks.

## UI Widgets

New UI widgets must follow the project's architecture:

- Define a **pure data input struct** in `crates/ui`.
- `crates/app` is responsible for extracting data from `DocumentView` and constructing that struct.
- UI modules must not depend on `DocumentView`, `Workspace`, or app-layer types directly.

## No Production Stubs

Do not submit stubs or placeholder implementations that compile successfully but fail at runtime.
If a feature is not yet implemented, make that clear in the code (e.g. `todo!()` or explicit error handling).

## 更多约定

项目架构约定、模块职责与依赖层次见 [`AGENTS.md`](AGENTS.md)。
