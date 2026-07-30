# mmap Style Panel and File-Level Theme Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 mmap 增加固定 280 逻辑像素的右侧风格面板，通过两列缩略图选择 6 个固定画布主题，并把所选稳定 ID 写入当前 `.mmap.md` 文件。

**Architecture:** `textora-ui` 提供纯内置配色数据、面板输入/动作和无应用依赖的 widget；`textora-markdown` 解析并编辑 MMF 全局 TOML，通过现有 `PluginQuery → EditPlan` 协议暴露主题状态和编辑计划；`textora-app` 保存每标签页的面板会话状态、把面板作为右侧 Dock child，并统一执行编辑事务。应用外壳继续使用全局 `Theme`，mmap 绘制只组合“文件级颜色方案 + 既有 mmap 几何”。

**Tech Stack:** Rust 2024、winit、项目自有 `ui::Widget`/`Dock`/`DrawList`、MMF TOML 解析器、`ViewPlugin`、`EditPlan`、`CanvasViewportSession`。

## Global Constraints

- 全程遵守 `ui` 纯数据边界：`ui` 不得依赖或访问 `DocumentView`、`Workspace`、Commands 或 app 状态。
- mmap 主题只改变画布、节点、连线、分支和语义状态颜色；不得改变应用外壳或 mmap 几何。
- 主题稳定 ID 固定为 `warm-night`、`dawn`、`amber`、`meadow`、`tide`、`iris`。
- 文件缺少 `theme` 时回退 `warm-night`，但不得自动修改文件；未知 ID 保留原值并回退显示。
- 面板逻辑宽度固定为 `280px`，两列三行缩略图，默认关闭；开关和折叠状态仅随标签页会话存活。
- 切换主题必须通过一个可撤销的 `EditTransaction` 更新源码并产生 dirty 状态；重复选择当前主题必须为 no-op。
- 面板开关不得改变 zoom；视口宽度变化后，旧视口中心内容点必须移动到新视口中心。
- Rust 互斥状态优先用 `enum`，不得用多个布尔值拼装主题解析状态或面板开关状态。
- 禁止 `.unwrap()`；仅在不变量已由前置条件保证时使用带明确理由的 `.expect(...)`。
- 每个任务最多修改 3 个文件，每次提交前必须运行该任务列出的编译和测试命令。
- 最终必须运行 `cargo fmt --all -- --check` 和 `./scripts/verify.sh`。

---

## File Map and Locked Interfaces

| File | Responsibility |
|---|---|
| `crates/ui/src/theme/mindmap.rs` | 内置主题注册表、固定颜色方案、主题选择状态、渲染借用视图 |
| `crates/markdown/src/mmf/model.rs` | MMF 全局属性源码范围 |
| `crates/markdown/src/mmf/parser.rs` | 解析 `theme` 及精确 TOML 值范围 |
| `crates/markdown/src/mmf/edit.rs` | 生成设置主题的局部 `EditPlan` |
| `crates/ui/src/plugin.rs` | mmap 主题查询与编辑计划协议 |
| `crates/markdown/src/mindmap_view.rs` | 查询主题状态、解析主题并组合几何 |
| `crates/markdown/src/mmf/canvas.rs` | 只消费 `MindmapRenderTheme` 绘制 |
| `crates/ui/src/core/widget.rs` | 面板动作加入统一 `WidgetAction`，分配键盘焦点 ID |
| `crates/ui/src/widgets/mindmap_style_panel.rs` | 右侧面板布局、绘制、命中和键盘交互 |
| `crates/ui/src/widgets/title_bar.rs` | mmap 风格按钮 |
| `crates/app/src/tab.rs` | 每标签页面板会话状态 |
| `crates/app/src/ui_shell.rs` | 右侧 Dock child 与面板输入注入 |
| `crates/app/src/canvas_viewport.rs` | 视口尺寸变化时保持中心内容锚点 |
| `crates/app/src/app_renderer.rs` | 从活动插件/标签页构造标题栏和面板输入 |
| `crates/app/src/actions.rs`、`events.rs`、`app_dispatch.rs` | UI 动作翻译、状态归约、主题事务执行 |

跨任务接口固定如下，后续任务不得自行改名：

```rust
// ui::theme
pub const DEFAULT_MINDMAP_COLOR_SCHEME_ID: &str = "warm-night";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MindmapThemeSelection {
    Default,
    Selected(String),
    Unknown(String),
    InvalidMetadata,
}

pub fn built_in_mindmap_color_schemes() -> &'static [MindmapColorScheme];
pub fn find_mindmap_color_scheme(id: &str) -> Option<&'static MindmapColorScheme>;
pub fn resolve_mindmap_theme_selection(id: Option<&str>) -> MindmapThemeSelection;

// textora-markdown::mmf::model / parser / edit
pub struct GlobalPropertySource {
    pub body_range: std::ops::Range<usize>,
    pub theme_value_range: Option<std::ops::Range<usize>>,
}
pub(crate) fn parse_global_property_source(
    source: &str,
) -> Result<Option<GlobalPropertySource>, MmfDiagnostic>;

pub(crate) fn plan_set_mindmap_theme(
    tree: &Tree,
    source: &str,
    theme_id: &str,
    source_generation: u32,
    cursor_byte: usize,
) -> EditPlan;

// ui::plugin
PluginQuery::MindmapThemeSelection
PluginQuery::PlanMindmapTheme { theme_id: String, source_generation: u32 }
PluginResponse::MindmapThemeSelection(MindmapThemeSelection)

// ui::mindmap_style_panel
pub struct MindmapStylePanelInput {
    pub selection: MindmapThemeSelection,
    pub options: Vec<MindmapStyleOption>,
    pub presets_expanded: bool,
}
pub enum MindmapStylePanelAction { Close, TogglePresets, SelectTheme(String) }
pub const PANEL_WIDTH_LOGICAL: f32 = 280.0;
```

---

### Task 1: Built-in mmap color scheme registry

**Files:**
- Modify: `crates/ui/src/theme/mindmap.rs`

**Interfaces:**
- Consumes: existing `MindmapCanvasTheme`, `MindmapNodeTheme`, `MindmapSemanticTheme`, `MindmapGeometry`.
- Produces: all `ui::theme` interfaces listed in “Locked Interfaces”, plus `MindmapRenderTheme<'a>`.

- [ ] **Step 1: Write failing registry and selection tests**

Add table-driven tests at the end of `crates/ui/src/theme/mindmap.rs`:

```rust
#[test]
fn built_in_color_schemes_have_stable_unique_ids() {
    let schemes = built_in_mindmap_color_schemes();
    let ids = schemes.iter().map(|scheme| scheme.id).collect::<Vec<_>>();
    assert_eq!(
        ids,
        ["warm-night", "dawn", "amber", "meadow", "tide", "iris"]
    );
    let unique = ids.iter().copied().collect::<std::collections::BTreeSet<_>>();
    assert_eq!(unique.len(), ids.len());
}

#[test]
fn absent_and_unknown_theme_ids_have_distinct_selection_states() {
    assert_eq!(resolve_mindmap_theme_selection(None), MindmapThemeSelection::Default);
    assert_eq!(
        resolve_mindmap_theme_selection(Some("tide")),
        MindmapThemeSelection::Selected("tide".into())
    );
    assert_eq!(
        resolve_mindmap_theme_selection(Some("future-theme")),
        MindmapThemeSelection::Unknown("future-theme".into())
    );
}

#[test]
fn fixed_scheme_colors_do_not_depend_on_application_theme() {
    let scheme = find_mindmap_color_scheme("dawn").expect("dawn is a built-in scheme");
    let dark_geometry = &MindmapTheme::default_dark().geometry;
    let light_geometry = &MindmapTheme::default_light().geometry;
    let dark_render = MindmapRenderTheme::new(scheme, dark_geometry);
    let light_render = MindmapRenderTheme::new(scheme, light_geometry);
    assert_eq!(dark_render.canvas.background, light_render.canvas.background);
    assert_eq!(dark_render.canvas.branch_palette, light_render.canvas.branch_palette);
}
```

- [ ] **Step 2: Run the tests and confirm the missing API failure**

Run:

```bash
cargo test -p textora-ui --lib -- theme::mindmap
```

Expected: compilation fails because `built_in_mindmap_color_schemes`, `MindmapThemeSelection`, and `MindmapRenderTheme` do not exist.

- [ ] **Step 3: Add the pure scheme types and registry**

Add these public types and functions in `mindmap.rs`:

```rust
pub const DEFAULT_MINDMAP_COLOR_SCHEME_ID: &str = "warm-night";

#[derive(Clone, Debug)]
pub struct MindmapColorScheme {
    pub id: &'static str,
    pub display_name: &'static str,
    pub canvas: MindmapCanvasTheme,
    pub node: MindmapNodeTheme,
    pub semantic: MindmapSemanticTheme,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MindmapThemeSelection {
    Default,
    Selected(String),
    Unknown(String),
    InvalidMetadata,
}

#[derive(Clone, Copy)]
pub struct MindmapRenderTheme<'a> {
    pub canvas: &'a MindmapCanvasTheme,
    pub node: &'a MindmapNodeTheme,
    pub semantic: &'a MindmapSemanticTheme,
    pub geometry: &'a MindmapGeometry,
}

impl<'a> MindmapRenderTheme<'a> {
    pub fn new(scheme: &'a MindmapColorScheme, geometry: &'a MindmapGeometry) -> Self {
        Self {
            canvas: &scheme.canvas,
            node: &scheme.node,
            semantic: &scheme.semantic,
            geometry,
        }
    }
}
```

Use a process-wide `std::sync::OnceLock<Vec<MindmapColorScheme>>`. Build `warm-night` by extracting the color members of `MindmapTheme::default_dark()`. Build the other five with a single helper that derives default/root/depth styles from these exact sRGB palette rows, then calls the existing gamma-correction methods once before storing them:

```rust
const SCHEME_PALETTES: [(&str, &str, &str, &str, [&str; 6]); 5] = [
    ("dawn", "晨曦", "#FFF9F6", "#F35F67", ["#F35F67", "#FF9D66", "#9FD3B6", "#8EDBD2", "#68BDE3", "#CE82DB"]),
    ("amber", "琥珀", "#FFF9F0", "#E58B2A", ["#E58B2A", "#F2B84B", "#D97C46", "#A9C45A", "#63B7A6", "#C78BCF"]),
    ("meadow", "青禾", "#F7FBF6", "#4E9B62", ["#4E9B62", "#77B875", "#A8C96F", "#5FAF9C", "#67A6C8", "#B286C3"]),
    ("tide", "潮汐", "#F5FAFD", "#318EB8", ["#318EB8", "#54A9CC", "#62C0BA", "#6F9ED1", "#8B83C7", "#C27FB4"]),
    ("iris", "鸢尾", "#FBF7FD", "#9657B5", ["#9657B5", "#B66CC2", "#D47FAC", "#7D8FD0", "#55AAA3", "#D69A5D"]),
];
```

The helper must use `#FFFFFF` cards, `#202124` primary text, `#5F6368` muted text, `#DADCE0` default borders, and the existing fixed semantic colors for todo/doing/done/blocked/canceled and P0–P3. Do not duplicate geometry in `MindmapColorScheme`.

- [ ] **Step 4: Run UI theme tests and compile the workspace**

Run:

```bash
cargo test -p textora-ui --lib -- theme::mindmap
cargo check -p textora-app
```

Expected: all selected tests pass and `textora-app` finishes with exit code 0.

- [ ] **Step 5: Commit Task 1**

```bash
git add crates/ui/src/theme/mindmap.rs
git commit -m "feat(ui): add fixed mmap color schemes"
```

---

### Task 2: MMF global theme source ranges and edit plan

**Files:**
- Modify: `crates/markdown/src/mmf/model.rs`
- Modify: `crates/markdown/src/mmf/parser.rs`
- Modify: `crates/markdown/src/mmf/edit.rs`

**Interfaces:**
- Consumes: existing `Tree`, `TomlBlock`, `EditPlan`, `EditTransaction`, `TextReplacement`.
- Produces: `GlobalPropertySource`, `parse_global_property_source(...)`, and `plan_set_mindmap_theme(...)` without widening `Tree` or breaking existing tree fixtures.

- [ ] **Step 1: Write failing parser range tests**

Add tests proving the parser records the whole body and exact quoted theme literal:

```rust
#[test]
fn global_property_source_records_theme_literal_range() {
    let source = "```toml mindmap\nversion = 1\ntheme = \"dawn\"\n```\n\n# Root\n";
    let property_source = parse_global_property_source(source)
        .expect("valid mmap metadata")
        .expect("global property source");
    let value_start = source.find("\"dawn\"").expect("theme literal");
    assert_eq!(property_source.theme_value_range, Some(value_start..value_start + 6));
    assert_eq!(
        &source[property_source.body_range],
        "version = 1\ntheme = \"dawn\"\n"
    );
}

#[test]
fn global_property_source_preserves_crlf_ranges() {
    let source = "```toml mindmap\r\nversion = 1\r\ntheme = \"tide\"\r\n```\r\n# Root\r\n";
    let range = parse_global_property_source(source)
        .expect("valid CRLF mmap metadata")
        .expect("global property source")
        .theme_value_range
        .expect("theme range");
    assert_eq!(&source[range], "\"tide\"");
}

#[test]
fn global_theme_requires_a_string_literal() {
    let error = parse("```toml mindmap\ntheme = 7\n```\n# Root\n")
        .expect_err("numeric theme metadata must be rejected");
    assert_eq!(error.kind, ParseErrorKind::InvalidToml);
}
```

- [ ] **Step 2: Run the parser tests and verify failure**

Run:

```bash
cargo test -p textora-markdown --lib -- mmf::parser::tests::global_property_source
```

Expected: compilation fails because `parse_global_property_source` and `GlobalPropertySource` are missing.

- [ ] **Step 3: Add typed global property source metadata**

In `model.rs` add:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalPropertySource {
    pub body_range: Range<usize>,
    pub theme_value_range: Option<Range<usize>>,
}
```

Keep `Tree` unchanged so existing direct tree fixtures remain source-compatible. Change the global-property parser to return a small internal result containing the existing `HashMap<String, String>` plus `Option<GlobalPropertySource>`. `parse()` stores only the map in `Tree`; expose the same parser through:

```rust
pub(crate) fn parse_global_property_source(
    source: &str,
) -> Result<Option<GlobalPropertySource>, MmfDiagnostic>;
```

The edit planner calls this API only on user action. Implement a generic line scanner:

```rust
fn find_quoted_value_range(
    source: &str,
    body_range: Range<usize>,
    key: &str,
) -> Option<Range<usize>>;
```

It must match only a full TOML key before `=`, trim whitespace, require a quoted string literal, honor escaped quotes while finding the closing quote, and return the range including both quotes. When a `theme` key exists, require `toml::Value::as_str()` and return `InvalidToml` for integers, arrays, tables, or booleans so a Ready document can never contain an uneditable theme value.

- [ ] **Step 4: Write failing theme edit-plan tests**

Add this helper and the concrete cases to `mmf/edit.rs` tests:

```rust
fn applied_source(source: &str, plan: EditPlan) -> (String, EditSelection) {
    let EditPlan::Apply(transaction) = plan else {
        panic!("expected an applied theme transaction");
    };
    let mut result = source.to_owned();
    let mut replacements = transaction.replacements;
    replacements.sort_by_key(|replacement| replacement.range.start);
    for replacement in replacements.into_iter().rev() {
        result.replace_range(replacement.range, &replacement.text);
    }
    (result, transaction.selection_after)
}

#[test]
fn set_theme_replaces_only_existing_theme_literal() {
    let source = "```toml mindmap\nversion = 1\ntheme = \"dawn\"\n```\n# Root\n";
    let tree = parser::parse(source).expect("fixture must parse");
    let (result, _) = applied_source(
        source,
        plan_set_mindmap_theme(&tree, source, "tide", 7, source.len()),
    );
    assert_eq!(
        result,
        "```toml mindmap\nversion = 1\ntheme = \"tide\"\n```\n# Root\n"
    );
}

#[test]
fn set_theme_inserts_field_before_existing_closing_fence() {
    let source = "```toml mindmap\nversion = 1\nlayout = \"auto\"\n```\n# Root\n";
    let tree = parser::parse(source).expect("fixture must parse");
    let (result, _) = applied_source(
        source,
        plan_set_mindmap_theme(&tree, source, "amber", 3, source.len()),
    );
    assert_eq!(
        result,
        "```toml mindmap\nversion = 1\nlayout = \"auto\"\ntheme = \"amber\"\n```\n# Root\n"
    );
}

#[test]
fn set_theme_creates_global_block_before_root() {
    let source = "# Root\n";
    let tree = parser::parse(source).expect("fixture must parse");
    let (result, _) = applied_source(
        source,
        plan_set_mindmap_theme(&tree, source, "tide", 1, source.len()),
    );
    assert_eq!(
        result,
        "```toml mindmap\nversion = 1\ntheme = \"tide\"\n```\n\n# Root\n"
    );
}

#[test]
fn set_theme_preserves_crlf_newlines() {
    let source = "```toml mindmap\r\nversion = 1\r\n```\r\n# Root\r\n";
    let tree = parser::parse(source).expect("fixture must parse");
    let (result, _) = applied_source(
        source,
        plan_set_mindmap_theme(&tree, source, "iris", 1, source.len()),
    );
    assert_eq!(
        result,
        "```toml mindmap\r\nversion = 1\r\ntheme = \"iris\"\r\n```\r\n# Root\r\n"
    );
}

#[test]
fn selecting_current_theme_returns_consume() {
    let source = "```toml mindmap\ntheme = \"dawn\"\n```\n# Root\n";
    let tree = parser::parse(source).expect("fixture must parse");
    assert_eq!(
        plan_set_mindmap_theme(&tree, source, "dawn", 1, source.len()),
        EditPlan::Consume
    );
}

#[test]
fn set_theme_adjusts_caret_when_insertion_precedes_cursor() {
    let source = "# Root\n";
    let tree = parser::parse(source).expect("fixture must parse");
    let original_cursor = source.find("Root").expect("root title") + 2;
    let (result, selection) = applied_source(
        source,
        plan_set_mindmap_theme(&tree, source, "tide", 1, original_cursor),
    );
    let EditSelection::Caret(cursor_after) = selection else {
        panic!("theme edit must preserve a caret");
    };
    assert_eq!(&result[cursor_after - 2..cursor_after + 2], "Root");
}
```

Use exact expected source strings, for example the no-block case must become:

````rust
let expected = "```toml mindmap\nversion = 1\ntheme = \"tide\"\n```\n\n# Root\n";
````

- [ ] **Step 5: Run edit tests and verify the missing planner failure**

Run:

```bash
cargo test -p textora-markdown --lib -- mmf::edit::tests::set_theme
cargo test -p textora-markdown --lib -- mmf::edit::tests::selecting_current_theme
```

Expected: compilation fails because `plan_set_mindmap_theme` does not exist.

- [ ] **Step 6: Implement the minimal local edit planner**

Implement `plan_set_mindmap_theme` with the following semantic branches. `plan_existing_global_block` replaces the quoted literal when present and otherwise inserts `theme = "{theme_id}"` at `body_range.end`; `plan_new_global_block` inserts the exact standard block used by the tests:

```rust
match crate::mmf::parser::parse_global_property_source(source) {
    Err(_) => EditPlan::Consume,
    Ok(Some(_)) if tree.global_props.get("theme").is_some_and(|id| id == theme_id) => EditPlan::Consume,
    Ok(Some(property_source)) => plan_existing_global_block(property_source),
    Ok(None) => plan_new_global_block(),
}
```

Do not add the source ranges to `Tree`; that would force unrelated direct `Tree` fixtures in layout and edit tests to change in the same commit.

Add a focused helper that shifts the caret by the replacement length delta when the replacement is before it, clamps a caret inside a replaced range to the new value end, and leaves earlier carets unchanged. Return one `EditTransaction` with `EditSelection::Caret(adjusted_cursor)`.

- [ ] **Step 7: Run MMF tests and compile**

Run:

```bash
cargo test -p textora-markdown --lib -- mmf::parser
cargo test -p textora-markdown --lib -- mmf::edit
cargo check -p textora-app
```

Expected: all commands exit 0.

- [ ] **Step 8: Commit Task 2**

```bash
git add crates/markdown/src/mmf/model.rs crates/markdown/src/mmf/parser.rs crates/markdown/src/mmf/edit.rs
git commit -m "feat(markdown): plan mmap theme metadata edits"
```

---

### Task 3: Plugin theme protocol and file-level mmap rendering

**Files:**
- Modify: `crates/ui/src/plugin.rs`
- Modify: `crates/markdown/src/mindmap_view.rs`
- Modify: `crates/markdown/src/mmf/canvas.rs`

**Interfaces:**
- Consumes: Task 1 registry and `MindmapRenderTheme`; Task 2 `plan_set_mindmap_theme`.
- Produces: locked `PluginQuery`/`PluginResponse` variants and fixed-theme rendering.

- [ ] **Step 1: Write failing MindmapView theme query tests**

Add tests in `mindmap_view.rs` for four states:

```rust
#[test]
fn theme_query_distinguishes_default_selected_unknown_and_invalid_metadata() {
    for (source, expected) in [
        ("# Root\n", MindmapThemeSelection::Default),
        (
            "```toml mindmap\ntheme = \"tide\"\n```\n# Root\n",
            MindmapThemeSelection::Selected("tide".into()),
        ),
        (
            "```toml mindmap\ntheme = \"future\"\n```\n# Root\n",
            MindmapThemeSelection::Unknown("future".into()),
        ),
        (
            "```toml mindmap\ntheme = [\n```\n# Root\n",
            MindmapThemeSelection::InvalidMetadata,
        ),
    ] {
        let (view, doc) = view_with_source(source);
        assert!(matches!(
            view.query(PluginQuery::MindmapThemeSelection, &doc),
            PluginResponse::MindmapThemeSelection(actual) if actual == expected
        ));
    }
}

#[test]
fn theme_plan_query_rejects_unknown_scheme_and_plans_known_scheme() {
    let (view, doc) = view_with_source("# Root\n");
    assert!(matches!(
        view.query(
            PluginQuery::PlanMindmapTheme {
                theme_id: "future".into(),
                source_generation: 1,
            },
            &doc,
        ),
        PluginResponse::EditPlan(EditPlan::Consume)
    ));
    assert!(matches!(
        view.query(
            PluginQuery::PlanMindmapTheme {
                theme_id: "tide".into(),
                source_generation: 1,
            },
            &doc,
        ),
        PluginResponse::EditPlan(EditPlan::Apply(transaction))
            if transaction.source_generation == 1
    ));
}
```

- [ ] **Step 2: Run the query tests and verify protocol failure**

Run:

```bash
cargo test -p textora-markdown --lib -- mindmap_view::tests::theme_
```

Expected: compilation fails because the theme query variants do not exist.

- [ ] **Step 3: Add typed plugin queries and response**

In `ui/src/plugin.rs` add:

```rust
PluginQuery::MindmapThemeSelection,
PluginQuery::PlanMindmapTheme { theme_id: String, source_generation: u32 },

PluginResponse::MindmapThemeSelection(crate::theme::MindmapThemeSelection),
```

In `MindmapView::query`, return `InvalidMetadata` for `MindmapDocumentState::Invalid`; for Ready, classify `tree.global_props.get("theme")`. For planning, require the requested generation to equal the Ready generation and require `find_mindmap_color_scheme(&theme_id).is_some()` before calling Task 2’s planner with `self.cursor_byte.unwrap_or(source.len())`.

- [ ] **Step 4: Write failing fixed-render tests**

Add this test, using the existing `render_test_draw_list_with_theme` helper and a small helper that collects rendered card rectangles from `DrawCmd::FillRoundedRect`:

```rust
fn rendered_card_rects(draw_list: &DrawList) -> Vec<Rect> {
    draw_list
        .cmds
        .iter()
        .filter_map(|command| match command {
            DrawCmd::FillRect { rect, radius, .. } if *radius > 0.0 => Some(*rect),
            _ => None,
        })
        .collect()
}

#[test]
fn file_theme_rendering_is_fixed_across_application_theme_modes() {
    let source = "```toml mindmap\ntheme = \"dawn\"\n```\n# Root\n## Child\n";
    let (mut dark_view, dark_doc) = view_with_source(source);
    let (mut light_view, light_doc) = view_with_source(source);
    let dark_app_theme = Theme::from_definition(&ThemeDefinition::default_dark());
    let light_app_theme = Theme::from_definition(&ThemeDefinition::default_light());
    let dark_draw = render_test_draw_list_with_theme(&mut dark_view, &dark_doc, &dark_app_theme);
    let light_draw = render_test_draw_list_with_theme(&mut light_view, &light_doc, &light_app_theme);
    let dawn = ui::theme::find_mindmap_color_scheme("dawn")
        .expect("dawn is a built-in mmap scheme");

    for draw_list in [&dark_draw, &light_draw] {
        assert!(draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::FillRect { color, .. } if *color == dawn.canvas.background
        )));
        let branch = dawn.canvas.branch_color(0).expect("dawn has branch colors");
        assert!(draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::TaperedMesh { color, .. } if *color == branch
        )));
    }
    assert_eq!(rendered_card_rects(&dark_draw), rendered_card_rects(&light_draw));
}
```

- [ ] **Step 5: Run the render test and verify it fails against app theme colors**

Run:

```bash
cargo test -p textora-markdown --lib -- mindmap_view::tests::file_theme_rendering_is_fixed
```

Expected: FAIL because rendering still uses `theme.mindmap` from the application theme.

- [ ] **Step 6: Convert mmap canvas rendering to `MindmapRenderTheme`**

Change `mmf/canvas.rs` rendering helpers from `&Theme` to `&MindmapRenderTheme<'_>` and mechanically replace `theme.mindmap.canvas/node/semantic/geometry` with `theme.canvas/node/semantic/geometry`. Keep test helpers that start from a full `Theme`, but adapt them through:

```rust
fn render_theme(theme: &Theme) -> MindmapRenderTheme<'_> {
    let scheme = find_mindmap_color_scheme(DEFAULT_MINDMAP_COLOR_SCHEME_ID)
        .expect("the default mmap scheme is registered");
    MindmapRenderTheme::new(scheme, &theme.mindmap.geometry)
}
```

In `MindmapView`, resolve a `'static` scheme from Ready state, combine it with `&theme.mindmap.geometry`, and pass that render theme to `mmf::canvas::render`. `prepare_canvas` and `update_layout_constants` continue to read only the application theme’s geometry. Invalid metadata uses the default scheme for its diagnostic canvas.

- [ ] **Step 7: Run plugin, canvas, and cache tests**

Run:

```bash
cargo test -p textora-markdown --lib -- mindmap_view::tests::theme_
cargo test -p textora-markdown --lib -- mindmap_view::tests::file_theme_rendering_is_fixed
cargo test -p textora-markdown --lib -- mmf::canvas
cargo test -p textora-markdown --lib -- mmap_non_geometry_render_changes_keep_connector_mesh_cache
cargo check -p textora-app
```

Expected: all commands exit 0; the existing non-geometry cache test still proves color changes do not discard connector geometry.

- [ ] **Step 8: Commit Task 3**

```bash
git add crates/ui/src/plugin.rs crates/markdown/src/mindmap_view.rs crates/markdown/src/mmf/canvas.rs
git commit -m "feat(markdown): render file-level mmap themes"
```

---

### Task 4: Unified widget action boundary for the style panel

**Files:**
- Modify: `crates/ui/src/core/widget.rs`
- Modify: `crates/app/src/events.rs`

**Interfaces:**
- Consumes: existing `WidgetAction` exhaustive routing.
- Produces: `MindmapStylePanelAction`, `WidgetAction::MindmapStylePanel`, and `ids::MINDMAP_STYLE_PANEL`.

- [ ] **Step 1: Add a compile-time action exhaustiveness test**

In `ui/src/core/widget.rs` tests add a construction test for:

```rust
let action = WidgetAction::MindmapStylePanel(
    MindmapStylePanelAction::SelectTheme("tide".into()),
);
assert!(matches!(
    action,
    WidgetAction::MindmapStylePanel(MindmapStylePanelAction::SelectTheme(id)) if id == "tide"
));
```

- [ ] **Step 2: Run the UI test and confirm missing action types**

Run:

```bash
cargo test -p textora-ui --lib -- core::widget
```

Expected: compilation fails because the variants do not exist.

- [ ] **Step 3: Add action types and a reserved focus ID**

Add:

```rust
pub const MINDMAP_STYLE_PANEL: WidgetId = WidgetId(2);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MindmapStylePanelAction {
    Close,
    TogglePresets,
    SelectTheme(String),
}

WidgetAction::MindmapStylePanel(MindmapStylePanelAction),
```

Update both exhaustive matches in `events.rs`: treat the new action as consumed, and temporarily translate it to no `AppAction`. This temporary no-op is removed in Task 10; it keeps every intermediate commit compiling before the app action variants exist.

- [ ] **Step 4: Run UI tests and workspace compilation**

```bash
cargo test -p textora-ui --lib -- core::widget
cargo check -p textora-app
```

Expected: both commands exit 0.

- [ ] **Step 5: Commit Task 4**

```bash
git add crates/ui/src/core/widget.rs crates/app/src/events.rs
git commit -m "feat(ui): add mmap style panel actions"
```

---

### Task 5: Pure UI style panel widget

**Files:**
- Create: `crates/ui/src/widgets/mindmap_style_panel.rs`
- Modify: `crates/ui/src/widgets/mod.rs`
- Modify: `crates/ui/src/lib.rs`

**Interfaces:**
- Consumes: Task 1 schemes and selection enum; Task 4 widget action and focus ID.
- Produces: `MindmapStylePanelInput`, `MindmapStyleOption`, `MindmapStylePanelWidget`, `PANEL_WIDTH_LOGICAL`.

- [ ] **Step 1: Create the widget tests first**

Create the file with the input/action-independent layout constants and tests covering:

```rust
pub const PANEL_WIDTH_LOGICAL: f32 = 280.0;
const PANEL_PADDING_LOGICAL: f32 = 12.0;
const COLUMN_GAP_LOGICAL: f32 = 8.0;
const CARD_ASPECT_RATIO: f32 = 120.0 / 76.0;

pub use crate::core::widget::MindmapStylePanelAction;

fn laid_out_panel(selection: MindmapThemeSelection) -> MindmapStylePanelWidget {
    let mut widget = MindmapStylePanelWidget::new();
    widget.set_input(MindmapStylePanelInput::from_selection(selection, true));
    let theme = crate::theme::test_theme();
    let mut measure = crate::core::NoopMeasure;
    let mut layout = LayoutCtx {
        ui_measure: None,
        measure: &mut measure,
        theme: &theme,
        dpi: 1.0,
    };
    widget.set_rect(Rect::new(0.0, 0.0, PANEL_WIDTH_LOGICAL, 600.0), &mut layout);
    widget
}

#[test]
fn six_scheme_cards_layout_as_two_columns_and_three_rows() {
    let widget = laid_out_panel(MindmapThemeSelection::Default);
    let cards = widget.card_rects_for_test();
    assert_eq!(cards.len(), 6);
    assert_eq!(cards[0].y, cards[1].y);
    assert_eq!(cards[2].y, cards[3].y);
    assert_eq!(cards[4].y, cards[5].y);
    assert_eq!(cards[0].x, cards[2].x);
    assert_eq!(cards[1].x, cards[3].x);
    assert!(cards.iter().all(|card| card.right() <= PANEL_WIDTH_LOGICAL));
}

#[test]
fn selected_card_paints_selection_border_and_checkmark() {
    let widget = laid_out_panel(MindmapThemeSelection::Selected("tide".into()));
    assert_eq!(widget.selected_option_for_test().map(|option| option.id.as_str()), Some("tide"));
    let mut draw_list = DrawList::new();
    let theme = crate::theme::test_theme();
    widget.paint(&mut PaintCtx::new(&mut draw_list, &theme, 1.0));
    assert!(draw_list.cmds.iter().any(|command| matches!(
        command,
        DrawCmd::StrokeRect { line_width, .. } if *line_width == 2.0
    )));
}

#[test]
fn invalid_metadata_disables_card_selection() {
    let mut widget = laid_out_panel(MindmapThemeSelection::InvalidMetadata);
    let card = widget.card_rects_for_test()[0];
    let theme = crate::theme::test_theme();
    let mut event_ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
    let action = widget.on_event(
        &Event::MouseDown {
            px: card.x + card.w * 0.5,
            py: card.y + card.h * 0.5,
            button: MouseButton::Left,
        },
        &mut event_ctx,
    );
    assert_eq!(action, Some(WidgetAction::Consumed));
}

#[test]
fn card_click_emits_stable_theme_id() {
    let mut widget = laid_out_panel(MindmapThemeSelection::Default);
    let tide_index = widget.option_index_for_test("tide").expect("tide card");
    let card = widget.card_rects_for_test()[tide_index];
    let theme = crate::theme::test_theme();
    let mut event_ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
    assert_eq!(
        widget.on_event(
            &Event::MouseDown {
                px: card.x + card.w * 0.5,
                py: card.y + card.h * 0.5,
                button: MouseButton::Left,
            },
            &mut event_ctx,
        ),
        Some(WidgetAction::MindmapStylePanel(
            MindmapStylePanelAction::SelectTheme("tide".into())
        ))
    );
}

#[test]
fn arrow_keys_move_focus_and_enter_selects() {
    let mut widget = laid_out_panel(MindmapThemeSelection::Default);
    let theme = crate::theme::test_theme();
    let mut event_ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
    widget.on_event(&Event::KeyDown(KeyCode::Right, Modifiers::NONE), &mut event_ctx);
    widget.on_event(&Event::KeyDown(KeyCode::Down, Modifiers::NONE), &mut event_ctx);
    assert_eq!(
        widget.on_event(&Event::KeyDown(KeyCode::Enter, Modifiers::NONE), &mut event_ctx),
        Some(WidgetAction::MindmapStylePanel(
            MindmapStylePanelAction::SelectTheme("meadow".into())
        ))
    );
}

#[test]
fn escape_emits_close_action() {
    let mut widget = laid_out_panel(MindmapThemeSelection::Default);
    let theme = crate::theme::test_theme();
    let mut event_ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
    assert_eq!(
        widget.on_event(&Event::KeyDown(KeyCode::Escape, Modifiers::NONE), &mut event_ctx),
        Some(WidgetAction::MindmapStylePanel(MindmapStylePanelAction::Close))
    );
}
```

The test input must be built through `MindmapStylePanelInput::from_selection(selection, true)` so production and tests use the same registry mapping.

- [ ] **Step 2: Export the module and run tests to confirm missing widget implementation**

Add `pub mod mindmap_style_panel;` to `widgets/mod.rs` and re-export it from `lib.rs`. Run:

```bash
cargo test -p textora-ui --lib -- mindmap_style_panel
```

Expected: compilation fails until the widget types and `Widget` implementation are added.

- [ ] **Step 3: Implement pure input construction and layout**

Use these public types:

```rust
#[derive(Clone, Debug)]
pub struct MindmapStyleOption {
    pub id: String,
    pub display_name: String,
    pub canvas_background: [f32; 4],
    pub root_fill: [f32; 4],
    pub branch_colors: Vec<[f32; 4]>,
    pub selected: bool,
}

#[derive(Clone, Debug)]
pub struct MindmapStylePanelInput {
    pub selection: MindmapThemeSelection,
    pub options: Vec<MindmapStyleOption>,
    pub presets_expanded: bool,
}

impl MindmapStylePanelInput {
    pub fn from_selection(selection: MindmapThemeSelection, presets_expanded: bool) -> Self;
}
```

`from_selection` must map the Task 1 registry, mark exactly one card selected for `Default`/`Selected`, mark no card for `Unknown`, and disable selection for `InvalidMetadata`.

- [ ] **Step 4: Implement paint, hit testing, tooltip, and keyboard behavior**

The widget must:

- Paint app-shell background, “风格” title, close icon, separator, “配色方案”, summary strip/name/chevron, and two-column card grid.
- Paint every preview with the same fixed miniature tree geometry: one root card, two first-level cards, two second-level cards, and connecting paths; only colors vary.
- Return `TogglePresets` when the summary row is clicked, `Close` for close/Escape, and `SelectTheme(id)` for enabled card click/Enter/Space.
- Override `id()` with `ids::MINDMAP_STYLE_PANEL` and `is_focusable()` with `true`.
- Expose scheme names through `tooltip_at`; hover must only return `WidgetAction::Consumed` and never select.
- Paint unknown-ID notice `找不到主题：{id}，已使用默认主题`; paint invalid notice `请先修复文件元数据`.

- [ ] **Step 5: Run widget tests and formatting**

```bash
cargo test -p textora-ui --lib -- mindmap_style_panel
cargo fmt --all -- --check
cargo check -p textora-app
```

Expected: all commands exit 0.

- [ ] **Step 6: Commit Task 5**

```bash
git add crates/ui/src/widgets/mindmap_style_panel.rs crates/ui/src/widgets/mod.rs crates/ui/src/lib.rs
git commit -m "feat(ui): add mmap style panel widget"
```

---

### Task 5A: Add the palette icon asset

**Files:**
- Modify: `crates/ui/src/widgets/icon.rs`

**Interfaces:**
- Consumes: existing SVG icon parser and cache.
- Produces: `draw_icon(..., "palette", ...)` for the title bar.

- [ ] **Step 1: Write the failing icon registry test**

Extend the existing icon-name test:

```rust
#[test]
fn palette_icon_is_registered_and_tessellates() {
    assert!(icon_svg("palette").is_some());
    assert!(ensure_icon("palette").is_some());
}
```

- [ ] **Step 2: Run the icon test and verify failure**

```bash
cargo test -p textora-ui --lib -- icon::tests::palette_icon
```

Expected: FAIL because `icon_svg("palette")` returns `None`.

- [ ] **Step 3: Add the fixed Lucide-compatible palette geometry**

Register this icon data without adding a runtime file dependency:

```rust
const DATA_PALETTE: IconSvg = IconSvg {
    paths: &[
        "M12 22a1 1 0 0 1 0-20 10 9 0 0 1 10 9 5 5 0 0 1-5 5h-2.25a1.75 1.75 0 0 0-1.4 2.8l.3.4a1.75 1.75 0 0 1-1.4 2.8z",
        "M7.5 10.5h.01",
        "M10.5 7.5h.01",
        "M14.5 6.5h.01",
        "M17.5 9.5h.01",
    ],
    circles: &[],
    stroke_width: 2.0,
};
```

Add `"palette" => Some(&DATA_PALETTE)` to `icon_svg`.

- [ ] **Step 4: Run icon tests and compile**

```bash
cargo test -p textora-ui --lib -- icon
cargo check -p textora-app
```

Expected: both commands exit 0.

- [ ] **Step 5: Commit Task 5A**

```bash
git add crates/ui/src/widgets/icon.rs
git commit -m "feat(ui): add palette icon"
```

---

### Task 6: Title-bar style button

**Files:**
- Modify: `crates/ui/src/widgets/title_bar.rs`
- Modify: `crates/app/src/events.rs`
- Modify: `crates/app/src/app_renderer.rs`

**Interfaces:**
- Consumes: existing right-aligned title-bar actions.
- Produces: `MindmapStyleButtonInput`, `TitleBarAction::ToggleMindmapStylePanel`.

- [ ] **Step 1: Write failing title-bar tests**

Add tests for:

```rust
#[test]
fn mmap_style_button_is_left_of_view_toggle_and_does_not_overlap() {
    let widget = laid_out_title_bar(TitleBarInput {
        can_toggle: true,
        mindmap_style: Some(MindmapStyleButtonInput { panel_visible: false }),
        ..test_title_bar_input()
    });
    assert!(widget.mindmap_style_rect_for_test().right() <= widget.toggle_rect_for_test().x);
}

#[test]
fn mmap_style_button_is_absent_when_input_is_none() {
    let widget = laid_out_title_bar(TitleBarInput {
        mindmap_style: None,
        ..test_title_bar_input()
    });
    assert_eq!(widget.mindmap_style_rect_for_test(), Rect::ZERO);
}

#[test]
fn active_mmap_style_button_uses_accent_color() {
    let widget = laid_out_title_bar(TitleBarInput {
        mindmap_style: Some(MindmapStyleButtonInput { panel_visible: true }),
        ..test_title_bar_input()
    });
    let theme = test_theme();
    let mut draw_list = DrawList::new();
    widget.paint(&mut PaintCtx::new(&mut draw_list, &theme, 1.0));
    assert!(draw_list.cmds.iter().any(|command| matches!(
        command,
        DrawCmd::FillTriangle { color, .. } if *color == theme.palette.accent
    )));
}

#[test]
fn mmap_style_button_click_emits_toggle_action() {
    let mut widget = laid_out_title_bar(TitleBarInput {
        mindmap_style: Some(MindmapStyleButtonInput { panel_visible: false }),
        ..test_title_bar_input()
    });
    let rect = widget.mindmap_style_rect_for_test();
    let theme = test_theme();
    let mut event_ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
    assert_eq!(
        widget.on_event(
            &Event::MouseDown {
                px: rect.x + rect.w * 0.5,
                py: rect.y + rect.h * 0.5,
                button: MouseButton::Left,
            },
            &mut event_ctx,
        ),
        Some(WidgetAction::TitleBar(TitleBarAction::ToggleMindmapStylePanel))
    );
}
```

Add `test_title_bar_input()` and `laid_out_title_bar()` beside existing title-bar test helpers so every literal uses one baseline constructor; add `#[cfg(test)]` rect accessors rather than making layout rectangles public.

- [ ] **Step 2: Run tests and confirm missing title input**

```bash
cargo test -p textora-ui --lib -- title_bar::tests::mmap_style
```

Expected: compilation fails because `MindmapStyleButtonInput` and the action variant are absent.

- [ ] **Step 3: Add the optional style button input and layout**

Use a single optional typed field instead of `enabled` + `visible` booleans:

```rust
#[derive(Clone, Copy, Debug)]
pub struct MindmapStyleButtonInput {
    pub panel_visible: bool,
}

pub struct TitleBarInput {
    pub mindmap_style: Option<MindmapStyleButtonInput>,
}

pub enum TitleBarAction {
    ToggleView,
    ToggleToc,
    ToggleMindmapStylePanel,
}
```

Lay out the view toggle at the far right and the mmap style button immediately left of it. Preserve current TOC positioning rules and add overlap assertions. Use the `palette` icon added in Task 5A.

- [ ] **Step 4: Keep the workspace compiling before final action wiring**

In `app_renderer.rs`, initialize the new title input field as `mindmap_style: None`. In `events.rs`, add a temporary no-op match arm for `ToggleMindmapStylePanel`; Task 10 replaces it with an `AppAction`.

- [ ] **Step 5: Run title-bar tests and compile**

```bash
cargo test -p textora-ui --lib -- title_bar
cargo check -p textora-app
```

Expected: both commands exit 0.

- [ ] **Step 6: Commit Task 6**

```bash
git add crates/ui/src/widgets/title_bar.rs crates/app/src/events.rs crates/app/src/app_renderer.rs
git commit -m "feat(ui): add mmap style title button"
```

---

### Task 7: Per-tab panel session and right-side Dock child

**Files:**
- Modify: `crates/app/src/tab.rs`
- Modify: `crates/app/src/ui_shell.rs`
- Modify: `crates/app/src/app_dispatch.rs`

**Interfaces:**
- Consumes: Task 5 panel widget and width.
- Produces: `MindmapStylePanelSession`, `UiShell::set_mindmap_style_panel_input`, right-side dock thickness query.

- [ ] **Step 1: Write failing per-tab state tests**

In `tab.rs` tests, require an enum with no impossible “closed but expanded” state:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MindmapStylePanelSession {
    Closed,
    Open { presets_expanded: bool },
}

#[test]
fn style_panel_session_opens_expanded_and_closes_without_persistence() {
    let mut session = MindmapStylePanelSession::Closed;
    assert!(!session.is_visible());
    session.toggle_visibility();
    assert_eq!(session, MindmapStylePanelSession::Open { presets_expanded: true });
    session.toggle_presets();
    assert_eq!(session, MindmapStylePanelSession::Open { presets_expanded: false });
    session.close();
    assert_eq!(session, MindmapStylePanelSession::Closed);
    session.toggle_visibility();
    assert_eq!(session, MindmapStylePanelSession::Open { presets_expanded: true });
}
```

Add methods `is_visible`, `presets_expanded`, `toggle_visibility`, `close`, and `toggle_presets`. All three `DocItem` constructors must initialize `Closed`.

- [ ] **Step 2: Write failing UiShell right-dock tests**

Add tests in `ui_shell.rs` that inject a panel input and assert:

- editor width is reduced by exactly `280.0 * dpi`;
- panel rect is right of the editor, below title bar, and above status bar;
- clearing input restores the editor width;
- changing only panel input updates the existing widget in place without rebuilding unrelated widgets.

- [ ] **Step 3: Run targeted tests and verify missing session/dock support**

```bash
cargo test -p textora-app --lib -- tab::tests::style_panel_session
cargo test -p textora-app --lib -- ui_shell::tests::mindmap_style_panel
```

Expected: compilation fails because the state and setter do not exist.

- [ ] **Step 4: Implement session state and UiShell injection**

Add to `UiShell`:

```rust
mindmap_style_panel_input: Option<ui::mindmap_style_panel::MindmapStylePanelInput>,
mindmap_style_panel_thickness: f32,
last_mindmap_style_panel_thickness: f32,

pub fn set_mindmap_style_panel_input(
    &mut self,
    input: Option<ui::mindmap_style_panel::MindmapStylePanelInput>,
    dpi: f32,
);

pub(crate) fn mindmap_style_panel_thickness(&self) -> f32;
```

The setter computes `0.0` or `PANEL_WIDTH_LOGICAL * dpi`, marks `dock_dirty` only when visibility/thickness changes, and updates the live widget input otherwise. Insert the panel as `Side::Right` after status-bar allocation and before the normal scrollbar, so it occupies only the content height.

When the setter hides the panel and keyboard focus is `ids::MINDMAP_STYLE_PANEL`, reset focus to `KeyboardFocusTarget::Editor`; switching tabs or switching to source view must never leave focus pointing at a removed widget.

- [ ] **Step 5: Update projected initial plugin bounds**

Extend `projected_editor_rect` in `app_dispatch.rs` with an explicit `mindmap_style_panel_thickness: f32` parameter and call `take_right` before the normal scrollbar. `plugin_editor_rect` passes `self.ui_shell.mindmap_style_panel_thickness()`. This keeps first-frame/fresh-dock bounds aligned with actual Dock layout.

- [ ] **Step 6: Run app shell tests and compile**

```bash
cargo test -p textora-app --lib -- tab::tests
cargo test -p textora-app --lib -- ui_shell
cargo test -p textora-app --lib -- plugin_bounds
cargo check -p textora-app
```

Expected: all commands exit 0.

- [ ] **Step 7: Commit Task 7**

```bash
git add crates/app/src/tab.rs crates/app/src/ui_shell.rs crates/app/src/app_dispatch.rs
git commit -m "feat(app): dock mmap style panel per tab"
```

---

### Task 8: Preserve canvas center across viewport width changes

**Files:**
- Modify: `crates/app/src/canvas_viewport.rs`

**Interfaces:**
- Consumes: existing `CanvasViewportSession::prepare` and `CanvasViewportSnapshot` transforms.
- Produces: viewport-resize anchor preservation without changing zoom.

- [ ] **Step 1: Write the failing viewport resize test**

Add:

```rust
#[test]
fn narrower_viewport_keeps_old_center_content_at_new_center() {
    let mut session = prepared_session();
    let before = snapshot(&session);
    let old_center = CanvasPoint::new(
        before.viewport.x + before.viewport.w * 0.5,
        before.viewport.y + before.viewport.h * 0.5,
    );
    let content_anchor = before.screen_to_content(old_center);

    session.prepare(
        CanvasContentMetrics { content_bounds: before.content_bounds, focus_anchor: None },
        Rect::new(before.viewport.x, before.viewport.y, before.viewport.w - 280.0, before.viewport.h),
        CanvasViewportConfig::for_dpi(1.0),
    );

    let after = snapshot(&session);
    let new_center = CanvasPoint::new(
        after.viewport.x + after.viewport.w * 0.5,
        after.viewport.y + after.viewport.h * 0.5,
    );
    assert_point_close(after.content_to_screen(content_anchor), new_center);
    assert_eq!(after.zoom, before.zoom);
}
```

- [ ] **Step 2: Run and verify the current behavior fails**

```bash
cargo test -p textora-app --lib -- canvas_viewport::tests::narrower_viewport
```

Expected: FAIL because current `prepare` preserves only the previous scroll position when metrics are unchanged.

- [ ] **Step 3: Preserve center only when viewport changes**

In `prepare`, calculate both `metrics_changed` and `viewport_changed` before overwriting cached values. Keep existing focus-anchor behavior for content metric changes. For viewport-only changes:

1. convert the previous viewport center to a content point;
2. resolve the new unanchored snapshot;
3. call `position_for_screen_anchor` with the content point and the new viewport center;
4. resolve and store that position.

Do not use `focus_anchor` for a pure viewport resize, and do not apply the branch on the first successful prepare.

- [ ] **Step 4: Run all canvas viewport tests and compile**

```bash
cargo test -p textora-app --lib -- canvas_viewport
cargo check -p textora-app
```

Expected: all tests pass, including existing pan/zoom/metrics-anchor behavior.

- [ ] **Step 5: Commit Task 8**

```bash
git add crates/app/src/canvas_viewport.rs
git commit -m "fix(app): preserve canvas center on dock resize"
```

---

### Task 9: Assemble active mmap title and panel inputs

**Files:**
- Modify: `crates/app/src/app_renderer.rs`

**Interfaces:**
- Consumes: Task 3 query response, Task 6 title button input, Task 7 session and UiShell setter.
- Produces: active-tab-driven style panel visibility and card selection.

- [ ] **Step 1: Write failing renderer input tests**

Add app renderer tests with lightweight plugins returning `MindmapThemeSelection` and names `PLUGIN_MINDMAP`/`PLUGIN_EDITOR`:

```rust
struct ThemeQueryPlugin {
    name: &'static str,
    selection: MindmapThemeSelection,
}

impl ViewPlugin for ThemeQueryPlugin {
    fn name(&self) -> &str {
        self.name
    }

    fn render(
        &mut self,
        _doc: &dyn DocView,
        _bounds: Rect,
        _theme: &Theme,
        _shaper: &mut Shaper,
        _dpi_scale: f32,
    ) -> DrawList {
        DrawList::new()
    }

    fn query(&self, query: PluginQuery, _doc: &dyn DocView) -> PluginResponse {
        match query {
            PluginQuery::MindmapThemeSelection => {
                PluginResponse::MindmapThemeSelection(self.selection.clone())
            }
            _ => PluginResponse::None,
        }
    }
}

fn theme_query_entry(name: &'static str, selection: MindmapThemeSelection) -> DocItem {
    let doc = DocumentView::new(vec!["# Root".into()], 80, 10.0);
    DocItem::new(doc, Box::new(ThemeQueryPlugin { name, selection }))
}

#[test]
fn open_mmap_tab_builds_selected_theme_input() {
    let mut entry = theme_query_entry(
        ui::plugin::PLUGIN_MINDMAP,
        MindmapThemeSelection::Selected("tide".into()),
    );
    entry.mindmap_style_panel.toggle_visibility();
    let input = active_mindmap_style_input(Some(&entry)).expect("open mmap panel input");
    assert!(input.options.iter().any(|option| option.id == "tide" && option.selected));
}

#[test]
fn non_mmap_tab_hides_button_and_clears_right_panel() {
    let mut entry = theme_query_entry(
        ui::plugin::PLUGIN_EDITOR,
        MindmapThemeSelection::Selected("tide".into()),
    );
    entry.mindmap_style_panel.toggle_visibility();
    assert!(active_mindmap_style_input(Some(&entry)).is_none());
}

#[test]
fn switching_tabs_restores_each_style_panel_session() {
    let mut first = theme_query_entry(ui::plugin::PLUGIN_MINDMAP, MindmapThemeSelection::Default);
    let second = theme_query_entry(ui::plugin::PLUGIN_MINDMAP, MindmapThemeSelection::Default);
    first.mindmap_style_panel.toggle_visibility();
    assert!(active_mindmap_style_input(Some(&first)).is_some());
    assert!(active_mindmap_style_input(Some(&second)).is_none());
    assert!(active_mindmap_style_input(Some(&first)).is_some());
}
```

- [ ] **Step 2: Run tests and verify current inputs stay absent**

```bash
cargo test -p textora-app --lib -- app_renderer::tests::open_mmap_tab_builds_selected_theme_input
cargo test -p textora-app --lib -- app_renderer::tests::switching_tabs_restores_each_style
```

Expected: FAIL because `app_renderer` still sets `mindmap_style: None` and never injects a panel.

- [ ] **Step 3: Add one pure assembly helper**

Add a private helper that reads only the active entry:

```rust
fn active_mindmap_style_input(
    entry: Option<&DocItem>,
) -> Option<ui::mindmap_style_panel::MindmapStylePanelInput>;
```

It returns `None` unless `plugin.name() == PLUGIN_MINDMAP` and the session is open. When open, query `PluginQuery::MindmapThemeSelection`; map a missing/unexpected response to `InvalidMetadata`; pass the session’s `presets_expanded` to `MindmapStylePanelInput::from_selection`.

- [ ] **Step 4: Inject title and panel inputs before `update_frame`**

Set:

```rust
mindmap_style: is_mmap.then_some(MindmapStyleButtonInput {
    panel_visible: entry.mindmap_style_panel.is_visible(),
}),
```

Call `ui_shell.set_mindmap_style_panel_input(panel_input, dpi)` before `ui_shell.update_frame`. When source view is active, pass `None` but keep the tab session untouched so returning to mmap restores the panel.

- [ ] **Step 5: Run renderer tests and compile**

```bash
cargo test -p textora-app --lib -- app_renderer
cargo check -p textora-app
```

Expected: all commands exit 0.

- [ ] **Step 6: Commit Task 9**

```bash
git add crates/app/src/app_renderer.rs
git commit -m "feat(app): build mmap style panel inputs"
```

---

### Task 10: Route style actions and execute theme edit transactions

**Files:**
- Modify: `crates/app/src/actions.rs`
- Modify: `crates/app/src/events.rs`
- Modify: `crates/app/src/app_dispatch.rs`

**Interfaces:**
- Consumes: Task 3 plan query, Task 4 widget actions, Task 6 title action, Task 7 tab state.
- Produces: complete button/panel action routing and validated theme edits.

- [ ] **Step 1: Write failing action translation tests**

In `events.rs` tests, assert exact translations:

```rust
TitleBarAction::ToggleMindmapStylePanel
    → AppAction::ToggleMindmapStylePanel

MindmapStylePanelAction::Close
    → AppAction::MindmapStylePanel(MindmapStylePanelAction::Close)

MindmapStylePanelAction::SelectTheme("tide".into())
    → AppAction::MindmapStylePanel(MindmapStylePanelAction::SelectTheme("tide".into()))
```

Because `AppAction` currently lacks `Debug`/`PartialEq`, use `matches!` or add only the derives that all contained fields support; do not introduce custom string comparisons.

- [ ] **Step 2: Write failing app dispatch tests**

Use a test plugin that returns a deterministic `EditPlan` for `PlanMindmapTheme` and record queries. Cover:

- title button toggles `Closed ↔ Open { presets_expanded: true }`;
- panel close closes only the active tab;
- `TogglePresets` only changes the open session;
- selecting `tide` applies one edit transaction, changes source, increments generation once, and marks dirty;
- selecting the current theme consumes with no generation or dirty change;
- stale generation and non-mmap active plugin do not modify any document;
- switching between two mmap tabs applies the edit only to the active tab.

- [ ] **Step 3: Run the targeted tests and verify missing app variants**

```bash
cargo test -p textora-app --lib -- events::tests::mindmap_style
cargo test -p textora-app --lib -- app_dispatch::tests::mindmap_style
```

Expected: compilation fails because the `AppAction` variants and reducer arms do not exist.

- [ ] **Step 4: Add app intents and replace temporary no-op translations**

Add:

```rust
AppAction::ToggleMindmapStylePanel,
AppAction::MindmapStylePanel(ui::core::widget::MindmapStylePanelAction),
```

In `events.rs`, translate the title action and the unified widget action. When a style-panel mouse action is dispatched, set keyboard focus to `ids::MINDMAP_STYLE_PANEL`, matching existing SearchBar focus handling.

- [ ] **Step 5: Implement state reduction and theme application**

Add focused methods in `app_dispatch.rs`:

```rust
fn toggle_active_mindmap_style_panel(&mut self) -> AppEffect;
fn dispatch_mindmap_style_panel_action(
    &mut self,
    action: MindmapStylePanelAction,
) -> AppEffect;
fn apply_active_mindmap_theme(&mut self, theme_id: String) -> AppEffect;
```

`apply_active_mindmap_theme` must:

1. return `AppEffect::NONE` unless the active plugin is mmap;
2. capture `doc.generation()` and query `PlanMindmapTheme`;
3. accept only `PluginResponse::EditPlan`;
4. execute through `crate::edit_transaction::execute_edit_plan`;
5. call `sync_plugin_state()` only when the outcome executed text changes;
6. return `AppEffect::REDRAW` for an executed edit or panel state change, otherwise `NONE`.

Do not directly call `DocumentView::replace_range`, do not write to disk, and do not optimistically update UI selection before the transaction succeeds.

- [ ] **Step 6: Run app action and integration tests**

```bash
cargo test -p textora-app --lib -- events::tests::mindmap_style
cargo test -p textora-app --lib -- app_dispatch::tests::mindmap_style
cargo test -p textora-app --lib -- edit_transaction
cargo check -p textora-app
```

Expected: all commands exit 0.

- [ ] **Step 7: Commit Task 10**

```bash
git add crates/app/src/actions.rs crates/app/src/events.rs crates/app/src/app_dispatch.rs
git commit -m "feat(app): apply mmap theme panel actions"
```

---

### Task 11: Manual protocol, full regression suite, and final cleanup

**Files:**
- Modify: `docs/manual_test_protocol.md`

**Interfaces:**
- Consumes: complete feature from Tasks 1–10.
- Produces: repeatable manual acceptance steps and verified repository state.

- [ ] **Step 1: Add the manual mmap style-panel protocol**

Append a section with these exact scenarios and expected results:

1. Open an old mmap without `theme`: panel shows 暖夜, document remains clean.
2. Open the panel: it occupies 280 logical pixels on the right, shows two columns, and does not cover nodes.
3. Select 潮汐: canvas changes immediately, file becomes dirty, TOML gains `theme = "tide"`.
4. Save and reopen: 潮汐 remains selected.
5. Open two mmap files, choose different themes, and switch tabs: each canvas and panel selection remain independent.
6. Toggle app light/dark mode: selected mmap colors remain unchanged while panel chrome follows the app.
7. Open/close panel after pan/zoom: zoom is unchanged and the old center content point remains centered.
8. Open an unknown theme ID: default renders, warning appears, source ID is untouched.
9. Break global TOML: diagnostic canvas remains, cards are disabled, repair via source view restores selection.
10. Switch to source view while panel is open and return: panel session is restored only for that tab.

- [ ] **Step 2: Run formatting and focused crate suites**

```bash
cargo fmt --all -- --check
cargo test -p textora-ui --lib -- mindmap
cargo test -p textora-markdown --lib -- mmf
cargo test -p textora-markdown --lib -- mindmap_view
cargo test -p textora-app --lib -- mindmap_style
cargo test -p textora-app --lib -- canvas_viewport
cargo check -p textora-app
```

Expected: every command exits 0 with no failed tests or formatting diff.

- [ ] **Step 3: Run the required full verification**

```bash
./scripts/verify.sh
```

Expected: exit code 0. If it reports an unrelated pre-existing failure, record the exact failing command and output before changing any code; do not weaken or skip the check.

- [ ] **Step 4: Inspect the final diff for scope and cleanliness**

```bash
git status --short
git diff --check
git diff --stat HEAD~10..HEAD
rg -n "TODO|TBD|unwrap\(\)" crates/ui/src/widgets/mindmap_style_panel.rs crates/markdown/src/mmf crates/markdown/src/mindmap_view.rs crates/app/src
```

Expected: only planned files are changed, `git diff --check` is silent, no new placeholder appears, and any existing unrelated `unwrap()` occurrence is not modified or copied into new code.

- [ ] **Step 5: Commit Task 11**

```bash
git add docs/manual_test_protocol.md
git commit -m "docs(mmap): add theme panel manual checks"
```

---

## Final Acceptance Checklist

- [ ] 风格按钮只在 mmap 视图显示，并位于源码视图按钮左侧。
- [ ] 固定右侧面板宽度为 280 逻辑像素，缩略图为两列三行。
- [ ] 6 个稳定主题 ID、中文名称和固定配色全部可选择。
- [ ] 文件级主题通过 MMF TOML 保存，可撤销、可重做、可产生 dirty 状态。
- [ ] 缺省、未知和无效元数据状态彼此可区分，且不存在自动覆盖源文件。
- [ ] 多标签页面板会话和文件主题互不干扰。
- [ ] app 浅色/深色模式不改变已选 mmap 配色。
- [ ] 主题切换不重建布局或连接线几何缓存。
- [ ] 面板尺寸变化保持 zoom 和中心内容锚点。
- [ ] `ui` 没有新增 app/DocumentView/Workspace 依赖。
- [ ] `cargo fmt --all -- --check` 与 `./scripts/verify.sh` 均通过。
