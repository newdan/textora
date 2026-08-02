# Textora

[English](README.en.md)

Textora 是一款使用 Rust 和 [wgpu](https://github.com/gfx-rs/wgpu) 构建的原生纯文本与 Markdown 桌面编辑器。

> **项目状态：** Textora 正在积极开发中。
>
> **平台支持：** 目前应用面向 **macOS 12 及更高版本**。可复用的工作区 crate 保持可移植性设计，但桌面应用尚未支持跨平台。

## 功能特性

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

不指定初始文件启动 Textora：

```bash
cargo run -p textora-app
```

启动时打开文件：

```bash
cargo run -p textora-app -- path/to/file.md
```

初始化 GPU 后端而不创建窗口：

```bash
cargo run -p textora-app -- --headless
```

### 构建

以调试模式构建应用：

```bash
cargo build -p textora-app
```

构建 macOS 发布版 `.app` 包：

```bash
./scripts/bundle-mac.sh
```

生成的应用包位于 `target/textora.app`。

### 验证

运行完整项目验证套件：

```bash
./scripts/verify.sh
```

该命令会检查架构边界、代码格式、Clippy 和整个工作区的测试套件。

## 文件视图

| 文件模式 | 默认视图 |
| --- | --- |
| `.md`、`.markdown` | Markdown 编辑器 |
| `.mmap.md` | 思维导图 |
| `.txt` | 纯文本编辑器，可切换至阅读视图 |
| 其他文件 | 纯文本编辑器 |

Markdown 文档可在编辑和预览视图间切换。思维导图使用后缀为 `.mmap.md` 的 Markdown 文件。

## 工作区结构

| Crate | 职责 |
| --- | --- |
| [`crates/app`](crates/app) | 桌面应用、产品行为和平台集成 |
| [`crates/ui`](crates/ui) | 可复用 UI 组件、布局、主题和小部件 |
| [`crates/core`](crates/core) | 文本缓冲区、文档模型、编辑和搜索基础能力 |
| [`crates/markdown`](crates/markdown) | Markdown 编辑、预览、阅读和思维导图视图 |
| [`crates/render`](crates/render) | GPU 渲染原语 |
| [`crates/shaping`](crates/shaping) | 文本塑形与字形布局 |
| [`crates/appkit-core`](crates/appkit-core) / [`crates/appkit-shell`](crates/appkit-shell) | 共享的应用模型和桌面壳层基础设施 |

UI 层接收来自应用层的纯数据输入，不依赖应用状态类型。详细架构规则见 [`AGENTS.md`](AGENTS.md)，开发约定见 [`CONTRIBUTING.md`](CONTRIBUTING.md)。

## 项目文档

计划和项目文档索引见 [`docs/README.md`](docs/README.md)。

## 许可证

Textora 依据 [MIT License](LICENSE) 发布。
