# 核心原则 (Core Principles)
- **请全程使用中文回复**。
- **沟通先行**：行动前说清方案，需求不明先问清楚。
- **根因分析**：遇 Bug 先写复现测试再修；同一 Bug 修改超两次须推翻重审，拒绝叠加防御性补丁。
- **任务拆解**：单任务修改超 3 个文件必须拆分为子任务。每次提交前必须确保编译通过。
- **重大修改，请全面验证** ： ./scripts/verify.sh。

## 代码洁癖 (Clean Code)
1. **命名严谨**：必须“精准自解释”，杜绝拼音或中英混杂，禁用 `data/info/temp/res/flag` 等宽泛词。
2. **消灭魔法值**：严禁硬编码无意义的数字/字符串，须提取为语义化的常量或枚举。
3. **职责单一**：函数只做一件事，超过 50 行必须考虑拆分。
4. **提前返回 (Early Return)**：优先判断异常并提前 `return`，消除深层 `if-else` 嵌套。
5. **清理废弃代码**：提交前必删死代码 (Dead Code)、多余注释及未使用的引入 (Unused Imports)。
6. **作用域最小化**：变量在首次使用处就近声明，严格控制生命周期。
7. **类型驱动状态 (Rust)**：优先用 `enum` 表示互斥状态，严禁组合多个 `bool` 字段。
8. **严谨处理错误 (Rust)**：严禁图省事滥用 `.unwrap()`。若确信不会 panic，必须用 `.expect("详细说明理由")`。
9. **视觉整洁**：严格遵守 `cargo fmt`。逻辑块间仅留一行空行，保持代码呼吸感。

## 文档与归档
- **临时规划**：日常开发使用 Agent 内置的 Task/Plan 面板。
- **长期沉淀**：重大的架构设计、重构计划及功能规范，必须以 Markdown 沉淀至 `docs/plans/` 或 `docs/specs/`。
- **阶段切分**：开发须解耦并控制单阶段工作量；涉及多模块协作时，优先设计接口与协议。

## 项目架构与规范
产品名是textora
markdown包名是 textora-markdown

### 依赖层次
```text
crates/ui (纯 UI 组件库)
  ├── 依赖: core, render, shaping, winit, wgpu, unicode_categories
  ├── 不依赖: DocumentView, Workspace, Commands, Events
  └── 模块: theme, settings, viewport, render_geom, layout, gutter, decorations, 
            以及 widgets 模块 (tab_bar, text_box, search_bar 等)

crates/app (应用层)
  ├── 依赖: ui, core, render, shaping, wgpu, winit
  └── 职责: 从 DocumentView 提取数据 → 构造 Widget 输入 → 调用 ui::render()
```

### UI 模块一览
| 模块 | 职责 | 输入抽象 |
|------|------|----------|
| `ui::theme` / `settings` | 主题与编辑器配置 | 纯数据 / 全局单例 `Settings::get()` |
| `ui::viewport` | 视口几何计算 | `LineMap` trait |
| `ui::render_geom` | 选区高亮、像素映射 | `AdvanceCacheEntry` |
| `ui::layout` | 换行与 CJK 边界检测 | 纯函数 |
| `ui::gutter` / `decorations` | 行号与光标/选区渲染 | `RenderContext` / 独立参数 |
| `ui::widgets::*` | 独立组件 (TextBox, TabBar 等) | 纯数据 struct (如 `TabInfo`) |

### 关键设计决策 (绝对红线)
- **跨层解耦规范**：必须在 `ui` (或 `ui::widgets`) 中定义纯数据输入 struct，由 `app` 层负责从 `DocumentView` 或其他状态模型中解构和映射。**绝对禁止让 `ui` 直接依赖或访问 `app` 层的状态结构体**。
