# textora-markdown 文档导航

`textora-markdown`（目录：`crates/markdown/`）是 Textora / Notora 的 Markdown 渲染、所见即所得（WYSIWYG）编辑、智能粘贴与思维导图引擎。

> 本目录文档面向需要修改或理解本 crate 的 AI Agent，侧重**边界、模型、不变量与验证路径**，不复述源码里的函数级细节。

## 文档索引

| 文档 | 回答的问题 | 何时阅读 |
|---|---|---|
| [architecture.md](architecture.md) | 系统长什么样？ | 首次接触本 crate、需要定位模块或理解数据流时 |
| [concepts.md](concepts.md) | 术语和坐标指什么？ | 涉及坐标换算、光标/选区、展开机制、空段落或 MMF 时 |
| [invariants.md](invariants.md) | 什么绝对不能破坏？ | 准备修改布局、编辑、投影、跨层接口之前（**必读**） |
| [guides.md](guides.md) | 某类改动怎么下手？ | 新增块类型、改按键行为、加命令、调样式或扩展粘贴/导图时 |
| [testing.md](testing.md) | 改完怎么验证？ | 编写复现测试、跑局部/全量验证或基准测试时 |
| [decisions.md](decisions.md) | 为什么这样设计？ | 质疑现有方案、需要了解历史取舍或查找上游 spec 时 |

## 快速阅读路线

- **只做渲染/样式/间距调整**：`architecture.md` → `invariants.md`（布局与间距部分）→ `guides.md` → `testing.md`
- **修 WYSIWYG 编辑/光标/IME Bug**：`concepts.md` → `invariants.md`（编辑与坐标部分）→ `testing.md`
- **改智能粘贴或思维导图（MMF）**：`architecture.md`（对应子系统节）→ `guides.md` → `testing.md`
- **排查跨 crate 边界问题**：`architecture.md`（上下游边界）→ `invariants.md`

## 上游设计规范目录

大量历史设计规范与代码审查沉淀在工作区根目录的 `docs/specs/` 与 `docs/` 下（如 WYSIWYG 空行不变量、表格编辑、公式与 Mermaid、智能粘贴、MMF 格式等），详见 [decisions.md](decisions.md) 中的索引。
