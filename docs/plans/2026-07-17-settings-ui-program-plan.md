# Settings UI Program Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a reusable macOS-style settings UI foundation, a modal in-window settings view, and real editor-setting persistence, while leaving sufficient generic controls for a later Syncthing integration.

**Architecture:** The program is split into three independently reviewable phases. Leaf controls emit identity-bearing actions; generic Form containers compose them; the existing `UiShell` overlay path gains an explicit modal policy; `SettingsView` maps pure editor-setting inputs and control actions without depending on app state structures.

**Tech Stack:** Rust 2024, existing custom wgpu UI, winit 0.30, shaping crate, zeroize, and the existing app effect/persistence pipeline.

## Global Constraints

- Product name is `textora`; the Markdown package remains `textora-markdown`.
- `crates/ui` must not depend on app, `DocumentView`, Keychain, worker objects, or Syncthing DTOs.
- The settings surface is an in-window singleton modal overlay; editor content remains visible but receives no mouse, wheel, keyboard, or IME input.
- Visual language follows current macOS System Settings without AppKit or pixel copying.
- Label status semantics are icon plus text; Button emphasis is supplied by background-capable style tokens.
- Switch and Checkbox follow their conventional distinct semantics.
- Masked TextBox and SensitiveText must support a later API-Key flow without exposing plaintext through Debug, logs, or DrawList text; this program does not connect that flow to Syncthing.
- Mutually exclusive Rust state uses enums rather than multiple booleans.
- A task may modify at most three files; split again before implementation if a discovered change exceeds that boundary.
- Every behavior change follows TDD: targeted failing test, minimal implementation, passing test, then commit.
- Every task commit must compile; the final program runs `./scripts/verify.sh`.
- Approved specification: `docs/specs/2026-07-17-settings-ui-foundation-design.md`.

---

## Phase Documents

1. [Phase 1: Leaf Controls and Unified Actions](./2026-07-17-settings-ui-phase1-controls-plan.md)
2. [Phase 2: Form Containers and Modal Overlay](./2026-07-17-settings-ui-phase2-form-overlay-plan.md)
3. [Phase 3: Real Settings View and Persistence](./2026-07-17-settings-ui-phase3-settings-view-plan.md)

## Dependency Gates

- Phase 2 starts only after Phase 1 exports `Label`, `Button`, `TextBox`, `Switch`, `Checkbox`, `ControlAction`, and `WidgetId`.
- Phase 3 starts only after Phase 2 proves modal events do not fall through and FormView provides scrolling and responsive rows.

## Deferred Scope

- Do not add a Syncthing category to the first SettingsView implementation.
- Do not add `textora-sync`, REST calls, Keychain access, connection testing, Device ID/version display, Web UI opening, or disconnect behavior in this program.
- A later Syncthing plan must compose the generic Label, Masked TextBox, Button, InlineGroup, FormRow, and FormSection controls delivered here; it must not add a dedicated Syncthing UI control family.

## Program Verification

- [ ] Run `cargo fmt --all -- --check`.
- [ ] Run `cargo test -p textora-ui`.
- [ ] Run `cargo test -p textora-app --lib`.
- [ ] Run `cargo check -p textora-app`.
- [ ] Run `./scripts/verify.sh`.
- [ ] Expected: every command exits 0; modal-input, persistence-failure, sensitive-text, responsive-form, and settings round-trip tests all pass.
