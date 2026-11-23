# 视图插件与 Markdown 边界重构计划

> **执行方式**　实施时按子任务使用 `subagent-driven-development` 或 `executing-plans`，逐项核对验收结果。本文件以 `a4243fb` 为核查基线，阶段验收结果随实施记录。

**目标**　用可组合的强类型接口替换视图消息总线，统一 Markdown 编辑坐标和字素边界，并消除已核实的命名冲突与 UI 重复实现。

**架构**　`ViewPlugin` 保留身份、渲染模式、渲染和文档同步。宿主通过独立的能力接口取得编辑、目录、画布和面板能力，通过现有编辑事务提交源码变更。Markdown 在源码快照上共享解析产物，在投影与布局边界显式转换坐标。

**技术环境**　Rust 2024 edition，工作区最低 Rust 1.93，Cargo features，Criterion，Textora 与 Notora 共用 AppKit 宿主。

## 全局约束

- `ui` 定义纯数据输入和通用控件。产品层将文档及会话状态映射为 UI 输入。`appkit-shell` 不新增对 Markdown、Mindmap 或产品 crate 的依赖，Markdown 与 Mindmap 也不互相依赖。
- 文档缓冲区保存权威源码。插件提供编辑计划，宿主继续校验标签页、读写权限、代际和 UTF-8 边界，并沿原入口更新撤销、选区、脏状态及保存回执。保留文件和设置的持久化格式。
- 参照 [Markdown 不变量](../../crates/markdown/docs/invariants.md) 与 [源码快照计划](2026-11-23-markdown-source-snapshot.md)。从现有 `MarkdownSource` 演进，保持代际校验、候选文本隔离和缓存生命周期。
- 子任务按可独立编译验收的切片交付。单任务预计修改超过 3 个文件时，拆成生产者接口、各消费者迁移、旧入口收尾等可编译子任务。兼容入口仅用于迁移，随最后一个消费者删除。发现行为缺陷先运行失败的复现测试，再修复。每次提交前编译通过。
- 每阶段执行表列测试与 `cargo fmt --all -- --check`。涉及跨 crate 协议、持久化或两产品交互的阶段结束时，以及整轮收尾时，执行 `./scripts/verify.sh`。定向测试必须匹配到实际用例，验收新增用例时核对断言和测试数量。

## 阶段顺序

| 阶段 | 改动边界 | 退出验收 |
| --- | --- | --- |
| 0 | 基准与回归用例定位 | 记录当前解析次数和真实视图耗时，锁定双产品行为入口 |
| 1 | 字素分割和内联换行 | 字素边界一致，`<br>` 渲染、插入和删除的契约可验证 |
| 2 | Markdown 内部坐标和物理行 | 投影行与渲染行不能互传，物理行只有一套索引 |
| 3 | 插件核心与编辑、画布、阅读能力 | 两产品调用强类型接口，旧总线仅剩面板两向协议 |
| 4 | 属性面板 | 两产品通过通用面板与事务接口工作，删除旧总线 |
| 5 | 剩余 Markdown 解析与布局成本 | 基准可区分候选校验、提交和重排，有证据的冗余被消除 |
| 6 | 独立命名、命令与 UI 去重 | 逐项验证持久化、产品快捷键及控件行为，完成全仓验证 |

阶段 0 的测试与基准为阶段 5 提供可比较数据。阶段 2 稳定 Markdown 内部坐标后，阶段 3 再替换跨层调用。阶段 4 依赖阶段 3 的能力注册，阶段 6 的机械迁移在接口稳定后进行。

## 阶段 0 · 建立可复用基线

- [x] **解析计数。** 在 `crates/markdown/src/view/source_snapshot_tests.rs` 扩展现有计数测试。用 `MARKDOWN_PARSE_COUNT` 分别记录渲染后的普通 `InsertText` 计划、表格 Enter 计划、选区替换计划、提交后 `set_source` 与下一次渲染、同源重排。用同一原文分别校验生成的替换、提交后字节与光标。已有断言保留：表格 Enter 计划只解析一次候选，改变字号重排原文解析零次，过期语义计划被拒绝。
- [x] **真实视图基准。** 在 `crates/markdown/benches/editing_perf.rs` 添加 `MarkdownEditorView` 场景。初始化 `Shaper`、视图和样本文档，在每轮测量前准备已渲染的独立视图。测量由当前编辑策略生成事务、按事务替换源码、同步新代际并绘制下一帧的耗时，另测无文本变化重排。使用现有混合文档及长 CJK 段落数据，覆盖普通插入和表格 Enter。保留旧基准作为独立规则与布局的对照；本阶段只加测试和基准。

**验收**

```bash
cargo test -p textora-markdown --lib view::source_snapshot_tests
cargo bench -p textora-markdown --bench editing_perf -- --quick
```

记录每条路径的解析次数，以及同一机器、同一构建配置下的基准结果。测试中的替换执行用于隔离测量视图成本，产品事务是否正确由阶段 3 和 4 的宿主测试覆盖。

### 实测基线

基准基线为 `a4243fb`，测试和基准提交为 `ad8b4ec`。Apple M5（10 核）、32 GB、macOS 27.0.1（Darwin arm64），Rust 与 Cargo 1.95.0，Criterion 0.5.1，使用默认 Markdown 特性与 Cargo bench 配置（ThinLTO、16 个 codegen units）。11 个源码快照测试、31 个 Criterion 场景和格式检查均通过。

#### 解析计数

每个编辑用例独立创建并渲染原始视图，共用原始字节，分别验证事务替换、新源码字节及 UTF-8 光标位置。

| 路径 | 计划解析次数 | 新代际同步解析次数 | 更新光标并渲染下一帧解析次数 |
| --- | ---: | ---: | ---: |
| 普通字符插入（策略返回宿主默认） | 0 | 1 | 0 |
| 表格表头 Enter（生成表格骨架） | 1 | 1 | 0 |
| 选中完整首段后输入带前导空格的文本（同一混合原文） | 2 | 1 | 0 |

原有回归条件继续覆盖单段落选区 Enter 和选区替换的候选各解析一次、改变字号的同源重排解析零次、过期语义计划拒绝，以及当前代际的宿主查询解析零次。

#### Criterion 基准耗时

耗时区间按 Criterion 的次序展示下界、估计值和上界。真实视图从独立的已渲染视图开始，复用已初始化的 Shaper。计时包含策略规划、测试侧应用替换和构造源码快照、新代际同步、光标更新与下一帧绘制。重排仅缩窄视口并绘制。表格场景在混合文档开头加了表头，表中的 KiB 为原混合样本的标称尺寸。

| 场景 | 标称 KiB | 下界／估计值／上界 |
| --- | ---: | --- |
| 旧混合文档规则与布局 | 5 | 235.31 µs／236.33 µs／240.41 µs |
| 旧混合文档规则与布局 | 22 | 1.1876 ms／1.2150 ms／1.2218 ms |
| 旧混合文档规则与布局 | 87 | 4.6636 ms／4.7334 ms／4.7508 ms |
| 旧混合文档规则与布局 | 218 | 11.049 ms／11.146 ms／11.535 ms |
| 旧长 CJK 段落布局 | 10 | 35.077 µs／35.469 µs／35.567 µs |
| 旧长 CJK 段落布局 | 40 | 143.86 µs／144.10 µs／145.06 µs |
| 旧长 CJK 段落布局 | 82 | 281.03 µs／282.86 µs／283.32 µs |
| 旧长 CJK 段落布局 | 163 | 553.77 µs／559.49 µs／560.92 µs |
| 旧长 CJK 段落布局 | 326 | 1.2656 ms／1.3110 ms／1.3223 ms |
| 真实编辑视图 · 混合文档插入 | 5 | 1.5601 ms／1.5688 ms／1.6035 ms |
| 真实编辑视图 · 混合文档插入 | 22 | 6.0043 ms／6.0546 ms／6.2559 ms |
| 真实编辑视图 · 混合文档插入 | 87 | 53.769 ms／56.050 ms／56.621 ms |
| 真实编辑视图 · 混合文档插入 | 218 | 259.41 ms／260.16 ms／263.13 ms |
| 真实编辑视图 · 混合文档表格 Enter | 5 | 1.6747 ms／1.6772 ms／1.6778 ms |
| 真实编辑视图 · 混合文档表格 Enter | 22 | 7.3864 ms／7.4003 ms／7.4558 ms |
| 真实编辑视图 · 混合文档表格 Enter | 87 | 57.191 ms／57.825 ms／60.359 ms |
| 真实编辑视图 · 混合文档表格 Enter | 218 | 280.86 ms／284.14 ms／297.26 ms |
| 真实编辑视图 · 混合文档重排 | 5 | 1.3445 ms／1.3813 ms／1.3905 ms |
| 真实编辑视图 · 混合文档重排 | 22 | 3.4018 ms／3.4047 ms／3.4164 ms |
| 真实编辑视图 · 混合文档重排 | 87 | 17.981 ms／18.397 ms／18.501 ms |
| 真实编辑视图 · 混合文档重排 | 218 | 80.439 ms／81.089 ms／83.689 ms |
| 真实编辑视图 · 长 CJK 段落插入 | 10 | 7.5716 ms／7.7161 ms／7.7523 ms |
| 真实编辑视图 · 长 CJK 段落插入 | 40 | 33.555 ms／35.053 ms／35.427 ms |
| 真实编辑视图 · 长 CJK 段落插入 | 82 | 66.871 ms／67.562 ms／70.326 ms |
| 真实编辑视图 · 长 CJK 段落插入 | 163 | 150.89 ms／151.25 ms／152.69 ms |
| 真实编辑视图 · 长 CJK 段落插入 | 326 | 376.49 ms／376.79 ms／378.02 ms |
| 真实编辑视图 · 长 CJK 段落重排 | 10 | 6.9907 ms／7.0934 ms／7.1190 ms |
| 真实编辑视图 · 长 CJK 段落重排 | 40 | 28.713 ms／28.946 ms／29.877 ms |
| 真实编辑视图 · 长 CJK 段落重排 | 82 | 71.219 ms／71.499 ms／72.617 ms |
| 真实编辑视图 · 长 CJK 段落重排 | 163 | 159.60 ms／162.28 ms／162.95 ms |
| 真实编辑视图 · 长 CJK 段落重排 | 326 | 419.29 ms／433.69 ms／437.29 ms |

编辑计时包含两次测试侧 `StringDocView` 构造，重排包含一次。每次构造扫描源码并建立行索引，复测沿用相同计时范围。Criterion 原始结果保存在 `target/criterion` 的 `stage_0` 命名快照中。

## 阶段 1 · 统一 Markdown 字素与换行

### 1.1 共用字素边界

- [x] **先复现。** 在 `crates/markdown/src/grapheme_map.rs` 和 `crates/markdown/src/view/view_tests.rs` 加入 `a\u{1ACF}b` 与 `a\u{00AD}b`，对比行末命中、逐簇移动、Backspace、Delete 与 Core 字节边界。两个样例在当前 Core 分别是 `[0, 1, 4, 5]` 和 `[0, 3, 4]`，断言编辑后的源码与光标，而不只比较字素数量。补测组合音标、ZWJ emoji、旗帜、CRLF 和空串的末尾哨兵。
- [x] **按消费者迁移。** 以 `textora_core::unicode::grapheme_start_indices` 和 `grapheme_byte_boundaries` 为准，先改 `grapheme_map.rs` 与 `view/mod.rs` 的备用几何，再按模块迁移 `view/preedit.rs`、`view/source_search.rs` 和 `augmenter/` 中参与编辑坐标计算的调用。需要字符串切片时从同一算法取得边界，避免为每次移动分配整篇边界表。独立审查 `layout/ascii_diagram.rs` 的分列和宽度语义，确认用同一边界后再从 `crates/markdown/Cargo.toml` 移除多余直接依赖。此次以现有 Core 版本为基准，Unicode 升级另议。

### 1.2 统一 `<br>` 识别、布局和删除范围

- [x] **先复现长标签。** 在 `crates/markdown/src/augmenter/augmenter_tests.rs` 使用 `before<br            />after`，断言在标签末尾 Backspace、开头 Delete 均删除完整的 `6..23`，光标落在字节 6，并保留前后文字。用 `builder_tests.rs` 确认同一标签产生逻辑换行，覆盖大小写、属性、代码字面量、HTML 块和内联样式跨换行。
- [x] **从解析产物派生范围。** 在 `crates/markdown/src/parser.rs` 建立由已识别 `InlineHtml` 事件得出的内联换行字节范围，支持按两端查找。`builder.rs` 和 `augmenter/backspace.rs` 消费同一结果；删掉固定 16 字节扫描。这个子任务保留各构建配置现有的 feature 行为。
- [x] **独立提交精简配置变化。** 在 `parser.rs` 和 `augmenter/mod.rs` 去掉内联 `<br>` 识别和标题、表格 Shift+Enter 对 `rich-markdown` 的依赖，同时清除删除路径的同类条件。更新 `lib_tests.rs`，明确精简配置下 `<br>` 变为换行，数学、Mermaid 与非换行 HTML 保持精简语义。为 ATX、Setext 和表格增加双配置的插入、删除与一次撤销往返测试。将这一子任务标为兼容行为变更单独审查。

**验收**

```bash
cargo test -p textora-core --lib unicode
cargo test -p textora-markdown --lib
cargo test -p textora-markdown --lib --no-default-features
cargo test -p textora-markdown --test system_boundary_audit -- --include-ignored
cargo check -p textora-app --no-default-features --features markdown
```

验收以两端都能用合法 UTF-8 边界定位为准。`<br>` 往返验证精确源码、容器归属和光标落点。两种 Markdown 配置都运行新增用例。

阶段验收通过。Core Unicode 11 项、默认 Markdown 1,335 项、精简 Markdown 1,215 项、系统边界审计 33 项，Textora Markdown-only 构建与格式检查均通过。

精简配置的内联换行兼容变更由 `eec0547` 独立审查。`93aa145` 的回归验证普通尖括号旁的 Backspace 保持零次解析，任意长度的标签仍由解析事件决定删除范围。

## 阶段 2 · 锁定内部坐标并复用物理行索引

- [x] **类型化投影与渲染行。** 在 `crates/markdown/src/projection.rs` 和 `selection.rs` 明确 `VisualPosition` 使用 `ProjectionLineIdx`，`ViewPos` 使用 `FlatLineIdx`，两者保留各自的顺序与生命周期。`layout/lazy_projection.rs` 输出 `Option<FlatLineIdx>` 和 `Option<ProjectionLineIdx>`，`view/engine_navigation.rs` 与选区、搜索调用方通过这些方法转换。先加类型化入口，再迁移各调用方，最后移除接受裸整数的转换。覆盖视口外投影、无投影渲染行、重排后布局版本过期和软折行亲和性。
- [x] **类型化源码与字素。** 在投影边界使用 `SourceByte`、`GraphemeIdx`，将 `SourceAnchor` 的源码字节、亲和性与 IME 虚拟字素序号继续分开。向命中、选择、虚拟文本和搜索扩展时，用窄作用域构造器。解析器范围、字符串切片和现有共享事务在明确边界解包。验收检查投影索引与渲染索引不能通过隐式转换互传，`source_anchor_at` 对过期代际、过期布局版本和无编辑投影继续返回区分明确的错误。
- [x] **共享一份物理行索引。** 将 `layout/source_line_map.rs` 中的物理范围按需缓存于 `source_snapshot.rs`，供 `view/mod.rs` 查询，并移除重复扫描和 `SourceLineAtByte`。物理范围显式区分正文末端与换行末端，保留 CRLF、EOF、空输入和尾空行语义。将 `augmenter/source_slice.rs` 的同类查询逐个对接；`editable_paragraphs.rs` 的 `EmptySourceLine` 继续表达可编辑候选及容器归属，`edit.rs` 的 `SourceLineContext` 继续表达逻辑行或单元格范围。

**验收**

```bash
cargo test -p textora-markdown --lib projection
cargo test -p textora-markdown --lib source_line_map
cargo test -p textora-markdown --lib replacement_preedit
cargo test -p textora-markdown --test system_boundary_audit -- --include-ignored
cargo test -p textora-markdown --lib --no-default-features
```

用 LF 和 CRLF 的空行、引用前缀、表格单元格和 EOF 测试验证索引查询，与原有回归一起比较点击命中、选区边界和重排结果。单独审查类型化方法签名，确保错误坐标在类型边界被拒绝。

阶段验收通过。投影 45 项、物理行相关 22 项、替换预编辑 11 项、源码快照 11 项、系统边界审计 33 项、精简 Markdown 1,222 项和格式检查均通过。同源重排只建一次物理行索引。选词回归覆盖组合附加符和 ZWJ 表情，并核对选中源码与字素端点。

## 阶段 3 · 逐组替换视图插件协议

### 3.1 核心和能力发现

- [x] 在 `crates/ui/src/plugin.rs` 保留核心 `ViewPlugin::{name, mode, render, sync_document}`。使用 `PluginMode::{Source, Reading, RichText, Canvas}` 表达四条渲染路线，避开窗口的 `ui::view_mode::ViewMode`。将能力发现放在对象安全的 `ViewPluginCapabilities: ViewPlugin` 上，由 `PluginFactory` 返回 `Box<dyn ViewPluginCapabilities>`。通过 `editable()`、`canvas_mut()`、`outline()`、`outline_mut()` 等显式访问器返回同一实例的借用，逐组添加所需的读写访问器。保留 `TabRuntime` 对同一插件的所有权及缓存复用。迁移期在 `ViewPluginCapabilities` 上保留尚未迁移的查询和消息入口，最后一批消费者切换后删除。
- [x] 迁移 `crates/appkit-shell/src/editor_plugin.rs`、Markdown/Novel、`crates/mindmap/src/mindmap_view.rs` 及两产品渲染消费者。按模式选择宿主绘制还是自绘，明确行号和光标闪烁的所有权。编辑许可继续取插件行为与 `DocumentEditingAccess` 的交集。用测试替身验证四条路线、只读画布、宿主默认编辑，以及阅读视图可选择与查找。
- [x] 将 `NeedsSourceUpdate` 和 `UpdateSource` 迁为 `sync_document`。在 `crates/appkit-shell/src/editor_runtime/model_session.rs`、`editor_painter.rs` 与 `crates/app/src/app_renderer.rs` 统一传入代际、普通或强制同步原因和延迟取全文的入口。激活、替换、撤销重做及复用缓存插件时保留当前强制刷新场景；普通重绘、源码编辑及直接读 `DocView` 的 Novel 避免为同步拷贝全文。源码更新和选区同步分开，通过测试锁定拖拽时禁止反向覆盖、软折行亲和性及 IME 生命周期。

### 3.2 迁移各能力

- [x] **编辑与输入。** `EditableView` 提供 `plan_edit`、`plan_semantic_edit`、`hit_test_caret` 和 `visual_move`，按需要保留文本光标与对象目标的不同返回类型。将粘贴偏好、按键映射和 IME 通知接到明确的输入接口。复用现有 `EditPolicy` 行为，保留 `UseDefault`、`Unsupported`、`NoChange` 与过期事务各自语义。先改 `tab_session.rs` 和 `model_session.rs` 的调用，再迁移插件及 Textora 的插表探测、两个产品的鼠标与键盘入口。Markdown 的表格能力与源码字距保护也改用明确的类型化查询。
- [x] **画布。** `CanvasView` 收纳 `prepare_canvas`、`render_canvas`、`handle_canvas_drag` 及控制命中和指针状态。迁移 `editor_painter.rs`、`editor_pointer.rs`、`crates/app/src/dispatch/mouse.rs` 与 Mindmap 实现。保持准备内容尺寸、求解视口、渲染的顺序；拖拽预览只改变视图，释放才提交一次事务，取消和过期计划均不写入。
- [x] **阅读与目录。** 从 `crates/markdown/src/view/engine_protocol.rs`、`novel_view.rs` 提取 `OutlineProvider` 的 `headings`、`scroll_to_heading`、当前章节和章节跳转能力。单独迁移滚动恢复、查找高亮、阅读选区和词行命中到各自类型化接口，两产品继续使用相同的回退行为。保留 Novel 在首章向上跳转时归零的现有行为。
- [x] **收尾。** 按每组生产者和消费者清除已迁移枚举变体、通用解包函数和测试替身的旧实现。此时旧查询与响应仅剩 `MindmapThemeSelection` 和 `PlanMindmapTheme` 两向面板协议。面板迁移完成后统一移除。

**验收**

```bash
cargo test -p textora-appkit-shell --lib
cargo test -p textora-appkit-shell --test editor_runtime_fake_product
cargo test -p textora-markdown --lib
cargo test -p textora-mindmap --lib
cargo test -p textora-app --lib canvas_drag
cargo test -p notora-app --lib -- --test-threads=1
./scripts/verify.sh
```

重点核对 `replace_document_synchronizes_even_when_buffer_generation_is_reused`、`undo_synchronizes_plugin_source_without_waiting_for_a_paint`、`edit_planning_synchronizes_stale_plugin_source_before_planning`、`keyboard_tab_switch_commands_cancel_started_canvas_drag_once`。补一条 Textora 与 Notora 各自从富文本切回源码、再恢复插件的回归，断言源码、代际、选区和撤销边界。

阶段验收通过，`./scripts/verify.sh` 退出 0。Textora 与 Notora 各通过一条富文本与源码往返回归，涵盖源码、代际、选区和两个独立撤销边界。Novel 首章向上跳转回归验证实际滚动位置归零。该阶段结束时，过渡查询仅剩属性面板的旧主题协议。

## 阶段 4 · 通用属性面板与思维导图接入

- [x] **定义最小契约。** 在 `crates/ui/src/widgets/inspector_panel.rs` 定义纯数据的 `InspectorPanelDescriptor`、面板和条目 ID、预设组、禁用状态、说明和选择动作。描述符只覆盖现有预设卡片、分组与状态提示所需形态。`InspectorPanelProvider` 暴露只读描述与 `plan_inspector_action`，以当前代际生成 `EditPlan`。AppKit Shell 持有通用的每标签页开合和焦点状态，沿现有事务入口提交。
- [x] **迁移提供者与双产品。** 将 MMF 主题的默认、未知、非法元数据判断与写入计划收敛到 `crates/mindmap/src/mindmap_view.rs` 及已有 `mmf/edit.rs`。用纯数据交给 UI。把 `crates/app/src/app_renderer.rs`、`app_dispatch.rs`、`app_lifecycle.rs`、`ui_shell.rs` 和 `crates/notora-app/src/runtime/`、`render/event_routing.rs` 按产品分两批迁入通用挂载与事件路由。保留现有预设样式、Dock 占位、菜单、焦点、键盘优先级和 IME 消费。保留文件中的旧主题 ID。
- [x] **去掉专用链路。** 两端迁移后，移除 `crates/appkit-shell/src/mindmap_style_panel/` 的专用控件、Shell 主题方法、`ui` 的思维导图专用面板动作和标题栏专用事件。`MindmapThemeSelection` 若仍供解析用，移到 `mindmap`。最后删除 `PluginQuery`、`PluginResponse`、`PluginMessage` 及转发 API，并迁移仍直接构造旧协议的集成测试。

**验收**

```bash
cargo test -p textora-mindmap --lib
cargo test -p textora-appkit-shell --lib
cargo test -p textora-app --lib mindmap
cargo test -p notora-app --lib -- --test-threads=1
./scripts/verify.sh
```

迁移或替换 `switching_tabs_restores_each_style_panel_session`、`mindmap_style_panel_ime_route_consumes_preedit_and_commit_only`、`mindmap_style_selection_executes_one_transaction_and_syncs_after_text_change` 与过期计划回归。为两产品验证未知主题回退、无效元数据禁用、选中当前值及过期请求均零写入、合法预设恰好一次源码替换和一次撤销。断言 Notora 的自动保存沿用原回执，普通文档无面板权限。旧总线符号在生产和测试代码均移除。

**执行记录**。UI 属性面板 16 项，思维导图 202 项，Shell 492 项，Textora 思维导图及面板定向共 26 项，Notora 库 472 项通过。Shell 保留每标签页开合和焦点，用当前挂载、标签 ID、文档代际与内容修订匹配 UI 动作。两产品验证有效主题写入与撤销、无效和过期请求零写入。Notora 验证自动保存回执和资源版本。旧消息及专用面板动作在 Rust 代码中无引用。全量 `./scripts/verify.sh` 通过，含边界审计、格式、Clippy、产品测试与工作区测试。

## 阶段 5 · 用真实路径消除剩余的重复工作

- [x] **按文本身份归因。** 用阶段 0 的样本复测，区分当前原文、擦除选区后的临时稿、待验证候选、已提交文本和 IME 临时稿。分别记录解析次数、结构构建和重排耗时。检查 `crates/markdown/src/source_snapshot.rs`、`view/editor_view.rs`、`augmenter/mod.rs` 和 `commands/mod.rs` 调用链，保留现有的零次原文重解析断言。
- [x] **仅复用已证实重复的产物。** 同一计划内有相同候选文本时，复用同一个 `MarkdownSource`。若基准确认完整候选已为提交文本全量解析，且提交同步再次解析同一字节串，再由 Markdown 编辑视图保留至多一份待提交候选。只有计划来源代际与本次变更一致且同步到的全文逐字节一致才可复用。测试同长异文、撤销、取消、外部替换、代际复用和 IME 隔离。必要的候选结构验证继续执行。若重复成本落在结构树或 `ensure_all_blocks`，以单独的布局切片优化，比较冷启动与复用布局输出。

**验收**

```bash
cargo test -p textora-markdown --test table_creation_projection_acceptance
cargo test -p textora-markdown --lib view::source_snapshot_tests
cargo test -p textora-markdown --lib --no-default-features
cargo bench -p textora-markdown --bench editing_perf -- --quick
```

记录基线与复测的每个场景解析次数和测量波动。当前快照的同源重排继续为零次解析。若移除候选与提交间的重复解析，计数测试应能证明相同最终文本减少一次完整解析，异文本仍重新解析。结构、精确源码和撤销行为与基线一致，性能结论以实测差异为准。

表格投影验收通过，Textora 的表头事务和撤销、重做集成用例通过。Markdown 默认 1352 项及精简 1232 项通过。格式和差异检查通过。

### 实施与复测

正常提交以请求来源代际、下一代际和全文逐字节比较识别同一个表头 Enter 验证候选。保留表数量实际增加的验证。常驻槽最多一份，新计划、强制同步和 IME 清除旧候选。用文档的一个事务验证一次撤销和一次重做，源码与光标均精确恢复。额外的变宽切片按同一快照身份移动已有可编辑结构树，再计算新几何。回归对比冷启动的行文本、矩形、源码投影、高度和光标，并断言同源变宽零解析、零重新构树。

| 文本身份与阶段 | 解析次数（改动前 → 改动后） | 构树与几何 |
| --- | ---: | --- |
| 已打开原文，进入编辑计划 | 0 → 0 | 插入和表头 Enter 无构树。整段选区首次取得原文用于规则的语法树一次 |
| 擦除选区的临时稿与段落保护候选 | 2 → 2 | 两份内容不同，擦除稿构树一次 |
| 表头 Enter 完整验证候选 | 1 → 1 | 校验读取解析事件，候选阶段零构树 |
| 普通插入、选区替换的提交全文 | 1 → 1 | 提交同步各构树、粗排一次 |
| 表头 Enter 提交全文 | 1 → 0 | 与验证候选逐字节相同，提交仍需构树、粗排 |
| 各编辑提交后的视觉帧 | 0 → 0 | 光标位置变化可触发受影响块的粗排，随后进行可见区域精排；帧不重新构树 |
| 变宽重排的原文 | 0 → 0 | 编辑结构构建 1 → 0；新宽度的粗排和精排保留 |
| 有选区的 IME 临时稿 | 1 → 1 | 输入事件不解析，临时帧独立构树、粗排和精排 |

在四档混合文档样本上用调试构建按文本指纹采集诊断数据，下面是 5、22、87、218 KiB 的单次采样。计时区间可能嵌套，表中的粗排、精排与整帧耗时不可相加。性能结论使用下方 Criterion 优化构建的数据。

| 变宽阶段 | 5 KiB（前 → 后） | 22 KiB（前 → 后） | 87 KiB（前 → 后） | 218 KiB（前 → 后） |
| --- | ---: | ---: | ---: | ---: |
| 结构构建 | 626.75µs → 0 | 2.741208ms → 0 | 10.96275ms → 0 | 27.716416ms → 0 |
| 结构性粗排 | 845µs → 831.125µs | 3.799709ms → 3.724542ms | 14.895833ms → 14.69075ms | 36.993792ms → 36.9005ms |
| 视口精排 | 9.7565ms → 9.827333ms | 9.813083ms → 9.857916ms | 10.034417ms → 9.906458ms | 9.860834ms → 9.841458ms |
| 重排整帧 | 16.152833ms → 15.426166ms | 30.371917ms → 26.623375ms | 126.165917ms → 111.116292ms | 526.147708ms → 480.856458ms |
| 其他关键阶段 | 5 KiB（前 → 后） | 22 KiB（前 → 后） | 87 KiB（前 → 后） | 218 KiB（前 → 后） |
| --- | ---: | ---: | ---: | ---: |
| 表格候选解析 | 298.833µs → 310.125µs | 1.188959ms → 1.186875ms | 4.715791ms → 4.655375ms | 11.753334ms → 11.680667ms |
| 表格提交解析 | 275.584µs → 0 | 1.17725ms → 0 | 4.671417ms → 0 | 11.713708ms → 0 |
| 表格提交构树 | 626.834µs → 642.584µs | 2.742333ms → 2.740959ms | 11.195958ms → 11.184208ms | 27.576667ms → 27.533125ms |
| 表格提交粗排 | 404.666µs → 402.25µs | 1.404417ms → 1.370208ms | 5.261ms → 5.336167ms | 13.052167ms → 13.086083ms |
| 选区提交构树 | 629.875µs → 629.75µs | 2.784417ms → 2.867791ms | 10.945917ms → 10.982167ms | 27.62175ms → 27.811542ms |
| 选区提交粗排 | 378.375µs → 383.042µs | 1.361167ms → 1.361833ms | 5.195959ms → 5.24375ms | 13.181083ms → 13.219125ms |
| IME 临时稿解析 | 304µs → 284.917µs | 1.196458ms → 1.243708ms | 4.634875ms → 4.674041ms | 11.607083ms → 11.543291ms |
| IME 临时稿构树 | 647.875µs → 635.792µs | 2.753125ms → 2.794791ms | 10.907833ms → 11.046959ms | 27.4065ms → 27.704791ms |
| IME 临时稿粗排 | 847.541µs → 830.75µs | 3.616ms → 3.662458ms | 14.526459ms → 14.581541ms | 37.030042ms → 36.599333ms |
| IME 临时稿精排 | 11.087ms → 9.758666ms | 9.800916ms → 10.127541ms | 9.884625ms → 9.917709ms | 10.001042ms → 10.153083ms |

#### Criterion 复测

与阶段 0 使用相同的构建配置和样本。阶段 5 基线和改动后均走生产的 Normal 同步入口，并新增四档全文首段选区替换。每格为 Criterion 的下界／估计值／上界，最后一列是对表头 Enter 与同源变宽的第二次快速测量。旧微基准和长 CJK 样本继续作为对照。

| 场景 | 改动前 | 阶段 5 最终 | 重复测量 |
| --- | --- | --- | --- |
| `mixed_document_single_key/5` | 222.36 µs／226.79 µs／227.90 µs | 257.69 µs／258.17 µs／260.07 µs | — |
| `mixed_document_single_key/22` | 1.0658 ms／1.0664 ms／1.0685 ms | 1.0791 ms／1.0905 ms／1.1361 ms | — |
| `mixed_document_single_key/87` | 4.4000 ms／4.4327 ms／4.5634 ms | 4.7594 ms／4.7952 ms／4.9384 ms | — |
| `mixed_document_single_key/218` | 11.554 ms／11.611 ms／11.841 ms | 11.973 ms／12.050 ms／12.360 ms | — |
| `long_cjk_paragraph_layout/10` | 30.112 µs／30.112 µs／30.113 µs | 34.286 µs／34.332 µs／34.515 µs | — |
| `long_cjk_paragraph_layout/40` | 129.98 µs／130.16 µs／130.87 µs | 133.77 µs／134.70 µs／138.43 µs | — |
| `long_cjk_paragraph_layout/82` | 263.32 µs／263.41 µs／263.80 µs | 271.23 µs／273.07 µs／280.42 µs | — |
| `long_cjk_paragraph_layout/163` | 523.38 µs／526.32 µs／538.11 µs | 559.44 µs／563.42 µs／579.30 µs | — |
| `long_cjk_paragraph_layout/326` | 1.0645 ms／1.0656 ms／1.0701 ms | 1.0889 ms／1.0931 ms／1.1103 ms | — |
| `markdown_editor_mixed/insert/5` | 1.4945 ms／1.5086 ms／1.5653 ms | 1.5422 ms／1.5436 ms／1.5492 ms | — |
| `markdown_editor_mixed/reflow/5` | 1.3007 ms／1.3021 ms／1.3074 ms | 1.1745 ms／1.1853 ms／1.2286 ms | 1.0761 ms／1.0783 ms／1.0870 ms |
| `markdown_editor_mixed/insert/22` | 5.3178 ms／5.3228 ms／5.3430 ms | 5.6940 ms／5.7496 ms／5.9719 ms | — |
| `markdown_editor_mixed/reflow/22` | 3.2560 ms／3.2682 ms／3.3171 ms | 2.7321 ms／2.7456 ms／2.7994 ms | 2.4606 ms／2.4763 ms／2.5391 ms |
| `markdown_editor_mixed/insert/87` | 45.805 ms／46.098 ms／47.271 ms | 47.070 ms／47.593 ms／49.685 ms | — |
| `markdown_editor_mixed/reflow/87` | 17.592 ms／17.715 ms／18.206 ms | 15.956 ms／16.083 ms／16.593 ms | 14.530 ms／14.673 ms／15.246 ms |
| `markdown_editor_mixed/insert/218` | 242.48 ms／243.02 ms／245.19 ms | 268.31 ms／271.22 ms／282.83 ms | — |
| `markdown_editor_mixed/reflow/218` | 71.915 ms／72.137 ms／73.025 ms | 73.206 ms／73.379 ms／74.070 ms | 67.273 ms／67.986 ms／70.839 ms |
| `markdown_editor_long_cjk/insert/10` | 7.1964 ms／7.2036 ms／7.2326 ms | 7.3144 ms／7.3573 ms／7.5292 ms | — |
| `markdown_editor_long_cjk/reflow/10` | 6.8955 ms／6.9359 ms／7.0974 ms | 7.1561 ms／7.3297 ms／7.3731 ms | 6.6225 ms／6.6465 ms／6.6526 ms |
| `markdown_editor_long_cjk/insert/40` | 30.497 ms／30.657 ms／31.296 ms | 30.888 ms／31.105 ms／31.975 ms | — |
| `markdown_editor_long_cjk/reflow/40` | 30.189 ms／30.418 ms／31.336 ms | 31.222 ms／31.378 ms／32.002 ms | 30.238 ms／30.522 ms／31.657 ms |
| `markdown_editor_long_cjk/insert/82` | 61.476 ms／61.956 ms／63.878 ms | 57.872 ms／58.228 ms／58.317 ms | — |
| `markdown_editor_long_cjk/reflow/82` | 64.154 ms／64.643 ms／66.598 ms | 59.172 ms／59.798 ms／62.301 ms | 59.807 ms／60.148 ms／61.511 ms |
| `markdown_editor_long_cjk/insert/163` | 144.87 ms／146.10 ms／150.98 ms | 134.14 ms／134.40 ms／135.42 ms | — |
| `markdown_editor_long_cjk/reflow/163` | 141.80 ms／143.26 ms／149.10 ms | 139.43 ms／139.48 ms／139.67 ms | 140.16 ms／141.00 ms／144.35 ms |
| `markdown_editor_long_cjk/insert/326` | 335.39 ms／336.66 ms／341.73 ms | 334.77 ms／345.16 ms／347.76 ms | — |
| `markdown_editor_long_cjk/reflow/326` | 379.10 ms／382.96 ms／398.42 ms | 373.72 ms／375.66 ms／376.15 ms | 372.12 ms／377.53 ms／378.88 ms |
| `markdown_editor_mixed/table_enter/5` | 1.5306 ms／1.5405 ms／1.5798 ms | 1.4537 ms／1.4600 ms／1.4850 ms | 1.4554 ms／1.4653 ms／1.5049 ms |
| `markdown_editor_mixed/table_enter/22` | 5.8888 ms／5.8995 ms／5.9423 ms | 5.3419 ms／5.3723 ms／5.4940 ms | 5.5854 ms／5.6297 ms／5.8072 ms |
| `markdown_editor_mixed/table_enter/87` | 50.625 ms／51.029 ms／52.642 ms | 48.296 ms／48.833 ms／50.983 ms | 49.581 ms／50.044 ms／51.900 ms |
| `markdown_editor_mixed/table_enter/218` | 255.83 ms／260.65 ms／261.86 ms | 250.37 ms／250.99 ms／253.45 ms | 269.36 ms／271.40 ms／279.56 ms |
| `markdown_editor_mixed/replace_selection/5` | 1.9373 ms／1.9720 ms／1.9807 ms | 1.8590 ms／1.8702 ms／1.9151 ms | — |
| `markdown_editor_mixed/replace_selection/22` | 7.7156 ms／7.7673 ms／7.9741 ms | 7.0692 ms／7.1109 ms／7.2778 ms | — |
| `markdown_editor_mixed/replace_selection/87` | 47.532 ms／47.973 ms／49.738 ms | 56.717 ms／57.024 ms／58.251 ms | — |
| `markdown_editor_mixed/replace_selection/218` | 258.15 ms／259.04 ms／262.63 ms | 286.39 ms／287.72 ms／293.06 ms | — |

表头 Enter 两次运行的耗时中心值变化范围为 -8.9% 至 +4.1%。

混合文档变宽两次运行的耗时中心值变化范围为 -24.2% 至 +1.7%。

长 CJK 段落变宽两次运行的耗时中心值变化范围为 -7.5% 至 +5.7%。

表头 Enter 的 22、87、218 KiB 在仅复用候选后的独立运行中中心值较基线约低 6.1%、14.9%、3.9%，5 KiB 约高 1.5%。两轮最终快速复测的波动包含持平或回升。能确认的是每次匹配提交少一次完整解析，同源变宽少一次树构建。短测不足以得出稳定的端到端吞吐量结论。粗排及精排对应新视口与光标，不同几何继续完整计算。

## 阶段 6 · 命名、命令和 UI 去重

### 6.1 标签模型与 Markdown 命名

- [x] 在 `crates/appkit-core/src/workspace/model.rs` 将 `WorkspaceModel<T>` 和条目命名对齐为 `TabSet<T>`、`TabEntry<T>`；在 `crates/appkit-shell/src/workspace.rs` 将宿主 `Workspace` 更名为 `EditorTabSet`。按 Core、Shell、产品消费端分批迁移，名称变化限定在开放标签模型。`notora-core::Workspace` 沿用笔记工作区语义，保留标签 ID、顺序、固定、预览替换、导航历史、持久化字段和日志定位。
- [x] 将 `crates/markdown/src/builder.rs` 中的 `BlockSource` 枚举更名为 `BlockSourceSpan`，保留连续范围、离散范围及完整 `block_range` 的语义，布局 trait 沿用原名。将 `view/mod.rs` 中的 `PreviewEngine` 更名为 `BlockViewEngine`，覆盖 Markdown 与 TXT 阅读两种生产调用。
- [x] 将 `crates/core/src/types/indices.rs` 的 `UniCharOffset` 迁为 `GraphemeOffset`，校正文档级偏移与行内偏移的语境。分批更新 `UnicharLineLookup` 和边界转换调用，保留文档级计数排除换行、多条空行可共享偏移的行为。该切片只更名，字素算法变化已经在阶段 1 单独验收。

### 6.2 编辑器渲染与命令归属

- [x] 把 `crates/markdown/src/view/preedit.rs` 的普通编辑渲染入口、空标题提示和光标绘制移到 `view/editor_view.rs` 或同目录的编辑渲染模块。预编辑继续拥有临时文档、真实与临时坐标转换及组合态装饰。回归覆盖无组合态绘制、选区替换、取消、相同文本提交、真实光标与候选窗位置。
- [x] 将 `AugmentKind`、`EditAugmentation` 迁入 `crates/markdown/src/augmenter/mod.rs`，从 Markdown 的公开增广入口导出，更新基准与集成测试。`ui` 删除旧定义和已失效的 `PluginQuery::AugmentEdit` 文档引用。Markdown 继续把内部增广结果转换为共享事务。
- [x] 将 `crates/appkit-core/src/edit_command.rs` 中的对话框、窗口、标签、侧栏、目录、历史、章节、查找和 Escape 动作搬入 `crates/app/src/menu_handler.rs` 所在的产品命令层。迁移 `input_mapper.rs`、`events.rs`、`app_lifecycle.rs` 与 `dispatch/editor.rs` 的消费端，再从 Core 清理旧变体。保持只读白名单、菜单查找与快捷键查找的区别、焦点优先级和历史导航切断撤销合并的时机。底层文件保存与标签运行时功能仍通过已有接口复用。

### 6.3 搜索栏、侧栏与主题

- [x] 在 `crates/ui/src/widgets/search_bar.rs` 与 `crates/ui/src/document_search_bar.rs` 收敛为一个文档查找组件，先共享输入、匹配计数、上一项、下一项和关闭控件，再接可选替换与正则功能。两产品保留各自的焦点、事件消费和动作映射；查找算法沿用现有共享路径。依次迁移 Textora `ui_shell.rs` 和 Notora `document_search_runtime.rs` 消费端，移除重复实现。回归涵盖空查询、零结果、Enter/Shift+Enter、Escape、按键与辅助功能焦点、隐藏时清理输入捕获、IME 预编辑绝不写入文档。
- [x] 在 `crates/ui/src/widgets/sidebar/persistent.rs` 和 `state.rs` 先共用悬停状态机，再将跨帧交互收敛为一份状态，由 `crates/app/src/ui_shell.rs` 的 Dock 生命周期维护。组件内保持布局矩形和即时控件悬停，重建时保留显隐、计时、滚动位置，避免恢复过期按钮命中。验收固定宽度、悬停时编辑器偏移、进出动画、鼠标折返和重建恢复。
- [x] 在 `crates/ui/src/theme/settings.rs` 让通用文本、按钮与控件读取 `ApplicationTheme` 的 19 个共享角色。设置页的表面和分隔角色保留少量派生访问。分批更新 `widgets/button.rs`、`widgets/switch.rs`、`button/style.rs` 及产品映射，保持浅色和深色的令牌值与主题文件格式。测试焦点环、强调文字、反相文字和警示文本的映射。

**验收**

```bash
cargo test -p textora-appkit-core --lib
cargo test -p textora-markdown --lib
cargo test -p textora-ui --lib
cargo test -p textora-app --lib
cargo test -p notora-app --lib document_search -- --test-threads=1
cargo check --workspace --all-targets
cargo test -p textora-markdown --lib --no-default-features
cargo check -p textora-app --no-default-features --features markdown
./scripts/verify.sh
```

核对 `persisted_workspace_golden_toml_roundtrip_preserves_schema`、`line_at_unichar_consecutive_empty_lines`（可随术语更新测试名）、`replacement_preedit_commit_source_update_does_not_duplicate_composition`、`search_ime_input_never_edits_document_and_escape_restores_editor`、`sync_sidebar_persistent_updates_persistent_from_widget`（随状态迁移更新）及 `settings_inherit_readability_roles_without_reusing_accent_or_inverse_text`。在两个产品的原生窗口各走一遍查找、目录、主题面板和真实中文 IME 组合及提交，记录实测结果。最终确认旧名字、重复状态与旧协议的临时转发入口已删除，持久化兼容测试保持通过。用户可见的行为变化应有阶段 1 的对应测试。

### 自动化验收记录

- 工作区 `./scripts/verify.sh` 通过边界、格式、Clippy、单测、集成测试及 doctest。库测试包括 Core 152 项、Shell 492 项、UI 1171 项、Markdown 1353 项、思维导图 202 项、Textora 913 项及 Notora 474 项。Shell 有 1 个原有忽略用例。
- `cargo check --workspace --all-targets` 和 Textora 仅 Markdown 功能编译通过。Markdown 精简配置 1233 项通过。持久化字段往返、连续空行、相同文本的 IME 提交、查找 IME、侧栏重建及两套主题的视觉角色映射均在测试中通过。
- Dock 重建搬移唯一的侧栏交互状态。重建时按新布局恢复已打开的菜单，并清除旧按钮或列表的悬停命中。新增用例复现过菜单丢失，现可继续点击并用 Escape 关闭。
- Notora 搜索控件切标签时按标签身份清理临时输入。两个用例先复现再修正，验证旧 IME 收尾既不会流入新查找框，也不会进入新文档正文。平台提交事件缺少会话标识，因此旧组合取消后，若只收到一条无新预编辑的孤立提交，会按旧组合收尾丢弃。
- Rust 产品代码及测试已清除旧协议、旧类型名及重复查找控件。

### macOS 原生窗口验收

用临时配置和测试文档启动两款应用，真实拼音候选由用户操作，提交后的查找框、匹配计数及正文由窗口画面核对。

| 场景 | Textora | Notora |
| --- | --- | --- |
| 查找 | 两条匹配，Enter 移到第二条，Shift+Enter 回到第一条，清空及关闭有效 | 两条匹配，Enter 移到第二条，Shift+Enter 回到第一条，Escape 隐藏查找栏 |
| 文档目录 | 打开后列出三个标题，点击「第三节」跳到对应位置 | 已检查外部 Markdown 和工作区 Markdown。当前产品无文档目录入口，无法执行目录跳转 |
| 思维导图主题 | 「素纸」切到「晨曦」，画布和选中预设同步更新，一次撤销恢复原色 | 在工作区笔记将「素纸」切到「晨曦」，画布同步更新，自动保存写入 `theme = "dawn"`，一次撤销后样本恢复到原始字节 |
| 真实简体拼音 | 用户看到候选并提交「中文」。查找框显示该词和 1/2 匹配计数 | 用户看到候选并提交「中文」。查找框显示该词和 1/2 匹配计数 |
