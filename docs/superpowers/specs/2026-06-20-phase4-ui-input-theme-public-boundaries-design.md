# Phase 4 UI 输入 / 主题加载 / 公共边界设计

## 目标

完成 Phase 4 重构尚未闭环的三条 UI 边界：

1. ThemeRegistry 只负责内存中的解析、依赖解析与查询；文件系统访问、诊断汇总和输出归 app 所有。
2. 复杂 widget 通过单一、显式、拥有所有权的 input struct 接收一帧数据；不再以长参数列表或隐式读取应用状态的方式注入。
3. crates/ui 只暴露稳定的语义模块和必要类型；实现目录、解析辅助模块与内部 widget 容器不再成为公共 API。

本设计不改变主题文件格式和用户可见 UI 行为。它在 Settings/DPI 行为稳定、逻辑 Settings / 物理 UiMetrics 迁移完成后实施，并在 Phase 3 AppEffect 边界完成后进行最终集成验证。

## 现状与缺口

### 主题加载

当前主题加载被分在 app 与 ui，但职责仍未彻底分离：

- app::theme_loader 读取目录和文件，但遇到一个不可读文件会立即返回，导致其余文件不再加载。
- 非 UTF-8 文件名和与内置主题重名的文件被静默跳过。
- ThemeRegistry::register_sources 只用字符串扫描预读 extends/is_dark，并把原始内容放进 pending。
- ThemeRegistry::get/get_or_default 需要 &mut self，并在首次访问时解析、解析依赖和输出 eprintln!。
- 注册时返回的错误列表不能覆盖真正的 TOML、继承和颜色错误，因为这些错误被推迟到了查询阶段。
- 重复 ID 覆盖顺序、继承环和部分失败后的可用主题集合没有明确契约。

结果是：查询操作具有隐藏副作用，错误报告取决于用户是否访问某个主题，且一个坏文件可能影响其他主题的发现。

### Widget 输入

部分 widget 已有清晰输入，例如 SearchBarSnapshot、StatusBarInput、TitleBarInput 和 TocInput；但三处复杂组件仍存在边界重复：

- TabBar 同时维护借用型 TabBarInput、私有 TabBarInputOwned 和含十余个参数的 set_tabs_input。
- Sidebar 已有借用型 SidebarInput，但 SidebarWidget::set_input 仍接收多个独立参数，并在 widget 内保存它们的副本。
- ScrollbarWidget::set_input 通过三个位置参数表达一组不可分割的 viewport 状态。

这些 API 容易在新增字段时漏传、错序或只更新一半，也使 app 到 ui 的一帧快照边界不清晰。

### 公共 API

crates/ui::lib 当前公开 widgets、theme_file、hex_color、text_renderer 等实现模块，并在根模块保留一批历史兼容 re-export。app 中又同时使用 ui::widgets::*、ui::tab_bar::* 和根级 re-export。

这会造成：

- 文件目录结构意外成为跨 crate 契约。
- 内部模块移动会触发 app 大范围修改。
- UI 的真实稳定能力边界无法从 lib.rs 读出。
- Phase 4 要求的 tab_bar、sidebar、scrollbar 等语义模块没有形成一致入口。

## 前置条件与实施顺序

本设计依赖以下两个已批准子项目：

1. `docs/plans-logical-settings-physical-metrics.md` 完成，使 UiMetrics 只包含物理布局数据，SidebarSettingsInput 只包含行为配置。
2. `docs/plans-phase3-app-effect-public-boundaries.md` 完成，使 app action/effect 边界稳定，避免 UI action 迁移与 dispatch 重构交叉进行。

Phase 4 内部顺序固定为：

1. 先完成 ThemeRegistry 的纯内存、 eager 解析和诊断契约。
2. 再完成 app 侧文件读取与诊断汇总。
3. 再迁移 TabBar、Sidebar、Scrollbar 的 input struct。
4. 最后迁移 app import 并收缩 ui::lib 公共模块。
5. 以公共 API 编译测试和静态边界测试封口。

公共 API 收缩必须最后进行，否则中间提交会同时承受行为迁移与 import 迁移，难以审查和回滚。

## 方案比较

### 方案 A：语义模块门面

保留 theme、settings、viewport、layout、render_geom、core 等领域模块；把各 widget 家族直接作为 ui 根下的语义模块；隐藏 widgets 容器和解析辅助模块。

优点：

- 与 AGENTS.md 定义的架构直接一致。
- 使用路径表达能力而不是目录组织。
- app import 迁移机械、可分批验证。
- 后续可在不破坏调用方的情况下继续拆分 widget 内部文件。

缺点：

- 需要一次性迁移当前 ui::widgets::* 使用点。
- lib.rs 需要明确维护允许公开的模块清单。

### 方案 B：全部扁平 re-export 到 ui 根

将所有公共类型直接暴露为 ui::TabBarWidget、ui::SidebarAction 等。

调用路径短，但名称空间很快拥挤，同名 Input/Action 难以区分，也无法表达组件归属。

### 方案 C：只隐藏明显辅助模块

仅把 theme_file 和 hex_color 改为私有，保留 ui::widgets::* 及当前兼容入口。

改动最小，但目录结构仍是公共契约，Phase 4 的模块边界目标没有真正完成。

### 结论

采用方案 A。根级 re-export 只保留高频、跨模块的基础类型；组件类型从对应语义模块导入，不建立第二套扁平 API。

## 总体架构

```text
文件系统 / ~/.config/edit+/themes
              ↓
app::theme_loader（发现、读取、I/O 诊断）
              ↓ ThemeSourceBatch
ui::theme::ThemeRegistry（TOML 解析、继承解析、颜色解析）
              ↓ ThemeRegistrationReport
app::ThemeLoadReport（合并、保存、输出诊断）
              ↓
ThemeRegistry 的不可变查询

App / Workspace / DocumentView
              ↓ 提取一帧纯数据
TabBarWidgetInput / SidebarWidgetInput / ScrollbarInput
              ↓ set_input
UI widget 状态、布局、绘制
              ↓
WidgetAction
              ↓
app dispatch
```

依赖方向始终为 app -> ui。ui 不得依赖 App、Workspace、DocumentView、AppAction 或文件系统。

## ThemeRegistry 设计

### 职责

ThemeRegistry 负责：

- 永久提供两个内置默认主题。
- 接收已经读入内存的 ThemeSource。
- eager 解析完整 TOML。
- 解析默认继承、用户主题继承和继承环。
- 把合法主题注册为 ThemeDefinition。
- 返回全部结构化注册错误。
- 提供无副作用、不可变的主题查询。

ThemeRegistry 不负责：

- 读取目录或文件。
- 判断配置目录位置。
- 直接输出日志或通知。
- 保存 app 级诊断历史。
- 主题热重载或异步加载。

### 数据结构

ThemeSource 保持 app 到 ui 的纯数据协议：

```rust
#[derive(Debug, Clone)]
pub struct ThemeSource {
    pub id: String,
    pub path: PathBuf,
    pub content: String,
}
```

ThemeRegistry 删除 PendingTheme 和 pending 字段：

```rust
pub struct ThemeRegistry {
    themes: BTreeMap<String, ThemeDefinition>,
    default_dark: ThemeDefinition,
    default_light: ThemeDefinition,
}
```

使用 BTreeMap 固化主题 ID 的枚举顺序；注册算法仍显式按 path、id 排序，不依赖调用方传入顺序。

### 注册入口

公开入口为：

```rust
pub fn register_sources(
    &mut self,
    sources: impl IntoIterator<Item = ThemeSource>,
) -> ThemeRegistrationReport
```

返回值：

```rust
#[derive(Debug, Default)]
pub struct ThemeRegistrationReport {
    pub registered_ids: Vec<String>,
    pub errors: Vec<ThemeLoadError>,
}
```

registered_ids 与 errors 都使用稳定顺序。registered_ids 只包含本次成功注册的用户主题，不包含内置主题。

保留 `register(id, definition) -> Result<(), RegisterError>` 作为直接注册完整 ThemeDefinition 的低层 API。RegisterError 明确包含 ReservedId(String) 与 DuplicateId(String)；内置 ID 和已有用户 ID 都被拒绝，不发生静默覆盖。因为该入口没有 source，所以它的错误不进入 ThemeLoadError，也不伪造文件路径。

### Eager 解析算法

register_sources 分为四步：

1. 收集并按 `(path, id)` 排序所有 ThemeSource。
2. 检查内置保留 ID 与重复 ID；同一批中首个 source 获得该 ID，后续 source 产生 DuplicateId 并被忽略。
3. 对候选 source 完整反序列化为内部 ParsedTheme，保留 extends、is_dark 和 ThemeFile。
4. 用三色 DFS 解析继承图并生成 ThemeDefinition。

DFS 状态为 Unvisited、Visiting、Resolved 或 Failed：

- 未显式 extends 时，按 is_dark 选择 default-dark 或 default-light；is_dark 未设置时沿用当前兼容语义，默认 true。
- extends 指向内置主题时直接使用内置 definition。
- extends 指向本批已解析用户主题时递归解析。
- extends 指向 Registry 已存在的用户主题时使用已注册 definition。
- extends 不存在时产生 UnknownExtends。
- 再次遇到 Visiting 节点时，产生包含完整闭环 ID 的 CyclicExtends。
- 基主题 Failed 时，派生主题也产生 BaseThemeFailed；错误中保留派生主题自身的 id/path 和基主题 id。
- ThemeFile::resolve 失败时产生 Resolve。

失败主题不插入 themes；与其无依赖关系的合法主题继续注册。环中的每个主题至多产生一条直接诊断，依赖该环的主题产生 BaseThemeFailed，而不是重复报告同一个环。

display_name 未显式设置且解析结果仍等于基主题名称时，继续使用 source id 作为 display_name，保持现有行为。

### 冲突和替换规则

规则必须确定且不可依赖 HashMap 顺序：

- 内置 ID 永远不可覆盖。
- 同一 register_sources 批次内重复 ID：按排序后的第一个 source 获胜，后续 source 被诊断并忽略。
- source ID 与 Registry 中已存在用户主题冲突：现存 definition 保留，新 source 产生 DuplicateId。
- clear_user_themes 删除全部用户主题，使下一批注册可重新使用这些 ID。
- 一次注册中失败的 ID 不占位；后续独立注册可以再次尝试该 ID。

本设计不提供隐式覆盖。未来热重载如需替换，必须设计显式 replace/reload API。

### 错误类型

ui::theme::ThemeLoadError 只表达内存解析和注册错误，不包含 std::io::Error：

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThemeLoadError {
    ReservedId { id: String, path: PathBuf },
    DuplicateId {
        id: String,
        first_path: Option<PathBuf>,
        duplicate_path: PathBuf,
    },
    TomlParse { id: String, path: PathBuf, message: String },
    UnknownExtends { id: String, path: PathBuf, base_id: String },
    CyclicExtends { ids: Vec<String> },
    BaseThemeFailed { id: String, path: PathBuf, base_id: String },
    Resolve { id: String, path: PathBuf, message: String },
}
```

错误保存可比较的字符串消息而不是第三方错误对象，便于 app 持久保存、测试稳定排序，也避免把 toml 错误类型扩大为公共 API。

CyclicExtends.ids 以词典序最小 ID 为起点，按继承方向排列，并在末尾再次放入起点，例如 `["a", "b", "a"]`。这样相同环从任意 DFS 起点发现都得到相同诊断。

ThemeRegistrationReport 在返回前统一排序：registered_ids 按 ID；errors 按“主路径、错误种类序号、主题 ID、消息”排序。CyclicExtends 没有单一路径，其主路径使用规范化环首 ID 对应 source 的路径。错误种类序号按 enum 在本设计中的声明顺序。路径直接使用 PathBuf 的 Ord，不用有损字符串参与排序。

### 查询契约

查询 API 改为不可变：

```rust
pub fn get(&self, id: &str) -> Option<&ThemeDefinition>

pub fn get_or_default(
    &self,
    id: &str,
    prefer_dark: bool,
) -> &ThemeDefinition
```

查询不解析、不注册、不删除 pending、不输出日志。Theme::resolve 因此接收 `&ThemeRegistry` 而不是 `&mut ThemeRegistry`。

list_ids、len 和 is_empty 只反映成功注册的用户主题；list_ids 另包含两个内置 ID，并保持词典序。失败和重复 source 不出现在查询结果中。

get_or_default 对未知或失败 ID 按 prefer_dark 稳定回退到内置主题，永不返回 None。

## App 侧主题文件加载与诊断

### 文件读取结果

app::theme_loader 不再返回 fail-fast 的 io::Result<Vec<ThemeSource>>，改为：

```rust
#[derive(Debug, Default)]
pub(crate) struct ThemeSourceBatch {
    pub(crate) sources: Vec<ui::theme::ThemeSource>,
    pub(crate) diagnostics: Vec<ThemeSourceDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ThemeSourceDiagnostic {
    DirectoryRead { path: PathBuf, message: String },
    EntryRead { directory: PathBuf, message: String },
    InvalidFileName { path: PathBuf },
    FileRead { path: PathBuf, message: String },
}
```

load_theme_sources(dir: &Path) -> ThemeSourceBatch 的规则：

- 目录不存在是正常的空结果，不产生诊断。
- 目录存在但不可读时返回一个 DirectoryRead，sources 为空。
- 读取单个目录项失败时记录 EntryRead，继续处理其余目录项。
- 只处理扩展名精确为 toml 的普通文件，不递归子目录。
- TOML 路径先排序，再逐个读取，保证 source 顺序稳定。
- TOML 文件 stem 不是有效 UTF-8 时记录 InvalidFileName，继续处理。
- 文件不可读或不是有效 UTF-8 时记录 FileRead，继续处理。
- 与内置主题同名的文件不在 loader 静默过滤；它作为 ThemeSource 交给 Registry，统一产生 ReservedId。

loader 只判断“能否形成内存 source”，不解析 TOML 内容或继承关系。

ThemeSourceBatch 返回前对 diagnostics 排序：先按 DirectoryRead、EntryRead、InvalidFileName、FileRead 的种类序号，再按关联路径，最后按消息。EntryRead 无法得到具体 entry path 时使用 directory 作为路径并以错误消息打破平局。这样 read_dir 的平台遍历顺序不会泄漏到报告顺序。

### App 汇总报告

App 保存一次启动加载的完整报告：

```rust
#[derive(Debug, Default)]
pub(crate) struct ThemeLoadReport {
    pub(crate) source_diagnostics: Vec<ThemeSourceDiagnostic>,
    pub(crate) registry_errors: Vec<ui::theme::ThemeLoadError>,
    pub(crate) registered_ids: Vec<String>,
}
```

App 新增 `theme_load_report: ThemeLoadReport` 字段。初始化流程为：

1. 调用 load_theme_sources。
2. 把 batch.sources 交给 ThemeRegistry::register_sources。
3. 合并 source diagnostics、registry errors 和 registered_ids。
4. 在 app 初始化边界统一输出诊断。
5. 保存报告，供测试和未来 UI 展示使用。

本阶段不新增用户可见错误弹窗。保留结构化报告而不是仅输出字符串，避免未来再从日志反向解析错误。

日志只在 app 初始化边界输出一次；ThemeRegistry 查询和 UI render 路径不得输出主题错误。

## Widget 单一输入设计

### 通用规则

复杂 widget 的帧输入遵守以下约束：

- 一个公共 owned input struct 对应一个 set_input 调用。
- input 只含纯数据、UiMetrics 和其他 ui 层类型，不含 App、Workspace 或 DocumentView。
- app 在 UiShell 输入构造边界从领域对象提取数据。
- widget 可从 input 派生内部借用 view，但不得维护第二个语义重复的公共输入类型。
- 同一个事实在 input 内只有一个来源；例如 tab 的 pin 状态只由 TabInfo.pinned 表达。
- 跨帧交互状态由 widget/state 持有，不塞进 input；一帧外部真值必须全部来自 input。
- input 字段使用带单位语义的名称；物理尺寸沿用 `_px` 或由 UiMetrics 明确提供。

已经具有单一、清晰输入的 SearchBar、StatusBar、TitleBar 和 Toc 保持现状，不为统一命名进行无行为收益的重构。

### TabBarWidgetInput

TabBar 使用一个公共 owned input：

```rust
#[derive(Debug, Clone)]
pub struct TabBarWidgetInput {
    pub tabs: Vec<TabInfo>,
    pub active_index: Option<usize>,
    pub back_enabled: bool,
    pub forward_enabled: bool,
    pub screen_size_px: (f32, f32),
    pub hovered_index: Option<usize>,
    pub scroll_offset_px: f32,
    pub metrics: UiMetrics,
}
```

入口为：

```rust
pub fn set_input(
    &mut self,
    input: TabBarWidgetInput,
    shaper: Option<&mut shaping::Shaper>,
)
```

TabBarWidgetInput 取代私有 TabBarInputOwned。现有 TabBarInput<'_> 若布局纯函数仍需要，可降为 tab_bar 模块私有借用 view；它不再是 app 侧契约。

TabInfo.pinned 是 tab pin 状态的唯一 UI 输入。layout_tabs 不再额外接收 pinned_indices，而是按 TabInfo.pinned 稳定分组和布局。UiShell 删除 tab_input_pinned_indices 缓存；app 构造 TabInfo 时从 Workspace::pinned_indices 写入 pinned 字段一次。这样不存在 TabInfo 与 HashSet 不一致时“排序看一个值、绘制看另一个值”的双真值风险。

hovered_index 与 scroll_offset_px 是 app/UiShell 当前持有并回灌的交互快照，因此属于输入；TabBarState 的内部 preview/menu 状态继续由 widget 持有。

### SidebarWidgetInput

Sidebar 使用：

```rust
#[derive(Debug, Clone)]
pub struct SidebarWidgetInput {
    pub tabs: Vec<TabInfo>,
    pub active_index: Option<usize>,
    pub traffic_light_inset_px: (f32, f32),
    pub screen_size_px: (f32, f32),
    pub metrics: UiMetrics,
    pub settings: SidebarSettingsInput,
}
```

入口为 `pub fn set_input(&mut self, input: SidebarWidgetInput)`。

SidebarSettingsInput 来自逻辑 Settings / 物理 UiMetrics 子项目，只包含 view mode、theme mode 和显示开关等行为状态。SidebarWidgetInput.metrics 只包含物理布局数据，两者不得重新合并。

现有 SidebarInput<'_> 若仅被 SidebarState/layout 内部需要，则改为模块私有借用 view；若已无独立价值则删除。SidebarWidget 仍负责 pinned tab 的稳定排序和原始 workspace index 映射，因为这是组件内部的显示模型。

SidebarConfig 与 SidebarPersistent 继续表达跨帧/跨启动状态，不进入每帧 input。

### ScrollbarInput

Scrollbar 使用：

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollbarInput {
    pub viewport_height_px: f64,
    pub total_display_rows: usize,
    pub scroll_top_rows: f64,
}
```

入口为 `pub fn set_input(&mut self, input: ScrollbarInput)`。

scroll_top 的单位是 display rows，不命名为 `_px`。输入校验和 clamp 规则继续由 ScrollbarState/layout 负责；本设计只消除位置参数歧义，不改变滚动行为。

### App 到 Widget 的数据流

UiShell 每帧构造 inputs：

```text
Workspace tabs/navigation ─┐
App hover/scroll state ────┼─ TabBarWidgetInput ─→ TabBarWidget
UiMetrics ─────────────────┘

Workspace tabs ────────────┐
Window chrome geometry ────┼─ SidebarWidgetInput ─→ SidebarWidget
UiMetrics + behavior input ┘

Viewport state ─────────────── ScrollbarInput ─→ ScrollbarWidget
```

构造完成后不允许 widget 回读 Workspace 或 Settings。widget 通过既有 TabBarAction、SidebarAction、Scrollbar action 输出用户意图，app dispatch 决定领域状态变化和 effect。

## crates/ui 公共模块门面

### 稳定领域模块

以下模块保持公共：

- ui::constants
- ui::core
- ui::decorations
- ui::gutter
- ui::layout
- ui::render_geom
- ui::settings
- ui::theme
- ui::view_mode
- ui::viewport

ui::core 继续作为 UI toolkit 的稳定子门面。现有 app 对 ui::core::geom、paint、widget、text_layout 等路径依赖较多，本阶段不把 core 内部全部扁平化；其内部公开项由 core/mod.rs 自己维护。

### 稳定 Widget 模块

lib.rs 将 widget 家族从私有 widgets 容器转出为根级语义模块：

```rust
mod widgets;

pub use widgets::{
    button, icon, list, popup_menu, scrollbar, search_bar, sidebar,
    status_bar, tab_bar, text_box, title_bar, title_bar_spacer,
    toc, tooltip,
};
```

调用方使用 ui::tab_bar::TabBarWidget、ui::sidebar::SidebarWidgetInput、ui::scrollbar::ScrollbarInput。ui::widgets 路径不再公开。

每个 widget 模块只 re-export 组件使用者需要的 Widget、Input、Action、必要 state/config 和纯布局结果。layout、hit、menu、persistent 等子模块默认私有，只有确属跨 crate 契约的类型才从组件模块门面导出。

### 私有实现模块

以下 lib.rs 模块改为私有：

- theme_file：TOML 反序列化和局部 definition resolve 实现。
- hex_color：颜色字符串解析辅助。
- text_renderer：组件内部文字绘制辅助。
- widgets：widget 文件组织容器。

私有不代表删除。theme.rs 和 widget 实现可以继续通过 crate 内路径使用它们。

### 根级 re-export

保留当前高频基础类型的根级快捷入口：

- Theme。
- Settings、ThemeMode、UiMetrics。
- RenderContext。
- core 中现有几何、事件、布局、绘制和 Widget 基础类型。

删除 ListWidget、PopupMenuWidget 等组件级根 re-export；它们只从 ui::list 和 ui::popup_menu 导入。也不再添加 ui::TabBarWidget 一类扁平快捷入口。

同一个公共类型应只有一个推荐导入路径，避免兼容入口永久化。

## 错误处理与降级行为

- 内置 default-light/default-dark 总是存在，用户主题全部失败也不影响应用启动。
- 一个文件读取失败不阻止其他主题 source 形成。
- 一个 TOML/颜色/继承错误不阻止无依赖的合法主题注册。
- 依赖失败基主题的派生主题被标记为 BaseThemeFailed，不使用半解析 definition。
- 当前 active theme ID 不存在或加载失败时，按外观回退到对应内置主题。
- 重复 ID 不覆盖既有 definition，避免排序或文件系统遍历差异改变实际主题。
- 诊断顺序稳定，测试不依赖平台 HashMap 或 read_dir 顺序。
- 所有诊断由 app 保存和输出；ui 查询/布局/绘制路径无日志副作用。

## 测试设计

### ThemeRegistry 单元测试

- 有效主题在 register_sources 返回前已完成解析，get(&self) 可直接查询。
- 未指定 extends 时按 is_dark 选择正确内置主题。
- 用户主题可继承同批中路径排序更后的主题，证明结果不依赖 source 顺序。
- TOML 错误、未知基主题、颜色错误分别产生结构化诊断。
- 两节点和三节点继承环产生规范化 CyclicExtends。
- 环外依赖环的主题产生 BaseThemeFailed。
- 同一批重复 ID 由排序后首个 source 获胜。
- 与已注册用户 ID 冲突时保留现有 definition。
- 内置 ID 产生 ReservedId。
- 一个无效主题不阻止不相关合法主题注册。
- clear_user_themes 后可重新注册相同 ID。
- list_ids/len/is_empty 只计入成功注册主题。
- get_or_default 对失败/未知 ID 稳定回退。

### App theme_loader 测试

- 不存在目录返回空 batch 且无诊断。
- 普通目录按路径排序返回 TOML sources。
- 非 TOML 文件和子目录被忽略。
- 无效 UTF-8 文件名产生 InvalidFileName，并继续加载其他文件。
- 单个不可读/无效 UTF-8 文件产生 FileRead，并继续加载其他文件。
- 读取目录项失败时产生 EntryRead；若平台难以稳定构造该状态，则把目录项收集逻辑拆成可注入 iterator 的纯函数测试。
- 内置同名 source 不被 loader 静默过滤。

### App 集成测试

- 启动阶段把 loader diagnostics 与 registry errors 全部保存到 ThemeLoadReport。
- App 在含一个坏主题和一个好主题时仍选择并渲染好主题。
- 当前主题失败时回退到正确内置 light/dark。
- 同一份报告只在初始化边界输出，不在多次 resolve 时重复输出。

### Widget 输入测试

- TabBarWidgetInput 产生与迁移前相同的 tab 顺序、active tab、pin、navigation、hover 和 scroll layout。
- TabBar layout 只读取 TabInfo.pinned；UiShell 不再保存第二份 pinned_indices。
- TabBar 在 DPI 1/2 下只使用 metrics.dpi 缩放一次。
- SidebarWidgetInput 保持 pinned-first 稳定排序和 workspace index 映射。
- Sidebar 行为菜单只读取 SidebarSettingsInput，布局只读取 UiMetrics。
- ScrollbarInput 的零行、viewport 大于内容、负值/超大 scroll_top 行为与迁移前一致。
- 每个 set_input 更新全部帧输入，旧字段不残留。

### 公共 API 编译测试

新增 crates/ui/tests/public_api.rs，只通过外部 crate 可见路径导入并实例化代表性类型：

- ui::theme::{ThemeRegistry, ThemeSource, ThemeLoadError}。
- ui::settings::{Settings, UiMetrics} 与 ui::sidebar::SidebarSettingsInput。
- ui::tab_bar::{TabBarWidget, TabBarWidgetInput, TabBarAction}。
- ui::sidebar::{SidebarWidget, SidebarWidgetInput, SidebarAction}。
- ui::scrollbar::{ScrollbarWidget, ScrollbarInput}。
- ui::core、viewport、render_geom 和 gutter 的代表性契约。

该测试保证允许路径可用。禁止路径由 lib.rs 静态门禁保证，因为 Rust compile-pass 测试不能直接表达“某路径必须不可见”。

### 静态边界门禁

最终检查：

```bash
rg -n "std::fs|read_dir|read_to_string" crates/ui/src
rg -n "eprintln!" crates/ui/src/theme.rs crates/ui/src/theme_file.rs
rg -n "DocumentView|Workspace|AppAction|AppCommand" crates/ui/src
rg -n "ui::widgets::|crate::widgets::" crates/app/src
rg -n "^pub mod (widgets|theme_file|hex_color|text_renderer);" crates/ui/src/lib.rs
rg -n '\bSettings\b' crates/ui/src/widgets
```

期望：

- ui 生产代码无文件系统访问。
- ThemeRegistry 无直接日志输出。
- ui 无 app 领域类型。
- app 不使用 ui::widgets 路径。
- lib.rs 不公开实现模块。
- widget 生产代码不直接依赖完整 Settings；测试辅助若需要构造 Settings，必须位于 cfg(test) 范围。

## 验收标准

实现完成需同时满足：

1. ThemeRegistry 不含 pending，get/get_or_default/Theme::resolve 使用不可变 Registry 引用。
2. 所有 source 在 register_sources 返回前完成成功注册或产生结构化错误。
3. app loader 对单文件失败继续执行，App 保存完整 ThemeLoadReport。
4. TabBar、Sidebar、Scrollbar 的 app 注入各自只有一个 input struct，不保留长位置参数入口。
5. UiMetrics 与 SidebarSettingsInput 分工符合逻辑 Settings / 物理 metrics 规格。
6. app 不再导入 ui::widgets::*。
7. theme_file、hex_color、text_renderer、widgets 不再是公共模块。
8. 公共 API 编译测试与静态门禁通过。
9. cargo check --workspace、cargo test --workspace、cargo clippy --workspace --all-targets 通过；若存在整改范围外历史 warning，必须在执行报告中逐项列明，不能用新增 allow 掩盖。
10. 每个中间提交可编译；行为修复先有失败测试，迁移任务每批最多修改三个文件。

## 边界情况

- 空 source 列表不改变 Registry，返回空报告。
- source path 相同但 ID 不同按 path 后的 ID 排序，结果确定。
- 重复 ID 的第一个 source 自身解析失败时，后续重复 source 仍不接管；本批 ID 失败，避免“错误文件是否可解析”改变冲突优先级。
- 用户主题继承先前批次的合法主题可成功；继承先前批次不存在或失败的 ID 产生 UnknownExtends。
- clear_user_themes 后 built-in 仍可用，active pair 不被清空。
- 非 UTF-8 文件内容由 read_to_string 归为 FileRead，不进入 ui TOML parser。
- ThemeFile 中 extends 与 is_dark 的顺序、注释和引号不再影响预解析，因为不再字符串扫描。
- Widget input 为空 tabs、active_index 越界、screen size 为零时保持无 panic，并由现有布局 fallback 处理。
- 一帧中 DPI 与 screen size 同时变化时，input 必须来自同一次 App 快照，不能混用前后两帧值。
- total_display_rows 为零时 scrollbar 隐藏；scroll_top_rows 的 NaN、正负无穷统一归零。迁移前先写刻画测试，迁移后的 input API 必须保持该明确契约。

## 不在本设计范围

- Settings::new 残留与 DPI 回归修复。
- Settings 逻辑单位 / UiMetrics 物理单位迁移本身。
- Phase 3 AppEffect、dispatch 和 app 公共 API 收口。
- 主题文件格式扩展、schema 版本、热重载、目录递归或异步加载。
- 用户可见主题错误面板、通知中心或设置页 UI。
- 改写已有 SearchBarSnapshot、StatusBarInput、TitleBarInput、TocInput 的命名。
- 重构 ui::core 的内部模块布局。
- warning 基线和 CI 工作流的全仓整改；本设计只禁止为本次改动新增掩盖性 allow。

## 设计完成后的计划拆分原则

后续实施计划必须按可独立验证的原子任务拆分，并遵守每项最多修改三个文件：

- ThemeRegistry 数据结构和查询纯化。
- eager TOML 解析与继承图解析。
- ThemeRegistry 错误和确定性测试。
- app loader 非 fail-fast 报告。
- App ThemeLoadReport 集成。
- TabBarWidgetInput 迁移。
- SidebarWidgetInput 迁移。
- ScrollbarInput 迁移。
- widget 语义模块门面与 app import 分批迁移。
- 私有实现模块和公共 API/静态门禁封口。

每个行为变化先写能失败的测试，再实现到测试通过；纯 import 迁移以 cargo check 为最小验证。不得把三个 widget 或主题加载与公共 API 收缩塞进同一个提交。
