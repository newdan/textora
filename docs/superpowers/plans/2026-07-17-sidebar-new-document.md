# Sidebar Typed New Document Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the Sidebar “新建” row into a split button whose primary action creates Markdown and whose dropdown creates TXT, MMAP, or MD typed untitled documents.

**Architecture:** `ui::sidebar` owns the pure `NewDocumentKind` contract, split-button geometry, and dropdown interaction. `app` translates the typed UI intent into a Workspace operation; Workspace creates an untitled `DocItem` with no disk path, a type-specific suggested filename, and the correct plugin. Display, save dialogs, and workspace persistence consume the suggested filename until a real path exists.

**Tech Stack:** Rust 2024, winit event routing, wgpu draw lists, existing `ui::popup_menu`, app Workspace/DocItem, Cargo test, `./scripts/verify.sh`.

## Global Constraints

- Sidebar primary click creates `未命名.md` through the Markdown editor.
- Dropdown order is exactly `新建 TXT`, `新建 MMAP`, `新建 MD`.
- MMAP uses the suggested filename `未命名.mmap.md` and the Mindmap plugin.
- Typed untitled documents keep `DocumentView::file_path == None` until a successful save.
- `ui` must not depend on `DocumentView`, Workspace, commands, events, or any app-layer type.
- Use precise English identifiers; do not introduce broad names such as `data`, `info`, `temp`, `res`, or `flag`.
- Do not use `.unwrap()` in production Rust; use typed control flow or a detailed `.expect(...)` only for proven invariants.
- Run `cargo fmt` before every code commit and ensure the affected crate compiles.
- The final integrated change must pass `./scripts/verify.sh`.

## File Structure

- `crates/ui/src/widgets/sidebar/types.rs`: defines `NewDocumentKind`, typed Sidebar actions, and split-button hover targets.
- `crates/ui/src/widgets/popup_menu/types.rs`: adds typed popup actions for new documents.
- `crates/ui/src/widgets/sidebar/menu.rs`: constructs the three-item new-document menu.
- `crates/ui/src/widgets/sidebar/layout.rs`: stores primary and dropdown button rectangles.
- `crates/ui/src/widgets/sidebar/state.rs`: computes split geometry, paints and hit-tests both regions, opens and dispatches the menu.
- `crates/ui/src/widgets/sidebar/widget_tests.rs`: covers widget-level split-button and menu behavior.
- `crates/app/src/actions.rs`: carries typed creation intent through the app dispatcher.
- `crates/app/src/events.rs`: translates `SidebarAction::NewDocument(kind)` to the app action.
- `crates/app/src/app_dispatch.rs`: dispatches the typed app action to document creation.
- `crates/app/src/tab.rs`: owns the optional suggested filename and resolves the visible document title.
- `crates/app/src/workspace.rs`: creates and restores typed untitled documents with the correct plugin.
- `crates/app/src/app_renderer.rs`: uses `DocItem::doc_title()` for tab and Sidebar labels.
- `crates/app/src/dispatch/commands.rs`: uses the resolved title as the Save As default.
- `crates/app/src/dispatch/tabs.rs`: exposes typed creation and uses the resolved title in close/save flows.
- `docs/manual_test_protocol.md`: records manual split-button verification.

---

### Task 1: Define the UI contract and new-document menu

**Files:**
- Modify: `crates/ui/src/widgets/sidebar/types.rs`
- Modify: `crates/ui/src/widgets/popup_menu/types.rs`
- Modify: `crates/ui/src/widgets/sidebar/menu.rs`

**Interfaces:**
- Consumes: existing `PopupMenu`, `PopupMenuItem`, `PopupMenuAction`, and physical-pixel `Rect` layout conventions.
- Produces: `ui::sidebar::NewDocumentKind`, `SidebarAction::NewDocument(NewDocumentKind)`, `SidebarAction::OpenNewDocumentMenu`, `PopupMenuAction::NewDocument(NewDocumentKind)`, and `build_new_document_menu(Rect, (f32, f32), &UiMetrics) -> PopupMenu`.

- [ ] **Step 1: Add failing menu contract tests**

Add tests to `sidebar/menu.rs` that call the not-yet-defined builder:

```rust
#[test]
fn new_document_menu_has_required_order_and_typed_actions() {
    let settings = crate::settings::Settings::new();
    let metrics = crate::settings::UiMetrics::from_settings(&settings, 1.0);
    let anchor = Rect::new(12.0, 40.0, 196.0, 28.0);

    let menu = build_new_document_menu(anchor, (800.0, 600.0), &metrics);

    let labels: Vec<&str> = menu.items.iter().map(|item| item.label.as_str()).collect();
    assert_eq!(labels, vec!["新建 TXT", "新建 MMAP", "新建 MD"]);
    assert!(matches!(
        menu.items[0].action,
        PMA::NewDocument(NewDocumentKind::Text)
    ));
    assert!(matches!(
        menu.items[1].action,
        PMA::NewDocument(NewDocumentKind::Mindmap)
    ));
    assert!(matches!(
        menu.items[2].action,
        PMA::NewDocument(NewDocumentKind::Markdown)
    ));
}

#[test]
fn new_document_menu_flips_above_when_below_screen() {
    let settings = crate::settings::Settings::new();
    let metrics = crate::settings::UiMetrics::from_settings(&settings, 1.0);
    let anchor = Rect::new(12.0, 560.0, 196.0, 28.0);

    let menu = build_new_document_menu(anchor, (800.0, 600.0), &metrics);

    assert!(menu.menu_rect.bottom() <= 600.0);
    assert!(menu.menu_rect.y < anchor.y);
}
```

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```bash
cargo test -p textora-ui --lib widgets::sidebar::menu::tests::new_document_menu -- --nocapture
```

Expected: compilation fails because `NewDocumentKind` and `build_new_document_menu` do not exist.

- [ ] **Step 3: Add the typed UI enums and menu builder**

In `sidebar/types.rs`, define the shared pure enum and replace the untyped action:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewDocumentKind {
    Text,
    Mindmap,
    Markdown,
}

pub enum SidebarAction {
    // existing variants...
    NewDocument(NewDocumentKind),
    OpenNewDocumentMenu,
    // remaining variants...
}
```

Add `NewDocMenu` to `SidebarHoverButton`. In `popup_menu/types.rs`, add:

```rust
PopupMenuAction::NewDocument(crate::sidebar::NewDocumentKind),
```

In `sidebar/menu.rs`, implement `build_new_document_menu` with `constants::ROW_HEIGHT * metrics.dpi`, a `200.0 * metrics.dpi` width, three non-active items, `show_checkmarks: false`, an anchor below the split button, and the same horizontal clamp/upward-flip rules as `build_settings_menu`.

- [ ] **Step 4: Verify GREEN and format**

Run:

```bash
cargo fmt --all -- --check
cargo test -p textora-ui --lib widgets::sidebar::menu::tests::new_document_menu -- --nocapture
cargo check -p textora-ui
```

Expected: formatting, both new menu tests, and UI compilation pass.

- [ ] **Step 5: Commit the UI contract**

```bash
git add crates/ui/src/widgets/sidebar/types.rs crates/ui/src/widgets/popup_menu/types.rs crates/ui/src/widgets/sidebar/menu.rs
git commit -m "feat(ui): define sidebar new document menu"
```

---

### Task 2: Implement split-button layout and interaction

**Files:**
- Modify: `crates/ui/src/widgets/sidebar/layout.rs`
- Modify: `crates/ui/src/widgets/sidebar/state.rs`
- Modify: `crates/ui/src/widgets/sidebar/widget_tests.rs`

**Interfaces:**
- Consumes: `NewDocumentKind`, `SidebarAction::NewDocument`, `SidebarAction::OpenNewDocumentMenu`, `PopupMenuAction::NewDocument`, and `build_new_document_menu` from Task 1.
- Produces: `SidebarLayout::new_btn_rect` as the Markdown primary region, `SidebarLayout::new_menu_btn_rect` as the arrow region, and widget event behavior that opens/selects the menu locally.

- [ ] **Step 1: Write failing split geometry and primary-action tests**

Add focused tests in `state.rs`:

```rust
#[test]
fn sidebar_new_document_row_is_split_without_overlap() {
    let (state, _) = laid_out_sidebar_state(1.0);
    let layout = state.current_layout().expect("sidebar layout must exist");

    assert!(layout.new_btn_rect.w > layout.new_menu_btn_rect.w);
    assert_eq!(layout.new_btn_rect.right(), layout.new_menu_btn_rect.x);
    assert_eq!(layout.new_btn_rect.y, layout.new_menu_btn_rect.y);
    assert_eq!(layout.new_btn_rect.h, layout.new_menu_btn_rect.h);
}

#[test]
fn sidebar_split_new_document_regions_emit_distinct_actions() {
    let (state, metrics) = laid_out_sidebar_state(1.0);
    let layout = state.current_layout().expect("sidebar layout must exist");
    let primary = layout.new_btn_rect;
    let dropdown = layout.new_menu_btn_rect;

    assert_eq!(
        state.hit_test_px(primary.x + 1.0, primary.y + 1.0, &metrics),
        Some(SidebarAction::NewDocument(NewDocumentKind::Markdown))
    );
    assert_eq!(
        state.hit_test_px(dropdown.x + 1.0, dropdown.y + 1.0, &metrics),
        Some(SidebarAction::OpenNewDocumentMenu)
    );
}
```

Reuse or extract a test-only `laid_out_sidebar_state(dpi)` helper from the existing repeated layout setup; do not add a production helper solely for tests.

- [ ] **Step 2: Run the focused tests and verify RED**

Run:

```bash
cargo test -p textora-ui --lib widgets::sidebar::state::tests::sidebar_new_document_row_is_split -- --exact
cargo test -p textora-ui --lib widgets::sidebar::state::tests::sidebar_split_new_document_regions_emit_distinct_actions -- --exact
```

Expected: compilation fails because `new_menu_btn_rect` and the typed hit-test behavior are missing.

- [ ] **Step 3: Implement layout, hover, painting, and hit testing**

Add `new_menu_btn_rect: Rect` to `SidebarLayout`, including `Rect::ZERO` in hidden layout. Split the existing row with a named logical width constant:

```rust
const NEW_MENU_BUTTON_WIDTH: f32 = 32.0;

let new_row_rect = Rect::new(12.0 * dpi, new_y, w - 24.0 * dpi, new_h);
let new_menu_width = NEW_MENU_BUTTON_WIDTH * dpi;
let new_btn_rect = Rect::new(
    new_row_rect.x,
    new_row_rect.y,
    (new_row_rect.w - new_menu_width).max(0.0),
    new_row_rect.h,
);
let new_menu_btn_rect = Rect::new(
    new_btn_rect.right(),
    new_row_rect.y,
    new_menu_width.min(new_row_rect.w),
    new_row_rect.h,
);
```

Update hover resolution so each region maps to `NewDoc` or `NewDocMenu`. Paint the primary region with the existing plus/text content and paint a centered `chevron-down` icon in the dropdown region. Hit testing must return typed Markdown for the primary region and `OpenNewDocumentMenu` for the dropdown.

Add `SidebarState::open_new_document_menu(screen_w, screen_h, metrics)` which calls Task 1's builder using `new_menu_btn_rect`, and extend `dispatch_menu_click`:

```rust
PMA::NewDocument(kind) => Some(SidebarAction::NewDocument(*kind)),
```

In `SidebarWidget::on_event`, intercept `OpenNewDocumentMenu`, open it in local state, and return `SidebarAction::Hovered` so the app requests redraw. Existing menu capture handles outside clicks; existing Escape handling must close the open menu.

- [ ] **Step 4: Add failing widget menu-selection tests**

In `widget_tests.rs`, add tests that click the dropdown center, assert `widget.state().open_menu().is_some()`, then click each item center in separate test instances and assert the emitted action matches Text, Mindmap, and Markdown. Add one Escape test that opens the menu, sends `Event::KeyDown(KeyCode::Escape, Modifiers::NONE)`, and asserts the menu is closed without a new-document action.

- [ ] **Step 5: Run widget tests and verify failures are behavioral**

Run:

```bash
cargo test -p textora-ui --lib widgets::sidebar::widget_tests::new_document -- --nocapture
```

Expected: any still-unimplemented local menu opening or Escape behavior fails with an assertion, not a setup error.

- [ ] **Step 6: Complete minimal widget behavior and verify GREEN**

Implement only the missing event branches identified by Step 5. Then run:

```bash
cargo fmt --all -- --check
cargo test -p textora-ui --lib widgets::sidebar:: -- --nocapture
cargo check -p textora-ui
```

Expected: all Sidebar tests and UI compilation pass.

- [ ] **Step 7: Commit split-button interaction**

```bash
git add crates/ui/src/widgets/sidebar/layout.rs crates/ui/src/widgets/sidebar/state.rs crates/ui/src/widgets/sidebar/widget_tests.rs
git commit -m "feat(ui): add sidebar new document split button"
```

---

### Task 3: Translate and dispatch typed creation actions

**Files:**
- Modify: `crates/app/src/actions.rs`
- Modify: `crates/app/src/events.rs`
- Modify: `crates/app/src/app_dispatch.rs`

**Interfaces:**
- Consumes: `ui::sidebar::NewDocumentKind` and `SidebarAction::NewDocument(kind)` from Task 1.
- Produces: `AppAction::NewDocument(NewDocumentKind)` with a compiling interim dispatcher arm. Task 5 replaces the interim generic creation with `App::new_typed_untitled_doc(kind)` after its failing behavior test.

- [ ] **Step 1: Replace the old Sidebar translation test with a failing typed test**

In `events.rs`, add:

```rust
#[test]
fn translate_sidebar_new_document_preserves_kind() {
    let settings = ui::settings::Settings::new();
    let sidebar_action = ui::sidebar::SidebarAction::NewDocument(
        ui::sidebar::NewDocumentKind::Mindmap,
    );
    let mut actions = Vec::new();

    translate_sidebar_action(&settings, 1.0, &sidebar_action, &mut actions);

    assert!(matches!(
        actions.as_slice(),
        [AppAction::NewDocument(ui::sidebar::NewDocumentKind::Mindmap)]
    ));
}
```

- [ ] **Step 2: Run the test and verify RED**

Run:

```bash
cargo test -p textora-app --lib events::tests::translate_sidebar_new_document_preserves_kind -- --exact
```

Expected: compilation fails because `AppAction::NewDocument` is missing and the current translator erases the kind into `NewEmptyTab`.

- [ ] **Step 3: Implement typed translation and dispatcher arm**

Add to `AppAction`:

```rust
/// Create an untitled document with a type-specific name and view plugin.
NewDocument(ui::sidebar::NewDocumentKind),
```

Translate without altering the enum:

```rust
S::NewDocument(kind) => actions.push(AppAction::NewDocument(*kind)),
```

Because `PopupMenuAction` is shared with the overlay translator, add its exhaustive arm too:

```rust
PMA::NewDocument(kind) => actions.push(AppAction::NewDocument(*kind)),
```

Add the minimal compiling dispatcher arm:

```rust
AppAction::NewDocument(_) => self.new_untitled_doc(),
```

This is intentionally the minimum GREEN implementation for the action-translation test. Task 5 first proves the missing typed behavior with an app-level failing test, then replaces this arm. Keep `NewEmptyTab` for native menu, keyboard, and tab-bar behavior outside the Sidebar scope.

- [ ] **Step 4: Verify the translation test**

Run:

```bash
cargo test -p textora-app --lib events::tests::translate_sidebar_new_document_preserves_kind -- --exact
```

Expected: PASS and app compilation succeeds.

- [ ] **Step 5: Commit typed action translation**

```bash
git add crates/app/src/actions.rs crates/app/src/events.rs crates/app/src/app_dispatch.rs
git commit -m "feat(app): dispatch typed sidebar documents"
```

---

### Task 4: Add suggested filenames and typed Workspace creation

**Files:**
- Modify: `crates/app/src/tab.rs`
- Modify: `crates/app/src/workspace.rs`
- Modify: `crates/app/src/app_renderer.rs`

**Interfaces:**
- Consumes: `ui::sidebar::NewDocumentKind`.
- Produces: `DocItem::new_untitled(doc, plugin, suggested_file_name)`, `DocItem::doc_title()`, `DocItem::suggested_file_name()`, `DocItem::clear_suggested_file_name()`, `Workspace::new_typed_untitled(kind, dims) -> NavEffect`, and typed titles in tab/Sidebar render inputs.

- [ ] **Step 1: Write failing DocItem title tests**

In `tab.rs`, add tests using an empty `DocumentView` and `EditorPlugin`:

```rust
#[test]
fn untitled_doc_uses_suggested_file_name_until_real_path_exists() {
    let doc = test_document_view();
    let mut item = DocItem::new_untitled(
        doc,
        Box::new(EditorPlugin::new()),
        "未命名.md".to_owned(),
    );

    assert_eq!(item.doc_title(), "未命名.md");
    item.doc.file_path = Some(PathBuf::from("/tmp/real.md"));
    assert_eq!(item.doc_title(), "real.md");
}
```

- [ ] **Step 2: Write failing Workspace type/plugin tests**

In `workspace.rs`, add one table-driven test:

```rust
#[test]
fn typed_untitled_documents_keep_no_path_and_select_name_and_plugin() {
    let cases = [
        (NewDocumentKind::Text, "未命名.txt", PLUGIN_EDITOR),
        (NewDocumentKind::Mindmap, "未命名.mmap.md", PLUGIN_MINDMAP),
        (NewDocumentKind::Markdown, "未命名.md", PLUGIN_MARKDOWN_EDITOR),
    ];

    for (kind, expected_title, expected_plugin) in cases {
        let mut workspace = Workspace::new();
        workspace.new_typed_untitled(kind, test_viewport());
        let entry = workspace.active_entry().expect("typed document must become active");

        assert!(entry.doc.file_path.is_none());
        assert_eq!(entry.doc_title(), expected_title);
        assert_eq!(entry.plugin.name(), expected_plugin);
    }
}
```

- [ ] **Step 3: Run focused tests and verify RED**

Run:

```bash
cargo test -p textora-app --lib tab::tests::untitled_doc_uses_suggested_file_name_until_real_path_exists -- --exact
cargo test -p textora-app --lib workspace::tests::typed_untitled_documents_keep_no_path_and_select_name_and_plugin -- --exact
```

Expected: compilation fails because the suggested-name constructor and typed Workspace method are absent.

- [ ] **Step 4: Implement suggested-name ownership and typed creation**

Add `suggested_file_name: Option<String>` to `DocItem`. Preserve `DocItem::new` for file-backed and legacy generic documents with `None`. Add:

```rust
pub(crate) fn new_untitled(
    doc: DocumentView,
    plugin: Box<dyn ViewPlugin>,
    suggested_file_name: String,
) -> Self;

pub(crate) fn doc_title(&self) -> String {
    self.doc
        .file_path
        .as_ref()
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .or_else(|| self.suggested_file_name.clone())
        .unwrap_or_else(|| "untitled".to_owned())
}

pub(crate) fn clear_suggested_file_name(&mut self) {
    self.suggested_file_name = None;
}

pub(crate) fn suggested_file_name(&self) -> Option<&str> {
    self.suggested_file_name.as_deref()
}
```

In Workspace, add an exhaustive mapping helper returning `(suggested_file_name, plugin_name)` and use the registry with `EditorPlugin` fallback. Construct `DocumentView` exactly like existing `new_untitled`, including `dirty_snapshot_id`, and keep `file_path` untouched.

In `app_renderer.rs`, replace the inline `DocumentView::file_path` title derivation with `v.doc_title()` so tab and Sidebar inputs display the suggested filename before the first save.

- [ ] **Step 5: Add failing persistence round-trip and compatibility tests**

Extend `PersistedTab` with `#[serde(default)] suggested_file_name: Option<String>`. Add tests that snapshot/restore `未命名.mmap.md` and assert the title/plugin survive. Add a TOML compatibility fixture without `suggested_file_name` and assert deserialization yields `None`.

- [ ] **Step 6: Verify persistence test RED, then implement serialization and restore**

Run the two new persistence tests first and confirm the round-trip assertion fails before wiring. Then:

- copy `entry.suggested_file_name().map(str::to_owned)` into `PersistedTab` during snapshot;
- restore via `DocItem::new_untitled` when a suggested name exists;
- use `DocItem::new` when the field is absent;
- preserve the already persisted `active_plugin` selection.

Update every explicit `PersistedTab` initializer in `workspace.rs` with `suggested_file_name: None` unless that test specifically covers typed restoration.

- [ ] **Step 7: Verify GREEN and compile Workspace consumers**

Run:

```bash
cargo fmt --all -- --check
cargo test -p textora-app --lib workspace::tests::typed_untitled -- --nocapture
cargo test -p textora-app --lib workspace::tests::persisted_workspace -- --nocapture
cargo check -p textora-app
```

Expected: typed creation, persistence compatibility tests, and app compilation pass.

- [ ] **Step 8: Commit Workspace model**

```bash
git add crates/app/src/tab.rs crates/app/src/workspace.rs crates/app/src/app_renderer.rs
git commit -m "feat(app): model typed untitled documents"
```

---

### Task 5: Integrate typed creation with display and save flows

**Files:**
- Modify: `crates/app/src/app_dispatch.rs`
- Modify: `crates/app/src/dispatch/commands.rs`
- Modify: `crates/app/src/dispatch/tabs.rs`

**Interfaces:**
- Consumes: `Workspace::new_typed_untitled`, `DocItem::doc_title`, and `DocItem::clear_suggested_file_name` from Task 4.
- Produces: `App::new_typed_untitled_doc(kind) -> AppEffect`, final typed action dispatch, and suggested Save As defaults.

- [ ] **Step 1: Write failing app-level typed creation test**

In the existing `dispatch/tabs.rs` test module or `app_tests.rs` if that module owns App construction, add:

```rust
#[test]
fn new_typed_untitled_doc_activates_markdown_with_suggested_title() {
    let mut app = App::new(None);

    let effect = app.new_typed_untitled_doc(NewDocumentKind::Markdown);
    let entry = app.workspace.active_entry().expect("new document must be active");

    assert!(effect.redraw);
    assert_eq!(entry.doc_title(), "未命名.md");
    assert!(entry.doc.file_path.is_none());
    assert_eq!(entry.plugin.name(), PLUGIN_MARKDOWN_EDITOR);
}
```

- [ ] **Step 2: Run the focused test and verify RED**

Run the exact test name with `cargo test -p textora-app --lib ... -- --exact`.

Expected: compilation fails because `new_typed_untitled_doc` does not exist.

- [ ] **Step 3: Implement typed App creation and title injection**

In `dispatch/tabs.rs`, mirror `new_untitled_doc`:

```rust
pub(crate) fn new_typed_untitled_doc(
    &mut self,
    kind: ui::sidebar::NewDocumentKind,
) -> AppEffect {
    let viewport = self.viewport_dimensions(self.screen_height());
    let effect = self.workspace.new_typed_untitled(kind, viewport);
    self.handle_workspace_effect(effect)
}
```

In `app_dispatch.rs`, replace Task 3's interim arm with:

```rust
AppAction::NewDocument(kind) => self.new_typed_untitled_doc(kind),
```

- [ ] **Step 4: Add a pure save-default helper test before dialog wiring**

Native dialogs cannot be asserted reliably in unit tests. Extract a single-purpose helper in `dispatch/commands.rs`:

```rust
fn save_dialog_default_name(entry: Option<&DocItem>) -> String {
    entry.map(DocItem::doc_title).unwrap_or_else(|| "未命名".to_owned())
}
```

Write tests proving a typed untitled entry returns `未命名.mmap.md` and a real path returns its real basename. Run them before implementation and confirm the missing helper fails compilation.

- [ ] **Step 5: Wire save and close prompts to resolved titles**

Use `save_dialog_default_name(self.workspace.entry(active_idx))` in `save_active_entry`. In `dispatch/tabs.rs`, use `entry.doc_title()` for the unsaved-changes message and Save As default. After a successful `save_as`, call `clear_suggested_file_name()` on that entry. Do not clear it on error or cancel.

- [ ] **Step 6: Verify Tasks 3 and 5 together**

Run:

```bash
cargo fmt --all -- --check
cargo test -p textora-app --lib events::tests::translate_sidebar_new_document_preserves_kind -- --exact
cargo test -p textora-app --lib dispatch::tabs -- --nocapture
cargo test -p textora-app --lib dispatch::commands -- --nocapture
cargo check -p textora-app
```

Expected: translation, typed creation, save-name tests, and app compilation all pass.

- [ ] **Step 7: Commit typed creation and save integration**

```bash
git add crates/app/src/app_dispatch.rs crates/app/src/dispatch/commands.rs crates/app/src/dispatch/tabs.rs
git commit -m "feat(app): integrate typed untitled document names"
```

---

### Task 6: Manual protocol and full verification

**Files:**
- Modify: `docs/manual_test_protocol.md`

**Interfaces:**
- Consumes: completed Tasks 1–5.
- Produces: documented manual acceptance checks and fresh full-suite verification evidence.

- [ ] **Step 1: Add the manual acceptance section**

Append a dated Sidebar new-document section covering:

```markdown
## Sidebar 类型化新建（2026-07-17）

- 点击“新建”主体，出现 `未命名.md`，使用 Markdown 编辑器。
- 点击右侧箭头，菜单依次显示“新建 TXT / 新建 MMAP / 新建 MD”。
- 三个菜单项分别创建 `未命名.txt`、`未命名.mmap.md`、`未命名.md`。
- 新建 MMAP 直接显示可编辑思维导图视图。
- 点击菜单外部或按 Escape 关闭菜单，不创建文档。
- 首次保存的默认文件名与当前未命名类型一致；取消后名称不变。
```

- [ ] **Step 2: Run formatting and focused suites**

```bash
cargo fmt --all -- --check
cargo test -p textora-ui --lib widgets::sidebar:: -- --nocapture
cargo test -p textora-app --lib workspace::tests:: -- --nocapture
cargo test -p textora-app --lib events::tests::translate_sidebar_new_document_preserves_kind -- --exact
cargo check -p textora-ui
cargo check -p textora-app
```

Expected: every command exits 0 with no test failures.

- [ ] **Step 3: Run the repository-wide required verification**

```bash
./scripts/verify.sh
```

Expected: exit 0; all formatting, checks, tests, and repository policy gates pass.

- [ ] **Step 4: Inspect the final diff and requirements**

```bash
git diff --check
git status --short
git log --oneline -6
```

Confirm every design requirement has a corresponding implementation/test and no unrelated user changes were modified.

- [ ] **Step 5: Commit the manual protocol**

```bash
git add docs/manual_test_protocol.md
git commit -m "docs: cover sidebar typed new documents"
```

Do not claim completion until `./scripts/verify.sh` has been run fresh after the final code change.
