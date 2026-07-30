# Settings UI Phase 2: Form Containers and Modal Overlay Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add reusable InlineGroup/Form containers and formalize the existing UiShell overlay path so modal content blocks editor input and restores focus.

**Architecture:** Form widgets own child widgets and route local-coordinate events without interpreting ControlAction. Overlay layout/input/dismiss types live in `ui::core`; app UiShell owns the actual overlay entry because it also owns keyboard focus and Dock dispatch.

**Tech Stack:** Existing Widget/DrawList/Rect infrastructure, DrawList clipping, winit events, and the Phase 1 leaf widgets.

## Global Constraints

- Generic containers must not contain settings or Syncthing field names.
- Form containers accept arbitrary Widget children; they propagate child WidgetAction unchanged.
- Modal policy blocks every event category even when the modal child returns `None`.
- Only one interactive overlay is active; tooltip remains a separate non-interactive path.
- Settings modal uses Escape or explicit close; backdrop click does not dismiss it.
- Every layout dimension is a named logical constant scaled once by DPI.
- Each task modifies at most three files and ends with a compiling commit.

---

### Task 1: Define overlay policies and centered layout geometry

**Files:**
- Create: `crates/ui/src/core/overlay.rs`
- Modify: `crates/ui/src/core/mod.rs`
- Modify: `crates/ui/src/core/widget.rs`

**Interfaces:**
- Produces: `OverlayLayout`, `OverlayInputPolicy`, `DismissPolicy`, and `OverlayAction`.

- [ ] **Step 1: Write failing centered-layout tests**

```rust
#[test]
fn centered_layout_respects_preferred_size_and_min_margin() {
    let layout = OverlayLayout::Centered {
        preferred_size: (900.0, 640.0),
        min_margin: 24.0,
        max_width_ratio: 0.92,
        max_height_ratio: 0.90,
    };
    assert_eq!(layout.resolve(Rect::new(0.0, 0.0, 1200.0, 800.0), 1.0), Rect::new(150.0, 80.0, 900.0, 640.0));
    assert_eq!(layout.resolve(Rect::new(0.0, 0.0, 640.0, 480.0), 1.0), Rect::new(24.0, 24.0, 592.0, 432.0));
}
```

- [ ] **Step 2: Run and observe failure**

Run: `cargo test -p textora-ui centered_layout_respects_preferred_size_and_min_margin`

Expected: FAIL because overlay core types do not exist.

- [ ] **Step 3: Implement pure overlay types**

```rust
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OverlayLayout {
    Fixed(Rect),
    Centered {
        preferred_size: (f32, f32),
        min_margin: f32,
        max_width_ratio: f32,
        max_height_ratio: f32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayInputPolicy { Modal, PassThrough }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DismissPolicy { ExplicitOnly, EscapeOrExplicit, EscapeBackdropOrExplicit }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayAction { DismissRequested }
```

Implement `resolve(screen_rect, dpi)` with finite, non-negative dimensions and named fallback ratios. Re-export from `core/mod.rs` and add `WidgetAction::Overlay(OverlayAction)` in `core/widget.rs`.

- [ ] **Step 4: Verify and commit**

Run: `cargo test -p textora-ui overlay`

Expected: PASS.

```bash
git add crates/ui/src/core/overlay.rs crates/ui/src/core/mod.rs crates/ui/src/core/widget.rs
git commit -m "feat(ui): define overlay policies"
```

### Task 2: Add InlineGroup

**Files:**
- Create: `crates/ui/src/widgets/inline_group.rs`
- Modify: `crates/ui/src/widgets/mod.rs`
- Modify: `crates/ui/src/lib.rs`

**Interfaces:**
- Produces: `InlineGroup`, `InlineChild`, `InlineWidth`, and `CrossAlignment`.

- [ ] **Step 1: Write failing layout and event-routing tests**

```rust
#[test]
fn inline_group_assigns_fixed_and_flexible_widths() {
    let mut group = fixture_group(vec![InlineWidth::Fixed(80.0), InlineWidth::Flex(1.0)]);
    layout(&mut group, Rect::new(0.0, 0.0, 300.0, 32.0), 1.0);
    assert_eq!(group.child_rect(0), Rect::new(0.0, 0.0, 80.0, 32.0));
    assert_eq!(group.child_rect(1), Rect::new(88.0, 0.0, 212.0, 32.0));
}

#[test]
fn inline_group_propagates_hit_child_action() {
    let mut group = group_with_button(WidgetId(40));
    assert!(matches!(click_child(&mut group, 10.0, 10.0), Some(WidgetAction::Control(ControlAction::Activated { id: WidgetId(40) }))));
}
```

- [ ] **Step 2: Run and observe failure**

Run: `cargo test -p textora-ui inline_group`

Expected: FAIL because InlineGroup is absent.

- [ ] **Step 3: Implement horizontal composition**

```rust
pub enum InlineWidth { Fixed(f32), Flex(f32), Content(f32) }
pub enum CrossAlignment { Start, Center, End, Stretch }

pub struct InlineChild {
    pub widget: Box<dyn Widget>,
    pub width: InlineWidth,
    rect: Rect,
}

impl InlineChild {
    pub fn fixed(widget: Box<dyn Widget>, width_logical: f32) -> Self;
    pub fn flex(widget: Box<dyn Widget>, weight: f32) -> Self;
    pub fn content(widget: Box<dyn Widget>, measured_width_logical: f32) -> Self;
}

pub struct InlineGroup {
    rect: Rect,
    children: Vec<InlineChild>,
    gap_logical: f32,
    alignment: CrossAlignment,
    focused_id: Option<WidgetId>,
}
```

Lay out fixed/content widths before distributing remaining width to Flex weights. Paint with temporary DrawList offsets and route mouse events only to hit children; route key/IME events to `focused_id`. Override `collect_focusable_ids` by delegating to children and `set_keyboard_focus` by forwarding the selected ID.

- [ ] **Step 4: Verify and commit**

Run: `cargo test -p textora-ui inline_group`

Expected: PASS.

```bash
git add crates/ui/src/widgets/inline_group.rs crates/ui/src/widgets/mod.rs crates/ui/src/lib.rs
git commit -m "feat(ui): add inline widget group"
```

### Task 3: Add responsive FormRow

**Files:**
- Create: `crates/ui/src/widgets/form/row.rs`
- Create: `crates/ui/src/widgets/form/mod.rs`
- Modify: `crates/ui/src/widgets/mod.rs`

**Interfaces:**
- Consumes: Phase 1 Label and arbitrary control Widget.
- Produces: `FormRow`, `FormRowStyle`, and `FormRowLayoutMode`.

- [ ] **Step 1: Write failing wide/narrow layout tests**

```rust
#[test]
fn form_row_switches_from_columns_to_stack_at_threshold() {
    let mut row = fixture_row();
    layout(&mut row, 760.0);
    assert_eq!(row.layout_mode(), FormRowLayoutMode::Columns);
    layout(&mut row, 520.0);
    assert_eq!(row.layout_mode(), FormRowLayoutMode::Stacked);
    assert!(row.control_rect().y > row.label_rect().y);
}
```

- [ ] **Step 2: Run and observe failure**

Run: `cargo test -p textora-ui form_row_switches_from_columns_to_stack_at_threshold`

Expected: FAIL because FormRow is absent.

- [ ] **Step 3: Implement the row**

```rust
pub struct FormRowStyle {
    pub min_height_logical: f32,
    pub label_width_logical: f32,
    pub column_gap_logical: f32,
    pub stack_gap_logical: f32,
    pub responsive_threshold_logical: f32,
    pub padding_logical: [f32; 4],
}

pub struct FormRow {
    rect: Rect,
    label: Label,
    description: Option<Label>,
    control: Box<dyn Widget>,
    style: FormRowStyle,
    layout_mode: FormRowLayoutMode,
}
```

Implement responsive child rectangles, child action propagation, and no business-value interpretation. Export from `form/mod.rs` and register the module in widgets.

- [ ] **Step 4: Verify and commit**

Run: `cargo test -p textora-ui form_row`

Expected: PASS.

```bash
git add crates/ui/src/widgets/form/row.rs crates/ui/src/widgets/form/mod.rs crates/ui/src/widgets/mod.rs
git commit -m "feat(ui): add responsive form row"
```

### Task 4: Add macOS-style FormSection surface

**Files:**
- Create: `crates/ui/src/widgets/form/section.rs`
- Modify: `crates/ui/src/widgets/form/mod.rs`

**Interfaces:**
- Produces: `FormSection`, `FormSectionStyle`, row separators, and section content height.

- [ ] **Step 1: Write failing surface and separator tests**

```rust
#[test]
fn form_section_paints_one_surface_and_internal_separators() {
    let section = laid_out_section(3);
    let draw = paint_for_test(&section);
    assert_eq!(count_surface_fills(&draw), 1);
    assert_eq!(count_separator_strokes(&draw), 2);
}
```

- [ ] **Step 2: Run and observe failure**

Run: `cargo test -p textora-ui form_section_paints_one_surface_and_internal_separators`

Expected: FAIL because FormSection is absent.

- [ ] **Step 3: Implement section composition**

```rust
pub struct FormSectionStyle {
    pub title_gap_logical: f32,
    pub description_gap_logical: f32,
    pub row_gap_logical: f32,
    pub corner_radius_logical: f32,
    pub border_width_logical: f32,
}

pub struct FormSection {
    rect: Rect,
    title: Label,
    description: Option<Label>,
    rows: Vec<FormRow>,
    style: FormSectionStyle,
}
```

Use `Theme::settings_theme()` for section surface, border, and separator. Clip separators to the rounded content bounds and propagate child actions unchanged.

- [ ] **Step 4: Verify and commit**

Run: `cargo test -p textora-ui form_section`

Expected: PASS.

```bash
git add crates/ui/src/widgets/form/section.rs crates/ui/src/widgets/form/mod.rs
git commit -m "feat(ui): add grouped form section"
```

### Task 5: Add scrolling FormView

**Files:**
- Create: `crates/ui/src/widgets/form/view.rs`
- Modify: `crates/ui/src/widgets/form/mod.rs`
- Modify: `crates/ui/src/lib.rs`

**Interfaces:**
- Produces: `FormView`, `FormViewStyle`, `set_sections`, `reset_scroll`, and `focused_ime_cursor_rect`.

- [ ] **Step 1: Write failing clipping and scroll-bound tests**

```rust
#[test]
fn form_view_clips_sections_and_clamps_scroll() {
    let mut view = laid_out_form_view(content_height(900.0), viewport_height(300.0));
    wheel(&mut view, -10_000.0);
    assert_eq!(view.scroll_offset(), 600.0);
    let draw = paint_for_test(&view);
    assert!(matches!(draw.cmds.first(), Some(DrawCmd::PushClip(_))));
    assert!(matches!(draw.cmds.last(), Some(DrawCmd::PopClip)));
}
```

- [ ] **Step 2: Run and observe failure**

Run: `cargo test -p textora-ui form_view_clips_sections_and_clamps_scroll`

Expected: FAIL because FormView is absent.

- [ ] **Step 3: Implement scroll, clipping, and event coordinate translation**

```rust
pub struct FormView {
    rect: Rect,
    sections: Vec<FormSection>,
    scroll_offset: f32,
    content_height: f32,
    focused_id: Option<WidgetId>,
}
```

Use `DrawList::clip`; offset section paint by `-scroll_offset`; translate mouse coordinates by the inverse offset; clamp to `max(content_height - rect.h, 0)`. Tab and Shift+Tab traverse visible enabled child IDs. Category replacement calls `reset_scroll()`.

- [ ] **Step 4: Verify and commit**

Run: `cargo test -p textora-ui form_view`

Expected: PASS.

```bash
git add crates/ui/src/widgets/form/view.rs crates/ui/src/widgets/form/mod.rs crates/ui/src/lib.rs
git commit -m "feat(ui): add scrolling form view"
```

### Task 6: Add the generic ModalFrame widget

**Files:**
- Create: `crates/ui/src/widgets/modal_frame.rs`
- Modify: `crates/ui/src/widgets/mod.rs`
- Modify: `crates/ui/src/lib.rs`

**Interfaces:**
- Produces: `ModalFrame`, `ModalFrameStyle`, close Button identity, and arbitrary content slot.

- [ ] **Step 1: Write failing close and paint tests**

```rust
#[test]
fn modal_frame_paints_surface_and_requests_close() {
    let mut modal = fixture_modal();
    assert!(paint_for_test(&modal).cmds.iter().any(is_modal_surface));
    assert_eq!(click_close(&mut modal), Some(WidgetAction::Overlay(OverlayAction::DismissRequested)));
    assert_eq!(key_escape(&mut modal), Some(WidgetAction::Overlay(OverlayAction::DismissRequested)));
}
```

- [ ] **Step 2: Run and observe failure**

Run: `cargo test -p textora-ui modal_frame_paints_surface_and_requests_close`

Expected: FAIL because ModalFrame is absent.

- [ ] **Step 3: Implement the generic frame**

```rust
pub struct ModalFrame {
    rect: Rect,
    title: Label,
    close_button: Button,
    content: Box<dyn Widget>,
    style: ModalFrameStyle,
}

impl ModalFrame {
    pub fn new(title: impl Into<String>, content: Box<dyn Widget>) -> Self;
    pub fn content_as_any_mut(&mut self) -> &mut dyn Any;
}
```

Re-export the Phase 2 overlay types from crate root. ModalFrame paints only panel surface/header/border; UiShell paints the full-screen scrim. Route close activation and Escape to `DismissRequested`, other events to content. The constructor builds the title Label and close Button from `ModalFrameStyle`; `content_as_any_mut` lets app refresh a business view without rebuilding the Overlay.

- [ ] **Step 4: Verify and commit**

Run: `cargo test -p textora-ui modal_frame`

Expected: PASS.

```bash
git add crates/ui/src/widgets/modal_frame.rs crates/ui/src/widgets/mod.rs crates/ui/src/lib.rs
git commit -m "feat(ui): add generic modal frame"
```

### Task 7: Replace OverlayChild with policy-bearing OverlayEntry

**Files:**
- Modify: `crates/app/src/ui_shell.rs`

**Interfaces:**
- Consumes: Phase 2 Overlay types and existing `KeyboardFocusTarget`.
- Produces: `push_overlay_with_policy(widget, layout, input_policy, dismiss_policy)` and exact modal consumption behavior while preserving the existing fixed popup helper.

- [ ] **Step 1: Write failing event-fallthrough and focus-restore tests**

```rust
#[test]
fn modal_overlay_consumes_unhandled_mouse_wheel_key_and_ime() {
    let mut shell = shell_with_modal(noop_widget());
    for event in modal_probe_events() {
        let result = shell.dispatch(&event, &mut event_ctx());
        assert_eq!(result, Some(WidgetAction::Consumed));
        assert_eq!(shell.fill_event_count(), 0);
    }
}

#[test]
fn dismissing_modal_restores_previous_focus() {
    let mut shell = shell_with_focus(KeyboardFocusTarget::Editor);
    shell.push_test_modal();
    shell.dismiss_overlay();
    assert_eq!(shell.keyboard_focus, KeyboardFocusTarget::Editor);
}
```

- [ ] **Step 2: Run and observe current fallthrough**

Run: `cargo test -p textora-app --lib -- overlay`

Expected: FAIL because current overlay dispatch allows several unhandled events to continue.

- [ ] **Step 3: Implement OverlayEntry and modal dispatch**

```rust
pub(crate) struct OverlayEntry {
    widget: Box<dyn Widget>,
    layout: OverlayLayout,
    layout_rect: Rect,
    input_policy: OverlayInputPolicy,
    dismiss_policy: DismissPolicy,
    restore_focus: KeyboardFocusTarget,
}
```

Resolve layout when screen size/DPI changes; paint modal scrim before the widget; dispatch all modal events to the widget and return `Consumed` when it returns `None`; intercept Escape according to DismissPolicy; restore focus on pop/clear. Keep tooltip overlay separate. Preserve `push_overlay(widget, layout_rect)` as a compatibility wrapper for current popup call sites and add the new policy-bearing method for modal callers.

Add `active_overlay_widget_mut<T: Any>(&mut self) -> Option<&mut T>` for scoped input refresh; it only downcasts the current outer overlay widget and does not expose the overlay vector.

- [ ] **Step 4: Verify and commit**

Run: `cargo test -p textora-app --lib -- overlay`

Run: `cargo check -p textora-app`

Expected: both exit 0.

```bash
git add crates/app/src/ui_shell.rs
git commit -m "refactor(app): enforce overlay input policies"
```

### Task 8: Translate overlay dismissal without leaking to the editor

**Files:**
- Modify: `crates/app/src/events.rs`
- Modify: `crates/app/src/actions.rs`
- Modify: `crates/app/src/app_dispatch.rs`

**Interfaces:**
- Produces: `AppAction::DismissOverlay` and exhaustive `WidgetAction::Overlay` translation.

- [ ] **Step 1: Write the failing translation test**

```rust
#[test]
fn overlay_dismiss_action_maps_once_and_is_consumed() {
    let (actions, consumed) = translate_fixture(WidgetAction::Overlay(OverlayAction::DismissRequested));
    assert!(consumed);
    assert!(matches!(actions.as_slice(), [AppAction::DismissOverlay]));
}
```

- [ ] **Step 2: Run and observe failure**

Run: `cargo test -p textora-app --lib -- overlay_dismiss_action_maps_once_and_is_consumed`

Expected: FAIL because no app action exists.

- [ ] **Step 3: Implement pure translation and dispatch**

Add `AppAction::DismissOverlay`; map OverlayAction in `events.rs`; dispatch by calling `ui_shell.pop_overlay()` and returning `AppEffect::REDRAW`. Do not persist or mutate editor state.

```rust
WidgetAction::Overlay(OverlayAction::DismissRequested) => {
    actions.push(AppAction::DismissOverlay);
}

AppAction::DismissOverlay => {
    self.ui_shell.pop_overlay();
    AppEffect::REDRAW
}
```

- [ ] **Step 4: Verify and commit**

Run: `cargo test -p textora-app --lib -- overlay_dismiss`

Run: `cargo check -p textora-app`

Expected: both exit 0.

```bash
git add crates/app/src/events.rs crates/app/src/actions.rs crates/app/src/app_dispatch.rs
git commit -m "feat(app): route modal dismissal"
```

### Task 9: Phase verification

**Files:**
- Modify only a file implicated by a reproduced verification failure; keep any repair commit within three files.

- [ ] Run `cargo fmt --all -- --check`.
- [ ] Run `cargo test -p textora-ui form`.
- [ ] Run `cargo test -p textora-ui modal`.
- [ ] Run `cargo test -p textora-app --lib -- overlay`.
- [ ] Run `cargo check -p textora-app`.
- [ ] Expected: every command exits 0; tests prove modal events never reach Dock and focus is restored after close.
