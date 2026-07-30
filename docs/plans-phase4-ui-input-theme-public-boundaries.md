# Phase 4 UI Input / Theme Loading / Public Boundaries Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让主题注册成为 eager、纯内存且可诊断的过程，让复杂 widget 只接收单一帧输入，并把 crates/ui 的公共 API 收敛为稳定语义模块。

**Architecture:** 先把 ThemeRegistry 从超大 theme.rs 中提取为私有实现单元，再一次性切换为确定性的 eager 解析；app 负责非 fail-fast 文件读取和完整报告。随后分别迁移 TabBar、Sidebar、Scrollbar 输入，先建立兼容语义门面、分批迁移 app import，最后隐藏 widgets/theme_file/hex_color/text_renderer 并用编译测试和源码门禁封口。

**Tech Stack:** Rust 2024、winit、现有 ui::theme/UiMetrics/SidebarSettingsInput/Widget trait、toml 0.5、app/ui 单元测试与 integration tests。

## Global Constraints

- 设计依据：`docs/superpowers/specs/2026-06-20-phase4-ui-input-theme-public-boundaries-design.md`。
- 前置完成：`docs/plans-settings-dpi-remediation.md`、`docs/plans-logical-settings-physical-metrics.md`、`docs/plans-phase3-app-effect-public-boundaries.md`。
- 每个任务最多创建或修改 3 个文件；同一任务列出的 Files 是硬上限。
- 行为变化必须先写失败测试、确认失败原因，再写最小实现。
- 每次提交前至少运行受影响 crate 的定向测试和 `cargo check -p edit-plus-app`。
- ThemeRegistry 不访问文件系统、不输出日志；app 是文件 I/O、诊断保存和日志输出的唯一边界。
- Settings 保持逻辑单位；UiMetrics 只含物理布局值；SidebarSettingsInput 只含行为值。
- UI input 不得包含 App、Workspace、DocumentView、AppAction 或 AppCommand。
- 每个中间提交必须编译；不得用新增 allow 掩盖本次改动产生的 warning。
- 不实现主题热重载、递归目录、异步加载、用户可见错误面板或 ui::core 内部重构。

---

## 文件职责映射

- `crates/ui/src/theme.rs`：主题数据模型、内置主题、Theme::resolve，以及 ThemeRegistry 公共类型的语义 re-export。
- `crates/ui/src/theme_registry.rs`：ThemeRegistry 存储、eager TOML 解析、继承图、确定性诊断和不可变查询。
- `crates/ui/src/theme_file.rs`：私有 TOML schema 与局部颜色 resolve。
- `crates/app/src/theme_loader.rs`：主题目录发现、文件读取、I/O 诊断和稳定排序。
- `crates/app/src/app.rs`：保存 ThemeLoadReport。
- `crates/app/src/app_init.rs`：组合 loader/registry 报告并在初始化边界输出一次。
- `crates/ui/src/widgets/tab_bar/{types,layout,state,widget}.rs`：TabInfo 唯一 pin 真值、布局借用 view 和 owned widget input。
- `crates/ui/src/widgets/sidebar/{types,mod}.rs`：SidebarWidgetInput 与 widget 注入。
- `crates/ui/src/widgets/scrollbar.rs`：ScrollbarInput 与数值规范化。
- `crates/app/src/ui_shell.rs`：拥有三类 widget frame input 并向现存/新建 widget 注入。
- `crates/app/src/app_renderer.rs`：从 App/Workspace 构造纯 UI input。
- `crates/ui/src/lib.rs`：稳定领域模块、widget 语义门面和私有实现模块。
- `crates/ui/tests/public_api.rs`：外部 crate 视角的允许路径 compile-pass。
- `crates/ui/tests/public_boundaries.rs`：禁止公共模块、反向依赖、UI 文件 I/O、主题日志和 app 旧路径门禁。

## 跨任务接口

后续任务统一使用以下最终签名，不得另起同义名称：

~~~rust
pub fn ThemeRegistry::register_sources(
    &mut self,
    sources: impl IntoIterator<Item = ThemeSource>,
) -> ThemeRegistrationReport;

pub fn ThemeRegistry::get(&self, id: &str) -> Option<&ThemeDefinition>;
pub fn ThemeRegistry::get_or_default(&self, id: &str, prefer_dark: bool) -> &ThemeDefinition;

pub(crate) fn load_theme_sources(dir: &Path) -> ThemeSourceBatch;

pub fn TabBarWidget::set_input(
    &mut self,
    input: TabBarWidgetInput,
    shaper: Option<&mut shaping::Shaper>,
);

pub fn SidebarWidget::set_input(&mut self, input: SidebarWidgetInput);
pub fn ScrollbarWidget::set_input(&mut self, input: ScrollbarInput);
~~~

## Phase A：ThemeRegistry 与 app 主题边界

### Task 1: 将 ThemeRegistry 提取为私有实现文件

**Files:**
- Create/Test: `crates/ui/src/theme_registry.rs`
- Modify/Test: `crates/ui/src/theme.rs`
- Modify: `crates/ui/src/lib.rs`

**Interfaces:**
- Consumes: 当前 `theme.rs` 中 ThemeSource、PendingTheme、ThemeRegistry、BUILTIN_*、LoadError、RegisterError 及五个 registry 测试。
- Produces: `crate::theme_registry` 私有模块；`ui::theme::{ThemeRegistry, ThemeSource, LoadError, RegisterError, BUILTIN_DARK_ID, BUILTIN_LIGHT_ID}` 路径暂时保持完全兼容。

- [ ] **Step 1: 记录移动前基线测试**

~~~bash
cargo test -p edit-plus-ui --lib theme::tests -- --nocapture
~~~

Expected: PASS；记录现有 5 个 registry 测试均执行。

- [ ] **Step 2: 在 lib.rs 声明私有实现模块**

在 `pub mod theme;` 相邻位置增加：

~~~rust
mod theme_registry;
~~~

不要公开 `theme_registry`。

- [ ] **Step 3: 原样移动 Registry 实现与测试**

把以下完整项目从 `theme.rs` 移到 `theme_registry.rs`：

~~~text
ThemeSource
PendingTheme
ThemeRegistry 及其 impl
BUILTIN_DARK_ID / BUILTIN_LIGHT_ID
LoadError 及 Display impl
RegisterError
register_invalid_hex
user_theme_extends_another_user_theme
user_theme_extends_chain_order_independent
invalid_hex_in_loaded_file_errors_at_get_time
empty_file_loads_with_defaults
~~~

`theme_registry.rs` 顶部使用：

~~~rust
use std::collections::HashMap;

use crate::theme::ThemeDefinition;
~~~

测试模块改为 `use super::*;`。`theme.rs` 在 ActiveThemePair 后增加唯一兼容出口：

~~~rust
pub use crate::theme_registry::{
    BUILTIN_DARK_ID, BUILTIN_LIGHT_ID, LoadError, RegisterError, ThemeRegistry, ThemeSource,
};
~~~

同时从 `theme.rs` 顶部删除不再使用的 HashMap import；Theme.scopes 仍需 HashMap，因此保留 `use std::collections::{BTreeMap, HashMap};`。

- [ ] **Step 4: 验证纯移动不改变行为**

~~~bash
cargo test -p edit-plus-ui --lib theme_registry::tests -- --nocapture
cargo check -p edit-plus-app
~~~

Expected: PASS；app 仍通过 `ui::theme::*` 编译。

- [ ] **Step 5: 提交**

~~~bash
git add crates/ui/src/theme_registry.rs crates/ui/src/theme.rs crates/ui/src/lib.rs
git commit -m "refactor(ui): isolate theme registry implementation"
~~~

### Task 2: 用确定性 eager 解析替换 pending/lazy 查询

**Files:**
- Modify/Test: `crates/ui/src/theme_registry.rs`
- Modify: `crates/ui/src/theme.rs`

**Interfaces:**
- Consumes: Task 1 的私有 `theme_registry.rs`、`ThemeFile::resolve(&ThemeDefinition)`。
- Produces: `ThemeLoadError`、`ThemeRegistrationReport`、不可变 get/get_or_default、无 pending 的 ThemeRegistry。

- [ ] **Step 1: 写 eager、隔离失败与不可变查询失败测试**

删除旧的 `invalid_hex_in_loaded_file_errors_at_get_time`，加入：

~~~rust
#[test]
fn register_sources_resolves_every_source_before_returning() {
    let mut registry = ThemeRegistry::new();
    let report = registry.register_sources([
        source("good", "z-good.toml", "is_dark = false\n"),
        source("bad", "a-bad.toml", "[palette]\naccent = \"not-hex\"\n"),
    ]);

    assert_eq!(report.registered_ids, vec!["good"]);
    assert!(matches!(
        report.errors.as_slice(),
        [ThemeLoadError::Resolve { id, .. }] if id == "bad"
    ));
    assert!(registry.get("good").is_some());
    assert!(registry.get("bad").is_none());
}

#[test]
fn get_is_an_immutable_side_effect_free_query() {
    fn query<'a>(registry: &'a ThemeRegistry, id: &str) -> Option<&'a ThemeDefinition> {
        registry.get(id)
    }

    let registry = ThemeRegistry::new();
    assert!(query(&registry, BUILTIN_DARK_ID).is_some());
    assert!(query(&registry, "missing").is_none());
}

#[test]
fn unrelated_valid_theme_survives_unknown_base() {
    let mut registry = ThemeRegistry::new();
    let report = registry.register_sources([
        source("broken", "a.toml", "extends = \"missing\"\n"),
        source("valid", "b.toml", "is_dark = true\n"),
    ]);

    assert_eq!(report.registered_ids, vec!["valid"]);
    assert!(matches!(
        report.errors.as_slice(),
        [ThemeLoadError::UnknownExtends { id, base_id, .. }]
            if id == "broken" && base_id == "missing"
    ));
}
~~~

增加测试 helper：

~~~rust
fn source(id: &str, path: &str, content: &str) -> ThemeSource {
    ThemeSource { id: id.into(), path: path.into(), content: content.into() }
}
~~~

- [ ] **Step 2: 运行测试并确认 lazy 契约失败**

~~~bash
cargo test -p edit-plus-ui --lib theme_registry::tests::register_sources_resolves_every_source_before_returning -- --exact
cargo test -p edit-plus-ui --lib theme_registry::tests::get_is_an_immutable_side_effect_free_query -- --exact
~~~

Expected: FAIL；register_sources 仍返回 Vec<LoadError>，get 仍要求 `&mut self`。

- [ ] **Step 3: 定义最终数据与错误类型**

用以下结构替换 PendingTheme、LoadError 和旧 Registry 字段：

~~~rust
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::theme::ThemeDefinition;
use crate::theme_file::ThemeFile;

#[derive(Debug, Clone)]
pub struct ThemeSource {
    pub id: String,
    pub path: PathBuf,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThemeLoadError {
    ReservedId { id: String, path: PathBuf },
    DuplicateId { id: String, first_path: Option<PathBuf>, duplicate_path: PathBuf },
    TomlParse { id: String, path: PathBuf, message: String },
    UnknownExtends { id: String, path: PathBuf, base_id: String },
    CyclicExtends { ids: Vec<String> },
    BaseThemeFailed { id: String, path: PathBuf, base_id: String },
    Resolve { id: String, path: PathBuf, message: String },
}

impl std::fmt::Display for ThemeLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReservedId { id, path } =>
                write!(f, "{}: reserved theme id {id}", path.display()),
            Self::DuplicateId { id, first_path, duplicate_path } => match first_path {
                Some(first) => write!(
                    f,
                    "{}: duplicate theme id {id}; first declared at {}",
                    duplicate_path.display(),
                    first.display(),
                ),
                None => write!(f, "{}: theme id {id} is already registered", duplicate_path.display()),
            },
            Self::TomlParse { id, path, message } =>
                write!(f, "{}: failed to parse theme {id}: {message}", path.display()),
            Self::UnknownExtends { id, path, base_id } =>
                write!(f, "{}: theme {id} extends unknown theme {base_id}", path.display()),
            Self::CyclicExtends { ids } =>
                write!(f, "cyclic theme inheritance: {}", ids.join(" -> ")),
            Self::BaseThemeFailed { id, path, base_id } =>
                write!(f, "{}: theme {id} depends on failed theme {base_id}", path.display()),
            Self::Resolve { id, path, message } =>
                write!(f, "{}: failed to resolve theme {id}: {message}", path.display()),
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ThemeRegistrationReport {
    pub registered_ids: Vec<String>,
    pub errors: Vec<ThemeLoadError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisterError {
    ReservedId(String),
    DuplicateId(String),
}

#[derive(Debug, Clone)]
pub struct ThemeRegistry {
    themes: BTreeMap<String, ThemeDefinition>,
    default_dark: ThemeDefinition,
    default_light: ThemeDefinition,
}

struct ParsedTheme {
    path: PathBuf,
    file: ThemeFile,
    base_id: String,
}

#[derive(Clone)]
enum ResolveState {
    Visiting,
    Resolved(ThemeDefinition),
    Failed,
}
~~~

保留 `pub type LoadError = ThemeLoadError;` 一个兼容周期供 app 编译；Task 5 更新 app 后不再有消费者，最终 Task 20 删除该 alias。

同时把 `theme.rs` 的 re-export 改为：

~~~rust
pub use crate::theme_registry::{
    BUILTIN_DARK_ID, BUILTIN_LIGHT_ID, LoadError, RegisterError, ThemeLoadError,
    ThemeRegistrationReport, ThemeRegistry, ThemeSource,
};
~~~

- [ ] **Step 4: 实现 eager 收集、解析和查询**

`register`、查询和基础集合 API 使用：

~~~rust
pub fn register(&mut self, id: String, def: ThemeDefinition) -> Result<(), RegisterError> {
    if matches!(id.as_str(), BUILTIN_DARK_ID | BUILTIN_LIGHT_ID) {
        return Err(RegisterError::ReservedId(id));
    }
    if self.themes.contains_key(&id) {
        return Err(RegisterError::DuplicateId(id));
    }
    self.themes.insert(id, def);
    Ok(())
}

pub fn get(&self, id: &str) -> Option<&ThemeDefinition> {
    match id {
        BUILTIN_DARK_ID => Some(&self.default_dark),
        BUILTIN_LIGHT_ID => Some(&self.default_light),
        _ => self.themes.get(id),
    }
}

pub fn get_or_default(&self, id: &str, prefer_dark: bool) -> &ThemeDefinition {
    self.get(id).unwrap_or(if prefer_dark { &self.default_dark } else { &self.default_light })
}

pub fn list_ids(&self) -> Vec<String> {
    let mut ids = vec![BUILTIN_DARK_ID.to_owned(), BUILTIN_LIGHT_ID.to_owned()];
    ids.extend(self.themes.keys().cloned());
    ids.sort();
    ids
}

pub fn len(&self) -> usize { self.themes.len() }
pub fn is_empty(&self) -> bool { self.themes.is_empty() }
pub fn clear_user_themes(&mut self) { self.themes.clear(); }
~~~

`register_sources` 必须按下列完整流程实现：

~~~rust
pub fn register_sources(
    &mut self,
    sources: impl IntoIterator<Item = ThemeSource>,
) -> ThemeRegistrationReport {
    let mut sources: Vec<_> = sources.into_iter().collect();
    sources.sort_by(|a, b| (&a.path, &a.id).cmp(&(&b.path, &b.id)));

    let mut report = ThemeRegistrationReport::default();
    let mut accepted = BTreeMap::<String, ThemeSource>::new();
    for source in sources {
        if matches!(source.id.as_str(), BUILTIN_DARK_ID | BUILTIN_LIGHT_ID) {
            report.errors.push(ThemeLoadError::ReservedId {
                id: source.id,
                path: source.path,
            });
            continue;
        }
        if let Some(first) = accepted.get(&source.id) {
            report.errors.push(ThemeLoadError::DuplicateId {
                id: source.id,
                first_path: Some(first.path.clone()),
                duplicate_path: source.path,
            });
            continue;
        }
        if self.themes.contains_key(&source.id) {
            report.errors.push(ThemeLoadError::DuplicateId {
                id: source.id,
                first_path: None,
                duplicate_path: source.path,
            });
            continue;
        }
        accepted.insert(source.id.clone(), source);
    }

    let mut parsed = BTreeMap::new();
    let mut failed_ids = BTreeSet::new();
    for (id, source) in accepted {
        match toml::from_str::<ThemeFile>(&source.content) {
            Ok(file) => {
                let base_id = file.extends.clone().unwrap_or_else(|| {
                    if file.is_dark.unwrap_or(true) { BUILTIN_DARK_ID } else { BUILTIN_LIGHT_ID }
                        .to_owned()
                });
                parsed.insert(id, ParsedTheme { path: source.path, file, base_id });
            }
            Err(error) => {
                failed_ids.insert(id.clone());
                report.errors.push(ThemeLoadError::TomlParse {
                    id,
                    path: source.path,
                    message: error.to_string(),
                });
            }
        }
    }

    let ids: Vec<_> = parsed.keys().cloned().collect();
    let mut states = BTreeMap::new();
    let mut cycle_members = BTreeSet::new();
    for id in ids {
        let mut stack = Vec::new();
        if let Some(definition) = resolve_theme(
            &id,
            &parsed,
            &self.themes,
            &self.default_dark,
            &self.default_light,
            &failed_ids,
            &mut states,
            &mut stack,
            &mut cycle_members,
            &mut report.errors,
        ) {
            self.themes.insert(id.clone(), definition);
            report.registered_ids.push(id);
        }
    }

    report.registered_ids.sort();
    sort_errors(&mut report.errors, &parsed);
    report
}
~~~

`resolve_theme` 使用以下完整骨架；四个查找顺序不得调整：

~~~rust
#[allow(clippy::too_many_arguments)]
fn resolve_theme(
    id: &str,
    parsed: &BTreeMap<String, ParsedTheme>,
    existing: &BTreeMap<String, ThemeDefinition>,
    default_dark: &ThemeDefinition,
    default_light: &ThemeDefinition,
    failed_ids: &BTreeSet<String>,
    states: &mut BTreeMap<String, ResolveState>,
    stack: &mut Vec<String>,
    cycle_members: &mut BTreeSet<String>,
    errors: &mut Vec<ThemeLoadError>,
) -> Option<ThemeDefinition> {
    if id == BUILTIN_DARK_ID { return Some(default_dark.clone()); }
    if id == BUILTIN_LIGHT_ID { return Some(default_light.clone()); }
    if let Some(definition) = existing.get(id) { return Some(definition.clone()); }
    match states.get(id) {
        Some(ResolveState::Resolved(definition)) => return Some(definition.clone()),
        Some(ResolveState::Failed) => return None,
        Some(ResolveState::Visiting) => {
            let start = stack.iter().position(|entry| entry == id).unwrap();
            let mut raw = stack[start..].to_vec();
            raw.push(id.to_owned());
            let cycle = canonical_cycle(raw);
            for member in &cycle[..cycle.len() - 1] {
                cycle_members.insert(member.clone());
                states.insert(member.clone(), ResolveState::Failed);
            }
            if !errors.iter().any(|error| matches!(
                error,
                ThemeLoadError::CyclicExtends { ids } if ids == &cycle
            )) {
                errors.push(ThemeLoadError::CyclicExtends { ids: cycle });
            }
            return None;
        }
        None => {}
    }

    let theme = parsed.get(id)?;
    states.insert(id.to_owned(), ResolveState::Visiting);
    stack.push(id.to_owned());

    let base = if failed_ids.contains(&theme.base_id) {
        None
    } else if theme.base_id == BUILTIN_DARK_ID {
        Some(default_dark.clone())
    } else if theme.base_id == BUILTIN_LIGHT_ID {
        Some(default_light.clone())
    } else if let Some(definition) = existing.get(&theme.base_id) {
        Some(definition.clone())
    } else if parsed.contains_key(&theme.base_id) {
        resolve_theme(
            &theme.base_id, parsed, existing, default_dark, default_light, failed_ids,
            states, stack, cycle_members, errors,
        )
    } else {
        errors.push(ThemeLoadError::UnknownExtends {
            id: id.to_owned(),
            path: theme.path.clone(),
            base_id: theme.base_id.clone(),
        });
        None
    };

    stack.pop();
    let Some(base) = base else {
        if !cycle_members.contains(id)
            && !errors.iter().any(|error| matches!(
                error,
                ThemeLoadError::UnknownExtends { id: failed, .. } if failed == id
            ))
        {
            errors.push(ThemeLoadError::BaseThemeFailed {
                id: id.to_owned(),
                path: theme.path.clone(),
                base_id: theme.base_id.clone(),
            });
        }
        states.insert(id.to_owned(), ResolveState::Failed);
        return None;
    };

    match theme.file.resolve(&base) {
        Ok(mut definition) => {
            if theme.file.display_name.is_none() && definition.display_name == base.display_name {
                definition.display_name = id.to_owned();
            }
            states.insert(id.to_owned(), ResolveState::Resolved(definition.clone()));
            Some(definition)
        }
        Err(error) => {
            errors.push(ThemeLoadError::Resolve {
                id: id.to_owned(),
                path: theme.path.clone(),
                message: error.to_string(),
            });
            states.insert(id.to_owned(), ResolveState::Failed);
            None
        }
    }
}
~~~

规范化环实现：

~~~rust
fn canonical_cycle(mut ids: Vec<String>) -> Vec<String> {
    ids.pop();
    let start = ids.iter().enumerate().min_by_key(|(_, id)| *id).map(|(i, _)| i).unwrap();
    ids.rotate_left(start);
    ids.push(ids[0].clone());
    ids
}
~~~

`sort_errors` 为每个 variant 返回 `(PathBuf, u8, String, String)`；variant 序号严格为 enum 声明顺序。实现：

~~~rust
fn sort_errors(errors: &mut [ThemeLoadError], parsed: &BTreeMap<String, ParsedTheme>) {
    errors.sort_by_key(|error| {
        let (path, kind, id) = match error {
            ThemeLoadError::ReservedId { id, path } => (path.clone(), 0, id.clone()),
            ThemeLoadError::DuplicateId { id, duplicate_path, .. } =>
                (duplicate_path.clone(), 1, id.clone()),
            ThemeLoadError::TomlParse { id, path, .. } => (path.clone(), 2, id.clone()),
            ThemeLoadError::UnknownExtends { id, path, .. } => (path.clone(), 3, id.clone()),
            ThemeLoadError::CyclicExtends { ids } => (
                parsed.get(&ids[0]).unwrap().path.clone(),
                4,
                ids[0].clone(),
            ),
            ThemeLoadError::BaseThemeFailed { id, path, .. } => (path.clone(), 5, id.clone()),
            ThemeLoadError::Resolve { id, path, .. } => (path.clone(), 6, id.clone()),
        };
        (path, kind, id, error.to_string())
    });
}
~~~

- [ ] **Step 5: 补全继承、环、重复与确定性测试**

加入以下用例，断言必须使用完整 variant 字段：

~~~rust
#[test]
fn inheritance_is_independent_of_source_order() {
    let mut registry = ThemeRegistry::new();
    let report = registry.register_sources([
        source("derived", "a.toml", "extends = \"base\"\n[editor]\ncursor = \"#00FF00\"\n"),
        source("base", "z.toml", "is_dark = true\n[palette]\naccent = \"#FF0000\"\n"),
    ]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.registered_ids, vec!["base", "derived"]);
    assert_eq!(registry.get("derived").unwrap().editor.cursor, [0.0, 1.0, 0.0, 1.0]);
}

#[test]
fn cycle_is_canonical_and_dependent_reports_base_failure() {
    let mut registry = ThemeRegistry::new();
    let report = registry.register_sources([
        source("c", "c.toml", "extends = \"a\"\n"),
        source("b", "b.toml", "extends = \"a\"\n"),
        source("a", "a.toml", "extends = \"b\"\n"),
    ]);
    assert!(report.errors.iter().any(|error| matches!(
        error,
        ThemeLoadError::CyclicExtends { ids } if ids == &["a", "b", "a"]
    )));
    assert!(report.errors.iter().any(|error| matches!(
        error,
        ThemeLoadError::BaseThemeFailed { id, base_id, .. }
            if id == "c" && base_id == "a"
    )));
    assert!(registry.is_empty());
}

#[test]
fn duplicate_first_source_wins_even_when_it_is_invalid() {
    let mut registry = ThemeRegistry::new();
    let report = registry.register_sources([
        source("same", "a.toml", "not = [valid"),
        source("same", "b.toml", "is_dark = true\n"),
    ]);
    assert!(report.errors.iter().any(|e| matches!(e, ThemeLoadError::DuplicateId { .. })));
    assert!(report.errors.iter().any(|e| matches!(e, ThemeLoadError::TomlParse { .. })));
    assert!(registry.get("same").is_none());
}
~~~

加入以下剩余契约测试：

~~~rust
#[test]
fn reserved_and_existing_ids_are_rejected_without_overwrite() {
    let mut registry = ThemeRegistry::new();
    registry.register("existing".into(), ThemeDefinition::default_dark()).unwrap();
    let report = registry.register_sources([
        source(BUILTIN_DARK_ID, "a.toml", "is_dark = true\n"),
        source("existing", "b.toml", "is_dark = false\n"),
    ]);
    assert!(matches!(
        &report.errors[0],
        ThemeLoadError::ReservedId { id, .. } if id == BUILTIN_DARK_ID
    ));
    assert!(matches!(
        &report.errors[1],
        ThemeLoadError::DuplicateId { id, first_path: None, .. } if id == "existing"
    ));
    assert!(registry.get("existing").unwrap().is_dark);
}

#[test]
fn clear_allows_same_user_id_to_register_again() {
    let mut registry = ThemeRegistry::new();
    assert_eq!(registry.register_sources([source("user", "a.toml", "")]).registered_ids, vec!["user"]);
    assert_eq!(registry.len(), 1);
    registry.clear_user_themes();
    assert!(registry.is_empty());
    assert_eq!(registry.register_sources([source("user", "b.toml", "")]).registered_ids, vec!["user"]);
}

#[test]
fn empty_batch_and_unknown_fallback_are_stable() {
    let mut registry = ThemeRegistry::new();
    assert_eq!(registry.register_sources(Vec::<ThemeSource>::new()), ThemeRegistrationReport::default());
    assert!(registry.get_or_default("missing", true).is_dark);
    assert!(!registry.get_or_default("missing", false).is_dark);
    assert_eq!(
        registry.list_ids(),
        vec![BUILTIN_DARK_ID.to_owned(), BUILTIN_LIGHT_ID.to_owned()]
    );
}

#[test]
fn repeated_batches_produce_identically_ordered_errors() {
    let make_report = || {
        let mut registry = ThemeRegistry::new();
        registry.register_sources([
            source("z", "z.toml", "extends = \"missing\"\n"),
            source("a", "a.toml", "not = [valid"),
        ])
    };
    assert_eq!(make_report().errors, make_report().errors);
}
~~~

- [ ] **Step 6: 运行全部 Registry/UI 测试和静态检查**

~~~bash
cargo test -p edit-plus-ui --lib theme_registry::tests -- --nocapture
rg -n "pending|load_pending|eprintln!|std::io" crates/ui/src/theme_registry.rs
cargo check -p edit-plus-app
~~~

Expected: 测试 PASS；扫描无输出；app 编译通过。

- [ ] **Step 7: 提交**

~~~bash
git add crates/ui/src/theme_registry.rs crates/ui/src/theme.rs
git commit -m "refactor(ui): eagerly resolve theme registry"
~~~

### Task 3: 让 Theme::resolve 只接受不可变 Registry

**Files:**
- Modify/Test: `crates/ui/src/theme.rs`
- Modify: `crates/app/src/app_window.rs`
- Modify: `crates/app/src/dispatch/chrome.rs`

**Interfaces:**
- Consumes: Task 2 的 `ThemeRegistry::get_or_default(&self, id, prefer_dark)`；Phase 3 后主题 action 位于 `dispatch/chrome.rs`。
- Produces: 本任务 Step 3 所列的完整 `Theme::resolve` 签名；所有运行时主题切换不再要求可变 Registry。

- [ ] **Step 1: 写不可变 resolve 编译测试**

在 `theme.rs` 测试模块增加：

~~~rust
#[test]
fn resolve_accepts_immutable_registry() {
    let registry = ThemeRegistry::new();
    let pair = ActiveThemePair::default();
    let theme = Theme::resolve(
        crate::settings::ThemeMode::Dark,
        winit::window::Theme::Light,
        &pair,
        &registry,
    );
    assert!(theme.is_dark);
}
~~~

- [ ] **Step 2: 运行测试并确认签名仍为 mutable**

~~~bash
cargo test -p edit-plus-ui --lib theme::tests::resolve_accepts_immutable_registry -- --exact
~~~

Expected: FAIL，类型不匹配，函数要求 `&mut ThemeRegistry`。

- [ ] **Step 3: 修改签名和两个运行时调用点**

最终签名：

~~~rust
pub fn resolve(
    mode: crate::settings::ThemeMode,
    system_theme: winit::window::Theme,
    pair: &ActiveThemePair,
    registry: &ThemeRegistry,
) -> Self
~~~

`app_window.rs` 与 `dispatch/chrome.rs` 的 Theme::resolve 最后一个实参统一为：

~~~rust
&self.theme_registry
~~~

不得为查询创建 clone 或局部 mut Registry。

- [ ] **Step 4: 运行主题测试和 app 编译**

~~~bash
cargo test -p edit-plus-ui --lib theme::tests -- --nocapture
cargo test -p edit-plus-app --lib -- theme
cargo check -p edit-plus-app
~~~

Expected: PASS。

- [ ] **Step 5: 提交**

~~~bash
git add crates/ui/src/theme.rs crates/app/src/app_window.rs crates/app/src/dispatch/chrome.rs
git commit -m "refactor(theme): make resolution side effect free"
~~~

### Task 4: 将 app theme_loader 改为非 fail-fast batch

**Files:**
- Modify/Test: `crates/app/src/theme_loader.rs`

**Interfaces:**
- Consumes: `ui::theme::ThemeSource`。
- Produces: `ThemeSourceBatch`、`ThemeSourceDiagnostic`、`load_theme_sources(&Path) -> ThemeSourceBatch`。

- [ ] **Step 1: 写批量成功、单文件失败和保留 ID 失败测试**

用以下测试替换旧 `test_load_theme_sources`：

~~~rust
#[test]
fn loads_sorted_sources_and_keeps_reserved_ids() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("z.toml"), "is_dark = true\n").unwrap();
    fs::write(dir.path().join("a.toml"), "is_dark = false\n").unwrap();
    fs::write(dir.path().join("default-dark.toml"), "is_dark = true\n").unwrap();
    fs::write(dir.path().join("ignore.txt"), "ignored").unwrap();

    let batch = load_theme_source_batch(dir.path());
    assert!(batch.diagnostics.is_empty());
    assert_eq!(
        batch.sources.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        vec!["a", "default-dark", "z"]
    );
}

#[test]
fn invalid_utf8_file_does_not_block_valid_source() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("a-bad.toml"), [0xff, 0xfe]).unwrap();
    fs::write(dir.path().join("z-good.toml"), "is_dark = true\n").unwrap();

    let batch = load_theme_source_batch(dir.path());
    assert_eq!(batch.sources.len(), 1);
    assert_eq!(batch.sources[0].id, "z-good");
    assert!(matches!(
        batch.diagnostics.as_slice(),
        [ThemeSourceDiagnostic::FileRead { path, .. }]
            if path.ends_with("a-bad.toml")
    ));
}

#[test]
fn missing_directory_is_empty_without_diagnostic() {
    let dir = tempdir().unwrap();
    let batch = load_theme_source_batch(&dir.path().join("missing"));
    assert!(batch.sources.is_empty());
    assert!(batch.diagnostics.is_empty());
}

#[test]
fn non_directory_path_reports_directory_read() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("not-a-directory");
    fs::write(&file, "content").unwrap();
    let batch = load_theme_source_batch(&file);
    assert!(batch.sources.is_empty());
    assert!(matches!(
        batch.diagnostics.as_slice(),
        [ThemeSourceDiagnostic::DirectoryRead { path, .. }] if path == &file
    ));
}

#[cfg(unix)]
#[test]
fn invalid_utf8_file_name_is_reported_without_stopping() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let dir = tempdir().unwrap();
    let invalid = dir.path().join(OsString::from_vec(vec![0xff, b'.', b't', b'o', b'm', b'l']));
    fs::write(&invalid, "is_dark = true\n").unwrap();
    fs::write(dir.path().join("valid.toml"), "is_dark = true\n").unwrap();
    let batch = load_theme_source_batch(dir.path());
    assert_eq!(batch.sources.iter().map(|source| source.id.as_str()).collect::<Vec<_>>(), vec!["valid"]);
    assert!(batch.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic,
        ThemeSourceDiagnostic::InvalidFileName { path } if path == &invalid
    )));
}

#[test]
fn entry_error_is_retained() {
    let dir = Path::new("themes");
    let mut diagnostics = Vec::new();
    let paths = collect_entry_paths(
        dir,
        vec![Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied"))],
        &mut diagnostics,
    );
    assert!(paths.is_empty());
    assert!(matches!(
        diagnostics.as_slice(),
        [ThemeSourceDiagnostic::EntryRead { directory, message }]
            if directory == dir && message.contains("denied")
    ));
}
~~~

- [ ] **Step 2: 运行测试并确认旧 Result API 失败**

~~~bash
cargo test -p edit-plus-app --lib theme_loader::tests -- --nocapture
~~~

Expected: FAIL，ThemeSourceBatch/diagnostic 不存在，且旧 loader fail-fast。

- [ ] **Step 3: 定义 batch、diagnostic 和确定性排序**

~~~rust
#[derive(Debug, Default)]
pub(crate) struct ThemeSourceBatch {
    pub(crate) sources: Vec<ThemeSource>,
    pub(crate) diagnostics: Vec<ThemeSourceDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ThemeSourceDiagnostic {
    DirectoryRead { path: PathBuf, message: String },
    EntryRead { directory: PathBuf, message: String },
    InvalidFileName { path: PathBuf },
    FileRead { path: PathBuf, message: String },
}
~~~

增加 `ThemeSourceDiagnostic::sort_key() -> (u8, PathBuf, String)`；序号依次为 0..=3，路径字段按 variant 取 path/directory，消息缺失时为空串。

`collect_entry_paths` 精确签名：

~~~rust
fn collect_entry_paths<I>(
    dir: &Path,
    entries: I,
    diagnostics: &mut Vec<ThemeSourceDiagnostic>,
) -> Vec<PathBuf>
where
    I: IntoIterator<Item = io::Result<fs::DirEntry>>,
~~~

每个 entry 按以下分支处理；返回路径最后 `sort()`：

~~~rust
match entry {
    Ok(entry) => match entry.file_type() {
        Ok(kind)
            if kind.is_file()
                && entry.path().extension().is_some_and(|extension| extension == "toml") =>
        {
            paths.push(entry.path());
        }
        Ok(_) => {}
        Err(error) => diagnostics.push(ThemeSourceDiagnostic::EntryRead {
            directory: dir.to_owned(),
            message: error.to_string(),
        }),
    },
    Err(error) => diagnostics.push(ThemeSourceDiagnostic::EntryRead {
        directory: dir.to_owned(),
        message: error.to_string(),
    }),
}
~~~

- [ ] **Step 4: 实现不提前返回的 loader**

~~~rust
pub(crate) fn load_theme_sources(dir: &Path) -> ThemeSourceBatch {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return ThemeSourceBatch::default(),
        Err(error) => return ThemeSourceBatch {
            sources: Vec::new(),
            diagnostics: vec![ThemeSourceDiagnostic::DirectoryRead {
                path: dir.to_owned(),
                message: error.to_string(),
            }],
        },
    };

    let mut batch = ThemeSourceBatch::default();
    let paths = collect_entry_paths(dir, entries, &mut batch.diagnostics);
    for path in paths {
        let Some(id) = path.file_stem().and_then(|stem| stem.to_str()).map(str::to_owned) else {
            batch.diagnostics.push(ThemeSourceDiagnostic::InvalidFileName { path });
            continue;
        };
        match fs::read_to_string(&path) {
            Ok(content) => batch.sources.push(ThemeSource { id, path, content }),
            Err(error) => batch.diagnostics.push(ThemeSourceDiagnostic::FileRead {
                path,
                message: error.to_string(),
            }),
        }
    }
    batch.diagnostics.sort_by_key(ThemeSourceDiagnostic::sort_key);
    batch
}
~~~

- [ ] **Step 5: 运行 loader 测试与编译**

~~~bash
cargo test -p edit-plus-app --lib theme_loader::tests -- --nocapture
cargo check -p edit-plus-app
~~~

Expected: loader 测试 PASS；app_init 因旧 Result match 编译失败是不可接受的，因此在本任务提交前临时把 app_init 的调用改为读取 `.sources` 会超过文件上限。为保持提交可编译，本任务先保留兼容包装：

~~~rust
pub(crate) fn load_theme_source_batch(dir: &Path) -> ThemeSourceBatch {
    // 上述最终实现
}

pub(crate) fn load_theme_sources(dir: &Path) -> io::Result<Vec<ThemeSource>> {
    let batch = load_theme_source_batch(dir);
    if batch.diagnostics.is_empty() {
        Ok(batch.sources)
    } else {
        Err(io::Error::other(format!("{:?}", batch.diagnostics)))
    }
}
~~~

测试调用 `load_theme_source_batch`。Task 5 原子迁移 app_init 后删除兼容包装并把 batch 函数重命名为最终 `load_theme_sources`。

- [ ] **Step 6: 提交**

~~~bash
git add crates/app/src/theme_loader.rs
git commit -m "refactor(app): collect theme source diagnostics"
~~~

### Task 5: 在 App 初始化边界汇总并保存 ThemeLoadReport

**Files:**
- Modify/Test: `crates/app/src/theme_loader.rs`
- Modify: `crates/app/src/app.rs`
- Modify/Test: `crates/app/src/app_init.rs`

**Interfaces:**
- Consumes: Task 4 ThemeSourceBatch；Task 2 ThemeRegistrationReport。
- Produces: `ThemeLoadReport`、`App.theme_load_report`；最终 `load_theme_sources(&Path) -> ThemeSourceBatch`。

- [ ] **Step 1: 写合并报告失败测试**

在 `app_init.rs` 的测试模块增加一个不构造 GPU/window 的纯 helper 测试：

~~~rust
#[test]
fn build_theme_registry_retains_source_and_registry_diagnostics() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a-invalid.toml"), "not = [valid").unwrap();
    std::fs::write(dir.path().join("b-good.toml"), "is_dark = true\n").unwrap();
    std::fs::write(dir.path().join("default-dark.toml"), "is_dark = true\n").unwrap();

    let (registry, report) = load_user_themes(dir.path());

    assert!(registry.get("b-good").is_some());
    assert_eq!(report.registered_ids, vec!["b-good"]);
    assert!(report.registry_errors.iter().any(|error| matches!(
        error,
        ui::theme::ThemeLoadError::TomlParse { id, .. } if id == "a-invalid"
    )));
    assert!(report.registry_errors.iter().any(|error| matches!(
        error,
        ui::theme::ThemeLoadError::ReservedId { id, .. } if id == "default-dark"
    )));
}
~~~

- [ ] **Step 2: 运行测试并确认报告/helper 不存在**

~~~bash
cargo test -p edit-plus-app --lib app_init::tests::build_theme_registry_retains_source_and_registry_diagnostics -- --exact
~~~

Expected: FAIL，load_user_themes 或 ThemeLoadReport 不存在。

- [ ] **Step 3: 定义 App 报告并实现纯组合 helper**

在 `theme_loader.rs` 增加：

~~~rust
#[derive(Debug, Default)]
pub(crate) struct ThemeLoadReport {
    pub(crate) source_diagnostics: Vec<ThemeSourceDiagnostic>,
    pub(crate) registry_errors: Vec<ui::theme::ThemeLoadError>,
    pub(crate) registered_ids: Vec<String>,
}
~~~

删除 Task 4 的兼容 wrapper，把 `load_theme_source_batch` 重命名为最终 `load_theme_sources`，并把 theme_loader.rs 测试中的同名调用全部同步重命名。

在 `app_init.rs` 增加：

~~~rust
fn load_user_themes(dir: &std::path::Path) -> (ui::theme::ThemeRegistry, ThemeLoadReport) {
    let batch = crate::theme_loader::load_theme_sources(dir);
    let mut registry = ui::theme::ThemeRegistry::new();
    let registration = registry.register_sources(batch.sources);
    let report = ThemeLoadReport {
        source_diagnostics: batch.diagnostics,
        registry_errors: registration.errors,
        registered_ids: registration.registered_ids,
    };
    (registry, report)
}
~~~

初始化使用：

~~~rust
let (theme_registry, theme_load_report) = load_user_themes(&themes_dir());
for diagnostic in &theme_load_report.source_diagnostics {
    eprintln!("[theme] {diagnostic:?}");
}
for error in &theme_load_report.registry_errors {
    eprintln!("[theme] {error}");
}
~~~

`ThemeLoadError` 的 Display 必须在 Task 2 覆盖全部 variant；此处不自行格式化 variant。

- [ ] **Step 4: 把报告存入 App**

`app.rs` 在 active_theme_pair 相邻位置增加：

~~~rust
pub(crate) theme_load_report: crate::theme_loader::ThemeLoadReport,
~~~

`app_init.rs` 的现有 App 字段初始化列表在 `active_theme_pair` 后放入 `theme_load_report`；Theme::resolve 最后实参使用 `&theme_registry`。

- [ ] **Step 5: 运行报告/主题/app 测试**

~~~bash
cargo test -p edit-plus-app --lib app_init::tests::build_theme_registry_retains_source_and_registry_diagnostics -- --exact
cargo test -p edit-plus-app --lib theme_loader::tests -- --nocapture
cargo test -p edit-plus-ui --lib theme_registry::tests -- --nocapture
cargo check -p edit-plus-app
~~~

Expected: PASS；坏文件与保留 ID 均被保留，好主题仍注册。

- [ ] **Step 6: 提交**

~~~bash
git add crates/app/src/theme_loader.rs crates/app/src/app.rs crates/app/src/app_init.rs
git commit -m "refactor(app): retain complete theme load report"
~~~

## Phase B：Widget 单一输入

### Task 6: 为 owned widget input 补齐 TabInfo 调试契约

**Files:**
- Modify/Test: `crates/ui/src/widgets/tab_bar/types.rs`

**Interfaces:**
- Consumes: 当前仅 Clone 的 TabInfo。
- Produces: `TabInfo: Debug + Clone`，使 TabBarWidgetInput/SidebarWidgetInput 可派生 Debug。

- [ ] **Step 1: 写 trait 编译测试**

在 types.rs 增加：

~~~rust
#[cfg(test)]
mod tests {
    use super::TabInfo;

    fn assert_debug_clone<T: std::fmt::Debug + Clone>() {}

    #[test]
    fn tab_info_supports_owned_input_diagnostics() {
        assert_debug_clone::<TabInfo>();
    }
}
~~~

- [ ] **Step 2: 运行测试并确认缺少 Debug**

~~~bash
cargo test -p edit-plus-ui --lib widgets::tab_bar::types::tests::tab_info_supports_owned_input_diagnostics -- --exact
~~~

Expected: FAIL，TabInfo 未实现 Debug。

- [ ] **Step 3: 补齐派生并验证**

~~~rust
#[derive(Debug, Clone)]
pub struct TabInfo {
    // 字段保持不变
}
~~~

~~~bash
cargo test -p edit-plus-ui --lib widgets::tab_bar::types::tests::tab_info_supports_owned_input_diagnostics -- --exact
cargo check -p edit-plus-app
~~~

Expected: PASS。

- [ ] **Step 4: 提交**

~~~bash
git add crates/ui/src/widgets/tab_bar/types.rs
git commit -m "refactor(ui): make tab info diagnosable"
~~~

### Task 7: 让 TabInfo.pinned 成为 TabBar 唯一 pin 真值

**Files:**
- Modify/Test: `crates/ui/src/widgets/tab_bar/layout.rs`
- Modify/Test: `crates/ui/src/widgets/tab_bar/state.rs`
- Modify/Test: `crates/ui/src/widgets/tab_bar/tests.rs`

**Interfaces:**
- Consumes: 现有 TabInfo.pinned 字段和 layout/state 测试。
- Produces: 本任务 Step 3 的完整 layout_tabs 签名不再接收 HashSet；私有 TabBarInput 暂留一个不参与布局的兼容字段，Task 8 删除。

- [ ] **Step 1: 写 pin 双真值回归测试**

在 `tests.rs` 增加一个只在 TabInfo 上声明 pin 的测试：

~~~rust
#[test]
fn tab_info_pinned_is_the_only_layout_source() {
    let mut tabs = sample_tabs(3);
    tabs[1].pinned = true;
    let ctx = test_ctx();

    let layout = layout_tabs(
        &tabs,
        0,
        &ctx,
        tab_bar_height(ctx.dpi),
        false,
        false,
        0.0,
        None,
    );

    assert_eq!(
        layout.tabs.iter().filter(|entry| entry.pinned).map(|entry| entry.index).collect::<Vec<_>>(),
        vec![1]
    );
}
~~~

如果当前测试 helper 没有 `sample_tabs`，增加：

~~~rust
fn sample_tabs(count: usize) -> Vec<TabInfo> {
    (0..count)
        .map(|index| TabInfo {
            title: format!("tab-{index}"),
            file_path: None,
            is_dirty: false,
            pinned: false,
            language: String::new(),
        })
        .collect()
}
~~~

- [ ] **Step 2: 运行测试并确认旧签名要求 pinned_indices**

~~~bash
cargo test -p edit-plus-ui --lib widgets::tab_bar::tab_bar_tests::tab_info_pinned_is_the_only_layout_source -- --exact
~~~

Expected: FAIL，layout_tabs 仍要求额外 `&HashSet<usize>` 参数。

- [ ] **Step 3: 从 layout 删除 pin 集合并让 state 忽略兼容字段**

`layout_tabs` 最终签名中删除 `pinned_indices`：

~~~rust
pub fn layout_tabs(
    tab_infos: &[TabInfo],
    active_index: usize,
    ctx: &TabBarCtx,
    tab_height: f32,
    back_enabled: bool,
    forward_enabled: bool,
    scroll_offset: f32,
    shaper: Option<&mut Shaper>,
) -> TabBarLayout
~~~

删除 `use std::collections::HashSet`。排序和宽度计算统一读取：

~~~rust
let a_pinned = tab_infos[*a].pinned;
let b_pinned = tab_infos[*b].pinned;
~~~

以及：

~~~rust
let is_pinned = tab_infos[i].pinned;
~~~

`state.rs` 暂时保留旧 widget 编译所需、但布局不再读取的兼容字段：

~~~rust
pub(crate) struct TabBarInput<'a> {
    pub tabs: &'a [TabInfo],
    pub active_index: Option<usize>,
    #[doc(hidden)]
    pub pinned_indices: &'a std::collections::HashSet<usize>,
    pub back_enabled: bool,
    pub forward_enabled: bool,
    pub screen_w: f32,
    pub screen_h: f32,
}
~~~

`update_layout` 调用 layout_tabs 时不再传 pin 集合。兼容字段只让尚未迁移的 widget.rs 保持编译，从本任务开始不影响排序、布局或绘制。

- [ ] **Step 4: 迁移 tab_bar/tests.rs 的所有布局调用**

每个原先单独构造 `HashSet` 的测试，把状态写回 tab fixture：

~~~rust
let mut tabs = sample_tabs(4);
tabs[0].pinned = true;
tabs[2].pinned = true;
~~~

删除 layout_tabs 实参中的 `&pinned`。对每个 pinned 行为测试保持原断言：固定位置、窄宽度、max_scroll、hit-test、pinned_total_width 均不变。

- [ ] **Step 5: 运行全部 TabBar 测试与残留扫描**

~~~bash
cargo test -p edit-plus-ui --lib widgets::tab_bar -- --nocapture
rg -n "pinned_indices" crates/ui/src/widgets/tab_bar/layout.rs crates/ui/src/widgets/tab_bar/tests.rs
cargo check -p edit-plus-app
~~~

Expected: 测试 PASS；扫描无输出；state/widget 中只允许存在不参与布局的兼容字段，Task 8 删除。

- [ ] **Step 6: 提交**

~~~bash
git add crates/ui/src/widgets/tab_bar/layout.rs crates/ui/src/widgets/tab_bar/state.rs crates/ui/src/widgets/tab_bar/tests.rs
git commit -m "refactor(ui): use tab info as pin source"
~~~

### Task 8: 定义 owned TabBarWidgetInput 并删除长参数入口

**Files:**
- Modify/Test: `crates/ui/src/widgets/tab_bar/widget.rs`
- Modify: `crates/ui/src/widgets/tab_bar/mod.rs`
- Modify: `crates/ui/src/widgets/tab_bar/state.rs`

**Interfaces:**
- Consumes: Task 7 私有 `TabBarInput<'_>`；前置计划最终 UiMetrics。
- Produces: 公共 `TabBarWidgetInput` 和 `TabBarWidget::set_input(input, shaper)`。

- [ ] **Step 1: 写输入完整替换失败测试**

在 `widget.rs` 增加测试模块：

~~~rust
#[cfg(test)]
mod input_tests {
    use super::*;

    fn metrics(dpi: f32) -> crate::settings::UiMetrics {
        crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), dpi)
    }

    fn tab(title: &str, pinned: bool) -> super::super::TabInfo {
        super::super::TabInfo {
            title: title.into(),
            file_path: None,
            is_dirty: false,
            pinned,
            language: String::new(),
        }
    }

    #[test]
    fn set_input_replaces_every_frame_field() {
        let mut widget = TabBarWidget::new();
        widget.set_input(
            TabBarWidgetInput {
                tabs: vec![tab("first", true)],
                active_index: Some(0),
                back_enabled: true,
                forward_enabled: false,
                screen_size_px: (800.0, 600.0),
                hovered_index: Some(0),
                scroll_offset_px: 30.0,
                metrics: metrics(2.0),
            },
            None,
        );
        widget.set_input(
            TabBarWidgetInput {
                tabs: vec![tab("second", false)],
                active_index: None,
                back_enabled: false,
                forward_enabled: true,
                screen_size_px: (400.0, 300.0),
                hovered_index: None,
                scroll_offset_px: 0.0,
                metrics: metrics(1.0),
            },
            None,
        );

        let input = widget.input.as_ref().unwrap();
        assert_eq!(input.tabs[0].title, "second");
        assert_eq!(input.active_index, None);
        assert!(!input.back_enabled);
        assert!(input.forward_enabled);
        assert_eq!(input.screen_size_px, (400.0, 300.0));
        assert_eq!(widget.state.hovered_index(), None);
        assert_eq!(widget.state.scroll_offset(), 0.0);
        assert_eq!(input.metrics.dpi, 1.0);
    }
}
~~~

- [ ] **Step 2: 运行测试并确认类型/API 不存在**

~~~bash
cargo test -p edit-plus-ui --lib widgets::tab_bar::widget::input_tests -- --nocapture
~~~

Expected: FAIL，TabBarWidgetInput 或 set_input 不存在。

- [ ] **Step 3: 定义最终 input 并替换内部 owned 类型**

删除 `TabBarInputOwned`，定义：

~~~rust
#[derive(Debug, Clone)]
pub struct TabBarWidgetInput {
    pub tabs: Vec<super::TabInfo>,
    pub active_index: Option<usize>,
    pub back_enabled: bool,
    pub forward_enabled: bool,
    pub screen_size_px: (f32, f32),
    pub hovered_index: Option<usize>,
    pub scroll_offset_px: f32,
    pub metrics: crate::settings::UiMetrics,
}
~~~

Widget 字段改为：

~~~rust
input: Option<TabBarWidgetInput>,
~~~

唯一注入入口：

~~~rust
pub fn set_input(
    &mut self,
    input: TabBarWidgetInput,
    shaper: Option<&mut shaping::Shaper>,
) {
    self.active_index = input.active_index.unwrap_or(0);
    self.state.set_hovered_index(input.hovered_index);
    self.state.set_scroll_offset(input.scroll_offset_px);
    let borrowed = TabBarInput {
        tabs: &input.tabs,
        active_index: input.active_index,
        back_enabled: input.back_enabled,
        forward_enabled: input.forward_enabled,
        screen_w: input.screen_size_px.0,
        screen_h: input.screen_size_px.1,
    };
    self.state.update_layout(&borrowed, shaper, input.metrics.dpi);
    self.input = Some(input);
}
~~~

`on_event` 中重布局时从 `input.metrics.dpi`、`input.screen_size_px` 和 `input.tabs` 构造同一 borrowed view；删除所有 `pinned_indices` 使用。shaper 是运行时服务，不属于帧数据，因此保留为第二参数。

- [ ] **Step 4: 从模块门面导出 input**

`mod.rs` 改为：

~~~rust
pub use widget::{TabBarWidget, TabBarWidgetInput};
pub(crate) use state::TabBarInput;
~~~

state 的公共 re-export 精确改为 `pub use state::{TabBarAction, TabBarState};`。

同时从 state.rs::TabBarInput 删除 Task 7 的兼容 pinned_indices 字段；widget 构造 borrowed input 时不再创建 pin HashSet。

- [ ] **Step 5: 运行 UI 测试**

~~~bash
cargo test -p edit-plus-ui --lib widgets::tab_bar -- --nocapture
cargo check -p edit-plus-ui
~~~

Expected: UI 测试 PASS。为让尚未迁移的 app 编译，暂时保留 set_tabs_input wrapper，其完整参数与迁移前一致：tabs、active_index、`_pinned_indices: &HashSet<usize>`、back_enabled、forward_enabled、screen_w、screen_h、shaper、hovered_index、scroll_offset、metrics；wrapper 完全忽略 `_pinned_indices`，只从 tabs[*].pinned 构造 TabBarWidgetInput。Task 9 删除 wrapper。

- [ ] **Step 6: 提交**

~~~bash
git add crates/ui/src/widgets/tab_bar/widget.rs crates/ui/src/widgets/tab_bar/mod.rs crates/ui/src/widgets/tab_bar/state.rs
git commit -m "refactor(ui): define owned tab bar input"
~~~

### Task 9: 由 UiShell 构造并注入 TabBarWidgetInput

**Files:**
- Modify/Test: `crates/app/src/ui_shell.rs`
- Modify/Test: `crates/app/src/app_renderer.rs`
- Modify: `crates/ui/src/widgets/tab_bar/widget.rs`

**Interfaces:**
- Consumes: Task 8 `TabBarWidgetInput`；App renderer 已把 Workspace pin 状态写入 TabInfo.pinned。
- Produces: UiShell 无 `tab_input_pinned_indices`；所有 TabBarWidget 调用只使用 set_input。

- [ ] **Step 1: 写 shell 不保存第二份 pin 状态的失败测试**

把 ui_shell 测试中的 `set_tabs_input` 调用改为最终签名，并增加：

~~~rust
#[test]
fn shell_builds_tab_widget_input_from_tab_info_pin_state() {
    let mut shell = UiShell::new();
    let mut pinned = test_tab("pinned");
    pinned.pinned = true;
    shell.set_tabs_input(
        vec![pinned],
        Some(0),
        false,
        false,
        None,
        0.0,
    );

    let input = shell.tab_widget_input(Screen { w: 800.0, h: 600.0 }, metrics(2.0));
    assert!(input.tabs[0].pinned);
    assert_eq!(input.screen_size_px, (800.0, 600.0));
    assert_eq!(input.metrics.dpi, 2.0);
}
~~~

- [ ] **Step 2: 运行测试并确认旧 shell 需要 HashSet**

~~~bash
cargo test -p edit-plus-app --lib ui_shell::tests::shell_builds_tab_widget_input_from_tab_info_pin_state -- --exact
~~~

Expected: FAIL，set_tabs_input 仍要求 pinned_indices，tab_widget_input 不存在。

- [ ] **Step 3: 删除 UiShell pin cache 并集中构造 input**

删除字段及初始化：

~~~rust
tab_input_pinned_indices: HashSet<usize>,
~~~

`set_tabs_input` 最终参数为：

~~~rust
pub fn set_tabs_input(
    &mut self,
    tabs: Vec<ui::tab_bar::TabInfo>,
    active_index: Option<usize>,
    back_enabled: bool,
    forward_enabled: bool,
    hovered_index: Option<usize>,
    scroll_offset_px: f32,
)
~~~

增加私有生产 helper；测试可在同模块访问：

~~~rust
fn tab_widget_input(
    &self,
    screen: Screen,
    metrics: ui::settings::UiMetrics,
) -> ui::tab_bar::TabBarWidgetInput {
    ui::tab_bar::TabBarWidgetInput {
        tabs: self.tab_input_tabs.clone(),
        active_index: self.tab_input_active_index,
        back_enabled: self.tab_input_back_enabled,
        forward_enabled: self.tab_input_forward_enabled,
        screen_size_px: (screen.w, screen.h),
        hovered_index: self.tab_hovered_index,
        scroll_offset_px: self.tab_scroll_offset,
        metrics,
    }
}
~~~

`update_widget_state` 与 `rebuild_dock_children` 都先构造一次 input，然后：

~~~rust
tbw.set_input(input, None);
~~~

- [ ] **Step 4: 迁移 renderer 并删除兼容 wrapper**

`app_renderer.rs` 调用删除：

~~~rust
self.workspace.pinned_indices().clone(),
~~~

保留构造 TabInfo 时的唯一写入：

~~~rust
pinned: self.workspace.pinned_indices().contains(&i),
~~~

从 `tab_bar/widget.rs` 删除 Task 8 临时 set_tabs_input wrapper。

- [ ] **Step 5: 运行 tab/shell/app 测试和扫描**

~~~bash
cargo test -p edit-plus-ui --lib widgets::tab_bar -- --nocapture
cargo test -p edit-plus-app --lib ui_shell::tests -- --nocapture
rg -n "tab_input_pinned_indices|set_tabs_input\(" crates/app/src crates/ui/src/widgets/tab_bar
cargo check -p edit-plus-app
~~~

Expected: 测试 PASS；扫描只允许 `UiShell::set_tabs_input` 自身和 renderer 调用，不出现 widget 长参数入口或 pin cache。

- [ ] **Step 6: 提交**

~~~bash
git add crates/app/src/ui_shell.rs crates/app/src/app_renderer.rs crates/ui/src/widgets/tab_bar/widget.rs
git commit -m "refactor(app): inject tab bar frame input"
~~~

### Task 10: 定义 SidebarWidgetInput 并替换长参数入口

**Files:**
- Modify: `crates/ui/src/widgets/sidebar/types.rs`
- Modify/Test: `crates/ui/src/widgets/sidebar/mod.rs`
- Modify/Test: `crates/ui/src/widgets/sidebar/widget_tests.rs`

**Interfaces:**
- Consumes: 前置计划的 UiMetrics、SidebarSettingsInput，以及现有 SidebarWidget::set_input 参数。
- Produces: `SidebarWidgetInput` 和临时 `set_frame_input(input)`；Task 11 原子迁移 app 后得到最终 `set_input(input)`。

- [ ] **Step 1: 写 owned input 全量更新失败测试**

在 widget_tests.rs 增加 helper 和测试：

~~~rust
fn sidebar_widget_input(tabs: Vec<TabInfo>, active_index: Option<usize>) -> SidebarWidgetInput {
    SidebarWidgetInput {
        tabs,
        active_index,
        traffic_light_inset_px: (68.0, 0.0),
        screen_size_px: (800.0, 600.0),
        metrics: metrics(1.0),
        settings: sidebar_settings(),
    }
}

#[test]
fn widget_input_replaces_geometry_metrics_and_behavior() {
    let mut widget = SidebarWidget::new(SidebarConfig::new_default(1.0), metrics(1.0));
    let mut first = sidebar_widget_input(vec![make_tab("first")], Some(0));
    first.settings.word_wrap = false;
    widget.set_frame_input(first);

    let mut second = sidebar_widget_input(vec![make_tab("second")], None);
    second.traffic_light_inset_px = (20.0, 4.0);
    second.screen_size_px = (400.0, 300.0);
    second.metrics = metrics(2.0);
    second.settings.word_wrap = true;
    widget.set_frame_input(second);

    assert_eq!(widget.tabs[0].title, "second");
    assert_eq!(widget.active_index, None);
    assert_eq!(widget.traffic_light_inset, (20.0, 4.0));
    assert_eq!((widget.screen_w, widget.screen_h), (400.0, 300.0));
    assert_eq!(widget.metrics.dpi, 2.0);
    assert!(widget.settings_input.word_wrap);
}
~~~

测试模块是 sidebar 子模块，可访问 pub(crate) 字段；前置计划已把行为字段命名为 `settings_input`，本计划固定沿用该名称。

- [ ] **Step 2: 运行测试并确认新类型/API 不存在**

~~~bash
cargo test -p edit-plus-ui --lib widgets::sidebar::widget_tests::widget_input_replaces_geometry_metrics_and_behavior -- --exact
~~~

Expected: FAIL，SidebarWidgetInput 或 set_frame_input 不存在。

- [ ] **Step 3: 定义最终 SidebarWidgetInput**

在 `types.rs` 增加：

~~~rust
#[derive(Debug, Clone)]
pub struct SidebarWidgetInput {
    pub tabs: Vec<TabInfo>,
    pub active_index: Option<usize>,
    pub traffic_light_inset_px: (f32, f32),
    pub screen_size_px: (f32, f32),
    pub metrics: crate::settings::UiMetrics,
    pub settings: SidebarSettingsInput,
}
~~~

旧 `SidebarInput<'_>` 继续供 SidebarState/layout 借用，改为 `pub(crate) struct SidebarInput<'a>`，并从 sidebar/mod.rs 的公共 re-export 列表删除。

`mod.rs` re-export 增加 SidebarWidgetInput，并先增加新入口：

~~~rust
pub fn set_frame_input(&mut self, input: SidebarWidgetInput) {
    let SidebarWidgetInput {
        tabs,
        active_index,
        traffic_light_inset_px,
        screen_size_px,
        metrics,
        settings,
    } = input;

    let mut indexed: Vec<(usize, TabInfo)> = tabs.into_iter().enumerate().collect();
    indexed.sort_by_key(|(_, tab)| !tab.pinned);
    let new_active = active_index.and_then(|active| {
        indexed.iter().position(|(original, _)| *original == active)
    });
    let new_tab_index_map: Vec<usize> =
        indexed.iter().map(|(original, _)| *original).collect();
    let new_tabs: Vec<TabInfo> = indexed.into_iter().map(|(_, tab)| tab).collect();
    let tabs_changed = new_tabs.len() != self.tabs.len()
        || new_active != self.active_index
        || new_tabs.iter().zip(self.tabs.iter()).any(|(a, b)| {
            a.title != b.title || a.is_dirty != b.is_dirty || a.pinned != b.pinned
        });
    if tabs_changed {
        self.list_items_dirty = true;
    }
    self.active_index = new_active;
    self.tab_index_map = new_tab_index_map;
    self.tabs = new_tabs;
    self.traffic_light_inset = traffic_light_inset_px;
    self.screen_w = screen_size_px.0;
    self.screen_h = screen_size_px.1;
    self.metrics = metrics;
    self.settings_input = settings;
}
~~~

不得改变 list_items_dirty 判定、pin 排序或 workspace index map。

- [ ] **Step 4: 迁移 widget_tests 的所有调用**

每个旧调用：

~~~rust
widget.set_input(tabs, active, inset, width, height, &metrics, settings);
~~~

改为：

~~~rust
widget.set_frame_input(SidebarWidgetInput {
    tabs,
    active_index: active,
    traffic_light_inset_px: inset,
    screen_size_px: (width, height),
    metrics,
    settings,
});
~~~

不要在测试中恢复 Settings 整体依赖。

- [ ] **Step 5: 运行全部 Sidebar 测试和编译**

~~~bash
cargo test -p edit-plus-ui --lib widgets::sidebar -- --nocapture
cargo check -p edit-plus-ui
~~~

Expected: PASS。为保持 app 编译，本任务保留原 set_input 参数：tabs、active_index、traffic_light_inset、screen_w、screen_h、`&UiMetrics`、SidebarSettingsInput；方法体只构造 SidebarWidgetInput 并调用 set_frame_input。Task 11 迁移 app 后删除旧方法并重命名新方法。

- [ ] **Step 6: 提交**

~~~bash
git add crates/ui/src/widgets/sidebar/types.rs crates/ui/src/widgets/sidebar/mod.rs crates/ui/src/widgets/sidebar/widget_tests.rs
git commit -m "refactor(ui): define owned sidebar input"
~~~

### Task 11: 由 UiShell 构造并注入 SidebarWidgetInput

**Files:**
- Modify/Test: `crates/app/src/ui_shell.rs`
- Modify: `crates/ui/src/widgets/sidebar/mod.rs`
- Modify/Test: `crates/ui/src/widgets/sidebar/widget_tests.rs`

**Interfaces:**
- Consumes: Task 10 SidebarWidgetInput；前置计划 ShellInputs.metrics/sidebar_settings。
- Produces: UiShell 的新建/原位更新 SidebarWidget 路径使用相同输入构造器。

- [ ] **Step 1: 写 shell input 一致性失败测试**

在 ui_shell.rs 测试模块增加：

~~~rust
#[test]
fn shell_builds_sidebar_input_from_one_frame_snapshot() {
    let mut shell = UiShell::new();
    shell.sidebar_tabs = vec![test_tab("one")];
    shell.sidebar_active_index = Some(0);
    shell.sidebar_traffic_light_inset = (68.0, 0.0);
    let mut inputs = shell_inputs();
    inputs.metrics = metrics(2.0);
    inputs.sidebar_settings.word_wrap = false;

    let input = shell.sidebar_widget_input(Screen { w: 900.0, h: 700.0 }, &inputs);

    assert_eq!(input.tabs[0].title, "one");
    assert_eq!(input.screen_size_px, (900.0, 700.0));
    assert_eq!(input.metrics.dpi, 2.0);
    assert!(!input.settings.word_wrap);
}
~~~

- [ ] **Step 2: 运行测试并确认 helper 不存在**

~~~bash
cargo test -p edit-plus-app --lib ui_shell::tests::shell_builds_sidebar_input_from_one_frame_snapshot -- --exact
~~~

Expected: FAIL，sidebar_widget_input 不存在。

- [ ] **Step 3: 实现唯一构造 helper 并迁移两个路径**

~~~rust
fn sidebar_widget_input(
    &self,
    screen: Screen,
    inputs: &ShellInputs,
) -> ui::widgets::sidebar::SidebarWidgetInput {
    ui::widgets::sidebar::SidebarWidgetInput {
        tabs: self.sidebar_tabs.clone(),
        active_index: self.sidebar_active_index,
        traffic_light_inset_px: self.sidebar_traffic_light_inset,
        screen_size_px: (screen.w, screen.h),
        metrics: inputs.metrics,
        settings: inputs.sidebar_settings,
    }
}
~~~

Task 14 建立语义模块门面后，Task 15 再把这里统一改为 ui::sidebar 路径。

`update_widget_state` 与 `rebuild_dock_children` 都调用同一 helper，并执行：

~~~rust
sw.set_frame_input(sidebar_input);
~~~

保留 `inject_persistent` 的原有调用顺序：先 set_frame_input，再恢复 persistent。

- [ ] **Step 4: 删除 legacy wrapper 并验证**

从 sidebar/mod.rs 删除旧长参数 set_input，把 set_frame_input 重命名为最终 `set_input(SidebarWidgetInput)`；widget_tests 与 ui_shell 的全部 set_frame_input 同步改名为 set_input。

~~~bash
cargo test -p edit-plus-ui --lib widgets::sidebar -- --nocapture
cargo test -p edit-plus-app --lib ui_shell::tests -- --nocapture
rg -n "set_frame_input|sw\.set_input\([^)]*," crates/ui/src/widgets/sidebar crates/app/src/ui_shell.rs
cargo check -p edit-plus-app
~~~

Expected: 测试 PASS；无 legacy wrapper 或多位置参数调用。

- [ ] **Step 5: 提交**

~~~bash
git add crates/app/src/ui_shell.rs crates/ui/src/widgets/sidebar/mod.rs crates/ui/src/widgets/sidebar/widget_tests.rs
git commit -m "refactor(app): inject sidebar frame input"
~~~

### Task 12: 定义 ScrollbarInput 并规范无效 scroll_top

**Files:**
- Modify/Test: `crates/ui/src/widgets/scrollbar.rs`

**Interfaces:**
- Consumes: 现有 ScrollbarWidget 三个 frame 字段。
- Produces: Copy 的 ScrollbarInput 和临时 set_frame_input；Task 13 原子迁移 app 后得到最终 set_input(ScrollbarInput)。

- [ ] **Step 1: 写单一输入和非有限值失败测试**

~~~rust
#[test]
fn set_input_replaces_all_scrollbar_fields() {
    let mut widget = ScrollbarWidget::new();
    widget.set_frame_input(ScrollbarInput {
        viewport_height_px: 50.0,
        total_display_rows: 100,
        scroll_top_rows: 25.0,
    });
    widget.set_frame_input(ScrollbarInput {
        viewport_height_px: 20.0,
        total_display_rows: 40,
        scroll_top_rows: 5.0,
    });
    assert_eq!(widget.input, ScrollbarInput {
        viewport_height_px: 20.0,
        total_display_rows: 40,
        scroll_top_rows: 5.0,
    });
}

#[test]
fn non_finite_scroll_top_is_normalized_to_zero() {
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut widget = ScrollbarWidget::new();
        widget.set_frame_input(ScrollbarInput {
            viewport_height_px: 20.0,
            total_display_rows: 40,
            scroll_top_rows: value,
        });
        assert_eq!(widget.input.scroll_top_rows, 0.0);
    }
}
~~~

- [ ] **Step 2: 运行测试并确认新类型不存在**

~~~bash
cargo test -p edit-plus-ui --lib widgets::scrollbar::tests::set_input_replaces_all_scrollbar_fields -- --exact
~~~

Expected: FAIL，ScrollbarInput 不存在。

- [ ] **Step 3: 用 input 替换三个 widget 字段**

~~~rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollbarInput {
    pub viewport_height_px: f64,
    pub total_display_rows: usize,
    pub scroll_top_rows: f64,
}

impl Default for ScrollbarInput {
    fn default() -> Self {
        Self { viewport_height_px: 0.0, total_display_rows: 0, scroll_top_rows: 0.0 }
    }
}
~~~

Widget 字段改为 `input: ScrollbarInput`。先增加新入口：

~~~rust
pub fn set_frame_input(&mut self, mut input: ScrollbarInput) {
    if !input.scroll_top_rows.is_finite() {
        input.scroll_top_rows = 0.0;
    }
    self.input = input;
}
~~~

`compute_layout` 读取 `self.input.viewport_height_px`、`total_display_rows`、`scroll_top_rows`。测试 make_widget helper 改为构造 ScrollbarInput。

- [ ] **Step 4: 运行全部 Scrollbar 测试和 App 编译**

~~~bash
cargo test -p edit-plus-ui --lib widgets::scrollbar -- --nocapture
cargo check -p edit-plus-ui
~~~

Expected: UI 测试 PASS。为保持 app 编译，保留原三参数 set_input(viewport, total, top)，其方法体只构造 ScrollbarInput 并调用 set_frame_input；Task 13 删除旧方法并重命名新方法。

- [ ] **Step 5: 提交**

~~~bash
git add crates/ui/src/widgets/scrollbar.rs
git commit -m "refactor(ui): define scrollbar frame input"
~~~

### Task 13: 由 UiShell 构造并注入 ScrollbarInput

**Files:**
- Modify/Test: `crates/app/src/ui_shell.rs`
- Modify: `crates/ui/src/widgets/scrollbar.rs`

**Interfaces:**
- Consumes: Task 12 ScrollbarInput。
- Produces: UiShell 的更新/重建路径都使用 `ScrollbarInput`；删除 legacy API。

- [ ] **Step 1: 写 shell scrollbar input 映射失败测试**

~~~rust
#[test]
fn shell_builds_scrollbar_input_with_explicit_units() {
    let mut shell = UiShell::new();
    shell.set_scrollbar_input(42.0, 100, 12.5);
    assert_eq!(
        shell.scrollbar_widget_input(),
        ui::widgets::scrollbar::ScrollbarInput {
            viewport_height_px: 42.0,
            total_display_rows: 100,
            scroll_top_rows: 12.5,
        }
    );
}
~~~

- [ ] **Step 2: 运行测试并确认 helper 不存在**

~~~bash
cargo test -p edit-plus-app --lib ui_shell::tests::shell_builds_scrollbar_input_with_explicit_units -- --exact
~~~

Expected: FAIL，scrollbar_widget_input 不存在。

- [ ] **Step 3: 增加构造 helper 并迁移两个注入点**

~~~rust
fn scrollbar_widget_input(&self) -> ui::widgets::scrollbar::ScrollbarInput {
    ui::widgets::scrollbar::ScrollbarInput {
        viewport_height_px: self.scrollbar_viewport_height,
        total_display_rows: self.scrollbar_total_display_rows,
        scroll_top_rows: self.scrollbar_scroll_top,
    }
}
~~~

`update_widget_state` 与 `rebuild_dock_children` 均改为：

~~~rust
sw.set_frame_input(self.scrollbar_widget_input());
~~~

为避免 `&mut self.dock.children` 与 `&self` 借用冲突，在循环前先构造 Copy input：

~~~rust
let scrollbar_input = self.scrollbar_widget_input();
~~~

- [ ] **Step 4: 收敛最终方法名并验证**

删除旧三参数 set_input，把 set_frame_input 重命名为最终 `set_input(ScrollbarInput)`；ui_shell 的两个调用同步改为 set_input。

~~~bash
cargo test -p edit-plus-ui --lib widgets::scrollbar -- --nocapture
cargo test -p edit-plus-app --lib ui_shell::tests -- --nocapture
rg -n "set_frame_input|set_input\([^)]*,[^)]*," crates/ui/src/widgets/scrollbar.rs crates/app/src/ui_shell.rs
cargo check -p edit-plus-app
~~~

Expected: PASS；扫描无输出。

- [ ] **Step 5: 提交**

~~~bash
git add crates/app/src/ui_shell.rs crates/ui/src/widgets/scrollbar.rs
git commit -m "refactor(app): inject scrollbar frame input"
~~~

## Phase C：语义模块门面与公共 API 收口

### Task 14: 建立 widget 根级语义模块和正向公共 API 测试

**Files:**
- Modify: `crates/ui/src/lib.rs`
- Create/Test: `crates/ui/tests/public_api.rs`

**Interfaces:**
- Consumes: widgets/mod.rs 当前公开的 14 个组件模块；Tasks 2、8、10、12 的最终 public types。
- Produces: `ui::tab_bar`、`ui::sidebar`、`ui::scrollbar` 等稳定路径；暂时保留 `ui::widgets` 兼容路径。

- [ ] **Step 1: 写外部 crate 视角的 compile-pass 测试**

创建 `crates/ui/tests/public_api.rs`：

~~~rust
use ui::core::{Event, Rect, Widget};
use ui::scrollbar::{ScrollbarAction, ScrollbarInput, ScrollbarWidget};
use ui::settings::{Settings, UiMetrics};
use ui::sidebar::{SidebarAction, SidebarSettingsInput, SidebarWidget, SidebarWidgetInput};
use ui::tab_bar::{TabBarAction, TabBarWidget, TabBarWidgetInput, TabInfo};
use ui::theme::{ThemeLoadError, ThemeRegistry, ThemeSource};

fn assert_widget<T: Widget>() {}
fn assert_debug<T: std::fmt::Debug>() {}

#[test]
fn semantic_public_modules_compile_for_external_consumers() {
    assert_widget::<TabBarWidget>();
    assert_widget::<SidebarWidget>();
    assert_widget::<ScrollbarWidget>();
    assert_debug::<ThemeLoadError>();
    let _event: Option<Event> = None;
    let _rect = Rect::ZERO;

    let settings = Settings::new();
    let metrics = UiMetrics::from_settings(&settings, 2.0);
    let behavior = SidebarSettingsInput::from(&settings);
    let tab = TabInfo {
        title: "tab".into(),
        file_path: None,
        is_dirty: false,
        pinned: true,
        language: String::new(),
    };
    let _tab_input = TabBarWidgetInput {
        tabs: vec![tab.clone()],
        active_index: Some(0),
        back_enabled: false,
        forward_enabled: false,
        screen_size_px: (800.0, 600.0),
        hovered_index: None,
        scroll_offset_px: 0.0,
        metrics,
    };
    let _sidebar_input = SidebarWidgetInput {
        tabs: vec![tab],
        active_index: Some(0),
        traffic_light_inset_px: (68.0, 0.0),
        screen_size_px: (800.0, 600.0),
        metrics,
        settings: behavior,
    };
    let _scrollbar_input = ScrollbarInput {
        viewport_height_px: 40.0,
        total_display_rows: 100,
        scroll_top_rows: 10.0,
    };
    let _registry = ThemeRegistry::new();
    let _source = ThemeSource {
        id: "sample".into(),
        path: "sample.toml".into(),
        content: "is_dark = true".into(),
    };
    let _actions: (Option<TabBarAction>, Option<SidebarAction>, Option<ScrollbarAction>) =
        (None, None, None);
}
~~~

- [ ] **Step 2: 运行测试并确认根级组件模块尚未公开**

~~~bash
cargo test -p edit-plus-ui --test public_api -- --nocapture
~~~

Expected: FAIL，`ui::sidebar`、`ui::scrollbar` 等路径不存在。

- [ ] **Step 3: 在 lib.rs 增加完整语义门面**

保持 `pub mod widgets;` 一个兼容阶段，并用一次 re-export 取代旧单独 tab_bar re-export：

~~~rust
pub use widgets::{
    button, icon, list, popup_menu, scrollbar, search_bar, sidebar, status_bar, tab_bar, text_box,
    title_bar, title_bar_spacer, toc, tooltip,
};
~~~

此任务暂不删除根级 ListWidget/PopupMenuWidget re-export，不私有化任何模块。

- [ ] **Step 4: 运行公共 API、UI 和 app 编译**

~~~bash
cargo test -p edit-plus-ui --test public_api -- --nocapture
cargo test -p edit-plus-ui --lib
cargo check -p edit-plus-app
~~~

Expected: PASS；新旧导入路径同时可用。

- [ ] **Step 5: 提交**

~~~bash
git add crates/ui/src/lib.rs crates/ui/tests/public_api.rs
git commit -m "refactor(ui): expose semantic widget modules"
~~~

### Task 15: 迁移 action/workspace/event 的 widget 导入

**Files:**
- Modify: `crates/app/src/actions.rs`
- Modify: `crates/app/src/workspace.rs`
- Modify/Test: `crates/app/src/events.rs`

**Interfaces:**
- Consumes: Task 14 根级 widget 模块。
- Produces: 三个领域文件不再依赖 ui::widgets 或组件根级类型 re-export。

- [ ] **Step 1: 建立本批旧路径清单**

~~~bash
rg -n "ui::widgets::|ui::PopupOutcome|ui::PopupMenuWidget" \
  crates/app/src/actions.rs crates/app/src/workspace.rs crates/app/src/events.rs
~~~

Expected: actions 命中 popup_menu/scrollbar/search_bar；workspace 命中 popup_menu；events 命中 title_bar/toc/scrollbar/search_bar/popup_menu/sidebar 及 ui::PopupOutcome。

- [ ] **Step 2: 按精确映射替换**

使用以下一一映射，不改类型名和行为：

~~~text
ui::widgets::popup_menu::*  -> ui::popup_menu::*
ui::widgets::scrollbar::*   -> ui::scrollbar::*
ui::widgets::search_bar::*  -> ui::search_bar::*
ui::widgets::sidebar::*     -> ui::sidebar::*
ui::widgets::title_bar::*   -> ui::title_bar::*
ui::widgets::toc::*         -> ui::toc::*
ui::PopupOutcome            -> ui::popup_menu::PopupOutcome
~~~

例如 actions.rs 顶部必须成为：

~~~rust
use ui::popup_menu::{ContextMenuAction, PopupMenu};
use ui::scrollbar::ScrollbarAction;
use ui::search_bar::SearchBarAction;
~~~

- [ ] **Step 3: 扫描本批并运行事件测试**

~~~bash
rg -n "ui::widgets::|ui::PopupOutcome|ui::PopupMenuWidget" \
  crates/app/src/actions.rs crates/app/src/workspace.rs crates/app/src/events.rs
cargo test -p edit-plus-app --lib events -- --nocapture
cargo test -p edit-plus-app --lib workspace -- --nocapture
cargo check -p edit-plus-app
~~~

Expected: 扫描无输出；测试与编译 PASS。

- [ ] **Step 4: 提交**

~~~bash
git add crates/app/src/actions.rs crates/app/src/workspace.rs crates/app/src/events.rs
git commit -m "refactor(app): use semantic ui action modules"
~~~

### Task 16: 迁移 UiShell、renderer 和 App 的 widget 导入

**Files:**
- Modify/Test: `crates/app/src/ui_shell.rs`
- Modify: `crates/app/src/app_renderer.rs`
- Modify: `crates/app/src/app.rs`

**Interfaces:**
- Consumes: Task 14 根级组件模块；Tasks 9/11/13 的最终 input。
- Produces: app 的每帧 UI 组装路径只使用 ui::<component>。

- [ ] **Step 1: 记录本批所有旧路径**

~~~bash
rg -n "ui::widgets::" crates/app/src/ui_shell.rs crates/app/src/app_renderer.rs crates/app/src/app.rs
~~~

Expected: 命中 scrollbar/search_bar/sidebar/status_bar/tab_bar/title_bar/tooltip/toc/popup_menu。

- [ ] **Step 2: 替换为语义路径并整理 imports**

所有 `ui::widgets::<module>` 机械替换为 `ui::<module>`。ui_shell.rs 顶部必须至少为：

~~~rust
use ui::scrollbar::ScrollbarWidget;
use ui::search_bar::{SearchBarSnapshot, SearchBarWidget};
use ui::sidebar::SidebarWidget;
use ui::status_bar::{StatusBarInput, StatusBarWidget};
use ui::tab_bar::TabBarWidget;
use ui::title_bar::{TitleBarInput, TitleBarWidget};
use ui::tooltip::{TooltipHint, TooltipWidget};
~~~

保留 `use ui::Theme;` 根级基础类型入口。app_renderer 中的 SearchBarSnapshot、StatusBarInput、TitleBarInput、TocInput/TocHeadingEntry 全部从对应组件模块导入或使用完整语义路径。

- [ ] **Step 3: 运行 shell/renderer 测试与扫描**

~~~bash
rg -n "ui::widgets::" crates/app/src/ui_shell.rs crates/app/src/app_renderer.rs crates/app/src/app.rs
cargo test -p edit-plus-app --lib ui_shell::tests -- --nocapture
cargo test -p edit-plus-app --lib app_renderer -- --nocapture
cargo check -p edit-plus-app
~~~

Expected: 扫描无输出；测试与编译 PASS。

- [ ] **Step 4: 提交**

~~~bash
git add crates/app/src/ui_shell.rs crates/app/src/app_renderer.rs crates/app/src/app.rs
git commit -m "refactor(app): use semantic ui shell modules"
~~~

### Task 17: 迁移窗口与搜索路径的 widget 导入

**Files:**
- Modify/Test: `crates/app/src/app_window.rs`
- Modify: `crates/app/src/app_search.rs`
- Modify: `crates/app/src/dispatch/search.rs`

**Interfaces:**
- Consumes: ui::search_bar、ui::sidebar、ui::title_bar。
- Produces: 窗口几何和搜索 dispatch 不依赖 ui::widgets。

- [ ] **Step 1: 记录并替换本批路径**

~~~bash
rg -n "ui::widgets::" \
  crates/app/src/app_window.rs crates/app/src/app_search.rs crates/app/src/dispatch/search.rs
~~~

映射：

~~~text
ui::widgets::search_bar -> ui::search_bar
ui::widgets::sidebar    -> ui::sidebar
ui::widgets::title_bar  -> ui::title_bar
~~~

dispatch/search.rs 最终 import：

~~~rust
use ui::search_bar::SearchBarAction;
~~~

- [ ] **Step 2: 运行窗口/搜索测试与扫描**

~~~bash
rg -n "ui::widgets::" \
  crates/app/src/app_window.rs crates/app/src/app_search.rs crates/app/src/dispatch/search.rs
cargo test -p edit-plus-app --lib app_window -- --nocapture
cargo test -p edit-plus-app --lib dispatch::search -- --nocapture
cargo check -p edit-plus-app
~~~

Expected: 扫描无输出；测试和编译 PASS。

- [ ] **Step 3: 提交**

~~~bash
git add crates/app/src/app_window.rs crates/app/src/app_search.rs crates/app/src/dispatch/search.rs
git commit -m "refactor(app): use semantic ui window modules"
~~~

### Task 18: 迁移 commands/editor/tabs dispatch 的 widget 导入

**Files:**
- Modify: `crates/app/src/dispatch/commands.rs`
- Modify: `crates/app/src/dispatch/editor.rs`
- Modify/Test: `crates/app/src/dispatch/tabs.rs`

**Interfaces:**
- Consumes: ui::popup_menu、ui::sidebar。
- Produces: 三个核心 dispatch 模块不依赖 ui::widgets。

- [ ] **Step 1: 记录并替换本批路径**

~~~bash
rg -n "ui::widgets::" \
  crates/app/src/dispatch/commands.rs crates/app/src/dispatch/editor.rs crates/app/src/dispatch/tabs.rs
~~~

精确映射：

~~~text
ui::widgets::popup_menu -> ui::popup_menu
ui::widgets::sidebar    -> ui::sidebar
~~~

不得改动 AppEffect merge、tab close/pin 规则或 sidebar key 行为。

- [ ] **Step 2: 运行 dispatch 测试与扫描**

~~~bash
rg -n "ui::widgets::" \
  crates/app/src/dispatch/commands.rs crates/app/src/dispatch/editor.rs crates/app/src/dispatch/tabs.rs
cargo test -p edit-plus-app --lib dispatch::commands -- --nocapture
cargo test -p edit-plus-app --lib dispatch::tabs -- --nocapture
cargo check -p edit-plus-app
~~~

Expected: 扫描无输出；测试和编译 PASS。

- [ ] **Step 3: 提交**

~~~bash
git add crates/app/src/dispatch/commands.rs crates/app/src/dispatch/editor.rs crates/app/src/dispatch/tabs.rs
git commit -m "refactor(app): use semantic ui dispatch modules"
~~~

### Task 19: 迁移顶层/chrome/viewport dispatch 的 widget 导入

**Files:**
- Modify: `crates/app/src/app_dispatch.rs`
- Modify/Test: `crates/app/src/dispatch/chrome.rs`
- Modify/Test: `crates/app/src/dispatch/viewport.rs`

**Interfaces:**
- Consumes: Phase 3 后的 chrome/viewport 模块及 Task 14 语义 widget 模块。
- Produces: crates/app/src 生产代码全局无 `ui::widgets::`。

- [ ] **Step 1: 记录本批和全 app 残留**

~~~bash
rg -n "ui::widgets::|use ui::widgets" \
  crates/app/src/app_dispatch.rs crates/app/src/dispatch/chrome.rs crates/app/src/dispatch/viewport.rs
rg -l "ui::widgets::|use ui::widgets" crates/app/src
~~~

Expected: 第一条只命中 popup_menu/sidebar/scrollbar 等组件；第二条的完整允许集合是 `actions.rs`、`app.rs`、`app_dispatch.rs`、`app_renderer.rs`、`app_search.rs`、`app_window.rs`、`events.rs`、`ui_shell.rs`、`workspace.rs`、`dispatch/chrome.rs`、`dispatch/commands.rs`、`dispatch/editor.rs`、`dispatch/search.rs`、`dispatch/tabs.rs`、`dispatch/viewport.rs`，且这些文件分别由 Tasks 15-19 覆盖。

- [ ] **Step 2: 替换本批路径**

~~~text
ui::widgets::popup_menu -> ui::popup_menu
ui::widgets::sidebar    -> ui::sidebar
ui::widgets::scrollbar  -> ui::scrollbar
ui::widgets::tab_bar    -> ui::tab_bar
~~~

同步把 PopupMenuWidget/PopupOutcome 的根级使用改为 `ui::popup_menu::{PopupMenuWidget, PopupOutcome}`。不改变 handler 返回的 AppEffect。

- [ ] **Step 3: 运行 dispatch 测试和全 app 扫描**

~~~bash
cargo test -p edit-plus-app --lib dispatch::chrome -- --nocapture
cargo test -p edit-plus-app --lib dispatch::viewport -- --nocapture
rg -n "ui::widgets::|use ui::widgets" crates/app/src
cargo check -p edit-plus-app --all-targets
~~~

Expected: 测试 PASS；扫描无输出；所有 targets 编译通过。

- [ ] **Step 4: 提交**

~~~bash
git add crates/app/src/app_dispatch.rs crates/app/src/dispatch/chrome.rs crates/app/src/dispatch/viewport.rs
git commit -m "refactor(app): finish semantic ui imports"
~~~

### Task 20: 删除 ThemeRegistry 兼容错误别名

**Files:**
- Modify/Test: `crates/ui/src/theme_registry.rs`
- Modify: `crates/ui/src/theme.rs`

**Interfaces:**
- Consumes: Task 5 后 app 只使用 ThemeLoadError/ThemeRegistrationReport。
- Produces: ui::theme 不再暴露旧 LoadError；ThemeRegistry 无 I/O 错误概念。

- [ ] **Step 1: 扫描旧错误名并建立删除条件**

~~~bash
rg -n "\bLoadError\b|ThemeLoadError" crates/app/src crates/ui/src crates/ui/tests
~~~

Expected: LoadError 只命中 theme_registry.rs alias 和 theme.rs re-export；真实调用均使用 ThemeLoadError。

- [ ] **Step 2: 删除 alias 与 re-export**

删除：

~~~rust
pub type LoadError = ThemeLoadError;
~~~

theme.rs re-export 列表只保留：

~~~rust
pub use crate::theme_registry::{
    BUILTIN_DARK_ID, BUILTIN_LIGHT_ID, RegisterError, ThemeLoadError,
    ThemeRegistrationReport, ThemeRegistry, ThemeSource,
};
~~~

- [ ] **Step 3: 运行主题/公共 API 测试与扫描**

~~~bash
cargo test -p edit-plus-ui --lib theme_registry::tests -- --nocapture
cargo test -p edit-plus-ui --test public_api -- --nocapture
rg -n "\bLoadError\b|std::io|pending|load_pending|eprintln!" crates/ui/src/theme_registry.rs
cargo check -p edit-plus-app
~~~

Expected: 测试 PASS；扫描无输出。

- [ ] **Step 4: 提交**

~~~bash
git add crates/ui/src/theme_registry.rs crates/ui/src/theme.rs
git commit -m "refactor(ui): remove lazy theme error compatibility"
~~~

### Task 21: 私有化实现模块并建立公共边界门禁

**Files:**
- Modify: `crates/ui/src/lib.rs`
- Modify/Test: `crates/ui/tests/public_api.rs`
- Create/Test: `crates/ui/tests/public_boundaries.rs`

**Interfaces:**
- Consumes: Tasks 14-20 已迁移的新公共路径。
- Produces: 私有 widgets/theme_file/hex_color/text_renderer；稳定正向 API 与源码级禁止门禁。

- [ ] **Step 1: 写源码边界失败测试**

创建 `crates/ui/tests/public_boundaries.rs`：

~~~rust
use std::fs;
use std::path::{Path, PathBuf};

fn rust_files(root: &Path) -> Vec<PathBuf> {
    fn visit(dir: &Path, out: &mut Vec<PathBuf>) {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
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
    for declaration in [
        "pub mod widgets;",
        "pub mod theme_file;",
        "pub mod hex_color;",
        "pub mod text_renderer;",
    ] {
        assert!(!lib.contains(declaration), "public implementation module: {declaration}");
    }
}

#[test]
fn ui_has_no_app_types_or_production_filesystem_access() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let source = joined_sources(&root.join("src"));
    for forbidden in ["DocumentView", "Workspace", "AppAction", "AppCommand"] {
        assert!(!source.contains(forbidden), "ui depends on app type {forbidden}");
    }
    for forbidden in ["std::fs", "read_dir(", "read_to_string("] {
        assert!(!source.contains(forbidden), "ui production filesystem use: {forbidden}");
    }
}

#[test]
fn theme_registry_does_not_log_or_load_lazily() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let registry = fs::read_to_string(root.join("src/theme_registry.rs")).unwrap();
    for forbidden in ["eprintln!", "pending", "load_pending", "std::io"] {
        assert!(!registry.contains(forbidden), "theme registry leak: {forbidden}");
    }
}

#[test]
fn app_uses_only_semantic_widget_paths() {
    let ui_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let app_source = joined_sources(&ui_root.parent().unwrap().join("app/src"));
    assert!(!app_source.contains("ui::widgets::"));
    assert!(!app_source.contains("use ui::widgets"));
}
~~~

- [ ] **Step 2: 运行门禁并确认公开实现模块导致失败**

~~~bash
cargo test -p edit-plus-ui --test public_boundaries -- --nocapture
~~~

Expected: FAIL，lib.rs 仍含四个 `pub mod`。

- [ ] **Step 3: 私有化模块并删除组件根级快捷出口**

lib.rs 最终声明：

~~~rust
mod hex_color;
mod text_renderer;
mod theme_file;
mod theme_registry;
mod widgets;

pub mod constants;
pub mod core;
pub mod decorations;
pub mod gutter;
pub mod layout;
pub mod render_geom;
pub mod settings;
pub mod theme;
pub mod view_mode;
pub mod viewport;

pub use widgets::{
    button, icon, list, popup_menu, scrollbar, search_bar, sidebar, status_bar, tab_bar, text_box,
    title_bar, title_bar_spacer, toc, tooltip,
};
~~~

保留 Theme、Settings、ThemeMode、UiMetrics、RenderContext 和 core 基础类型的根级 re-export。删除以下组件级根 re-export block：

~~~rust
pub use widgets::list::{
    ListAction, ListItem, ListItemIndicator, ListItemKind, ListStyle, ListWidget, Orientation,
};
pub use widgets::popup_menu::{PopupMenuWidget, PopupOutcome};
~~~

- [ ] **Step 4: 扩展 public_api 对领域模块代表类型的正向检查**

在 public_api.rs 额外导入：

~~~rust
use ui::gutter::RenderContext;
use ui::render_geom::AdvanceCacheEntry;
use ui::viewport::{LineMap, ScrollAnchor};
~~~

在测试中只做类型检查，避免构造内部数据：

~~~rust
fn assert_public_type<T>() {}
assert_public_type::<RenderContext<'static>>();
assert_public_type::<AdvanceCacheEntry>();
assert_public_type::<ScrollAnchor>();
fn assert_line_map<T: LineMap>() {}
let _ = assert_line_map::<PublicLineMapFixture>;
~~~

测试文件定义完整 fixture：

~~~rust
struct PublicLineMapFixture;

impl LineMap for PublicLineMapFixture {
    fn map_line_count(&self) -> usize { 0 }
    fn map_total_rows(&self) -> usize { 0 }
    fn map_display_to_doc(&self, _display_row: usize) -> usize { 0 }
    fn map_doc_to_display(&self, _doc_line: usize) -> usize { 0 }
    fn visual_line_count(&self, _doc_line: usize) -> u16 { 1 }
}
~~~

- [ ] **Step 5: 运行公共 API、门禁与全 workspace 验收**

~~~bash
cargo fmt --all -- --check
cargo test -p edit-plus-ui --test public_api -- --nocapture
cargo test -p edit-plus-ui --test public_boundaries -- --nocapture
cargo test -p edit-plus-ui --lib
cargo test -p edit-plus-app --lib
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets
~~~

Expected: 全部 PASS。若 clippy 显示整改范围外历史 warning，记录其精确文件/规则；本任务自身不得新增 warning 或 allow。

- [ ] **Step 6: 最终静态核验**

~~~bash
rg -n "std::fs|read_dir|read_to_string" crates/ui/src
rg -n "eprintln!|pending|load_pending" crates/ui/src/theme.rs crates/ui/src/theme_registry.rs
rg -n "DocumentView|Workspace|AppAction|AppCommand" crates/ui/src
rg -n "ui::widgets::|use ui::widgets" crates/app/src
rg -n "^pub mod (widgets|theme_file|hex_color|text_renderer);" crates/ui/src/lib.rs
rg -n '\bSettings\b' crates/ui/src/widgets
~~~

Expected: 前五条无输出；最后一条只允许 cfg(test) 测试 helper 构造 Settings，生产实现不命中。

- [ ] **Step 7: 提交**

~~~bash
git add crates/ui/src/lib.rs crates/ui/tests/public_api.rs crates/ui/tests/public_boundaries.rs
git commit -m "refactor(ui): enforce public component boundaries"
~~~

## 规格覆盖索引

| 规格要求 | 实施任务 | 验证证据 |
|---|---:|---|
| ThemeRegistry 与主题数据文件分离 | 1 | 私有 theme_registry.rs；移动前后测试 |
| eager 完整解析、不可变查询、无日志/I/O | 2、3、20 | Registry 单测；静态扫描；Theme::resolve 编译测试 |
| 继承乱序、未知基类、继承环、失败隔离 | 2 | 结构化 variant 断言与 canonical cycle 测试 |
| reserved/duplicate/clear/list/fallback 确定语义 | 2 | Registry 定向测试与稳定 report 排序 |
| app loader 单文件失败继续、稳定诊断 | 4 | invalid UTF-8、entry error、missing/non-directory 测试 |
| App 保存并统一输出 ThemeLoadReport | 5 | load_user_themes 纯组合测试；App 字段 |
| Tab pin 状态只有 TabInfo.pinned | 6、7、9 | layout 回归、pin cache 扫描 |
| TabBarWidgetInput 单一 owned 输入 | 8、9 | 全字段替换测试；legacy wrapper 删除扫描 |
| SidebarWidgetInput 分离 metrics/behavior | 10、11 | frame snapshot 与 pinned index map 测试 |
| ScrollbarInput 与非有限 scroll 规则 | 12、13 | 输入替换、NaN/Infinity、UiShell 映射测试 |
| 语义组件模块可公开导入 | 14 | external public_api integration test |
| app 不再依赖 ui::widgets | 15-19 | 分批 rg；app all-targets check |
| 隐藏 widgets/theme_file/hex_color/text_renderer | 21 | lib 门禁与 private module 声明 |
| ui 不依赖 app 类型或文件系统 | 21 | public_boundaries integration test |
| workspace tests/check/clippy 验收 | 21 | fmt、两 crate tests、workspace all-targets、clippy |

自审结论：设计规格中的全部范围内要求均映射到至少一个实施任务；无未覆盖条目。

## 完成定义

- ThemeRegistry 注册返回前已完成每个 source 的成功解析或结构化失败。
- Registry 查询只需不可变引用，且无 I/O、日志或 lazy mutation。
- app 对目录项、文件和 Registry 错误全部保存；一个坏文件不阻塞好主题。
- TabBar 只从 TabInfo.pinned 读取 pin 状态，UiShell 不保存第二份 pin HashSet。
- TabBar、Sidebar、Scrollbar 各只有一个 public owned input 入口，所有临时 legacy/frame wrapper 已删除。
- app 生产代码无 ui::widgets 路径。
- ui::widgets、theme_file、hex_color、text_renderer 对外不可见；稳定语义模块可由 integration test 编译。
- 全 workspace tests/check/clippy 完成，且每个实现提交均可编译。
