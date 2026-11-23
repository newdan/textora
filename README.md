# Textora

[English](README.en.md)

本仓库包含两个使用 Rust 和 [wgpu](https://github.com/gfx-rs/wgpu) 构建的原生桌面应用：**Textora** 是纯文本与 Markdown 编辑器，**Notora** 是带本地笔记库的笔记应用。两者共享编辑器内核与桌面宿主。

> **项目状态：** 项目正在积极开发中。
>
> **平台支持：** 两个桌面应用均以 **macOS 12 及更高版本**为运行和打包目标。

新接手项目？从[新人接手指南](docs/onboarding.md)开始，照着一次点击、一次编辑和一次保存走读代码。

## Textora 功能特性

- 支持标签页、侧边栏导航、自动换行、行号与状态信息的纯文本编辑。
- 提供源码、所见即所得和预览视图的 Markdown 编辑。
- 根据 Markdown 文件、`.mmap.md` 思维导图和 `.txt` 阅读模式自动路由视图。
- 支持 Unicode 文本处理的查找与替换，以及文本缓冲区中的正则表达式支持。
- 支持语法高亮、明暗主题、可配置的排版和用户主题加载。
- 支持工作区持久化、最近文件、未保存文档恢复与外部文件变更监测。
- 通过 `wgpu` 和 `cosmic-text` 提供 GPU 加速渲染与文本塑形。

## 快速开始

### 前置条件

- macOS 12 或更高版本
- 由 [`rust-toolchain.toml`](rust-toolchain.toml) 指定的 Rust 工具链，当前为 **1.93.0**

### 运行

从仓库根目录运行。Cargo 默认成员是 `textora-app`；启动 Notora 时需要显式选择 `notora-app`。

```bash
# Textora
cargo run -p textora-app --bin textora
cargo run -p textora-app --bin textora -- path/to/file.md

# Notora（传入文件路径时，按外部文件打开）
cargo run -p notora-app --bin notora
cargo run -p notora-app --bin notora -- path/to/file.md
```

启动代码分别位于 [`crates/app/src/main.rs`](crates/app/src/main.rs) 和 [`crates/notora-app/src/main.rs`](crates/notora-app/src/main.rs)。

仅初始化 Textora 的 GPU 后端，不创建窗口：

```bash
cargo run -p textora-app -- --headless
```

### 构建

以调试模式构建两个应用：

```bash
cargo build -p textora-app
cargo build -p notora-app
```

构建精简版可执行文件（保留基础 Markdown 和思维导图，不编译 LaTeX、Mermaid 图表与 HTML 富文本粘贴支持）：

```bash
cargo lite
```

产物位于 `target/lite/textora`。精简版将数学公式和 HTML 标签显示为原文，`mermaid` 代码块仍按普通代码块显示。

构建 macOS 发布包：

```bash
./scripts/bundle-mac.sh
./scripts/bundle-notora-mac.sh
```

默认的应用包路径为 `target/textora.app` 和 `target/notora.app`。

### 验证

运行仓库的常规验证入口：

```bash
./scripts/verify.sh
```

[`scripts/verify.sh`](scripts/verify.sh) 的覆盖范围：

| 检查 | 覆盖范围 |
| --- | --- |
| 架构边界 | 运行 [`scripts/check_architecture.sh`](scripts/check_architecture.sh)；`TEXTORA_ARCHITECTURE_MIGRATION` 非空时跳过此项 |
| 格式与 Clippy | 全工作区格式检查；`cargo clippy --workspace --all-targets -- -D warnings` |
| 默认测试 | `notora-app` 单独以单线程运行；其余工作区包一起测试，但排除 `textora-sync` |

`textora-sync` 仍在 Clippy 检查范围内，测试需单独运行 `cargo test -p textora-sync`（部分测试使用本地回环端口）。默认测试不执行标记为 `#[ignore]` 的用例，也不会实际操作桌面界面。专项编辑验证见 [`scripts/verify-editing.sh`](scripts/verify-editing.sh)。

## Textora 文件视图

| 文件模式 | 默认视图 |
| --- | --- |
| `.md`、`.markdown` | Markdown 编辑器 |
| `.mmap.md` | 思维导图 |
| `.txt` | 纯文本编辑器，可切换至阅读视图 |
| 其他文件 | 纯文本编辑器 |

Markdown 文档可在编辑和预览视图间切换。思维导图使用后缀为 `.mmap.md` 的 Markdown 文件。

## 工作区结构

| 目录 | Cargo 包名 | 职责 |
| --- | --- | --- |
| [`crates/app`](crates/app) | `textora-app` | Textora 产品层：窗口、命令路由、文件工作区与同步集成 |
| [`crates/notora-app`](crates/notora-app) | `notora-app` | Notora 产品层：笔记库界面、笔记操作、自动保存与编辑器适配 |
| [`crates/notora-core`](crates/notora-core) | `notora-core` | 笔记域：笔记库目录、SQLite 索引、笔记命令和文件监控 |
| [`crates/file-watch`](crates/file-watch) | `textora-file-watch` | 共享文件监听基础层，基于 `notify`，由 Textora 应用层和 Notora 笔记域使用 |
| [`crates/appkit-core`](crates/appkit-core) | `textora-appkit-core` | 无界面的 `DocumentModel`、标签集合 `TabSet`、会话持久化与文件安全 |
| [`crates/appkit-shell`](crates/appkit-shell) | `textora-appkit-shell` | 共享桌面宿主：`EditorRuntime`、标签运行态、输入和绘制管线、保存任务 |
| [`crates/core`](crates/core) | `textora-core` | 文本缓冲区、撤销历史、Unicode 导航、搜索、高亮和文件读写原语 |
| [`crates/ui`](crates/ui) | `textora-ui` | 可复用控件、主题和视口几何；定义编辑器视图的插件协议 |
| [`crates/markdown`](crates/markdown) | `textora-markdown` | Markdown 解析、源码与视觉投影、编辑策略、预览与阅读 |
| [`crates/mindmap`](crates/mindmap) | `textora-mindmap` | MMF 解析与编辑、思维导图布局和交互视图 |
| [`crates/render`](crates/render) | `textora-render` | GPU 绘制原语与字形图集 |
| [`crates/shaping`](crates/shaping) | `textora-shaping` | 字体选择、文本塑形、字形光栅化与缓存 |
| [`crates/sync`](crates/sync) | `textora-sync` | Textora 的 Syncthing 客户端、同步命令与状态 |
| [`crates/encryption`](crates/encryption) | `textora-encryption` | 加密 Markdown 格式、加解密和解锁会话 |
| [`crates/lsh`](crates/lsh) | `lsh` | 语法高亮语言定义、编译器和运行时 |
| [`crates/stdext`](crates/stdext) | `stdext` | 内存分配、容器、Unicode 和系统原语等底层工具 |

UI 层接收来自应用层的纯数据输入，不依赖应用状态类型。详细架构规则见 [`AGENTS.md`](AGENTS.md)，开发约定见 [`CONTRIBUTING.md`](CONTRIBUTING.md)。

## 项目文档

- [新人接手指南](docs/onboarding.md)：编辑提交、保存完成、点击映射的代码入口和验证用例。
- [计划与既有文档索引](docs/README.md)：查阅历史整改和设计背景。

## 许可证

Textora 依据 [MIT License](LICENSE) 发布。
