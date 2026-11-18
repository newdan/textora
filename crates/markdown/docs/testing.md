# 测试与验证体系

`textora-markdown` 的测试分布在单元测试、独立测试子模块和 `tests/` 集成测试中，另有性能基准（Criterion）与仓库级全量门禁脚本。

## 1. 测试分层与位置

### 单元测试与模块内测试（`src/`）
几乎所有核心模块都在文件末尾内嵌 `#[cfg(test)] mod tests`，此外还有按主题拆出的独立测试文件：
- **Feature 降级测试**：`src/lib.rs` 内的 `lite_tests`（仅在关闭 `rich-markdown` 时运行）、`integration_tests`、`style_tests`、`tasklist_tests`、`boundary_tests`。
- **视图与排版专项**：`src/view_empty_paragraph_tests.rs`、`src/view_font_tests.rs`、`src/view_heading_container_tests.rs`、`src/view_paragraph_semantics_tests.rs`、`src/view_rule_spacing_tests.rs`、`src/render_spacing_tests.rs`、`src/editable_paragraph_erasure_tests.rs`、`src/layout/bug_test.rs`。
- **高频密集区**：`view_tests.rs`、`augmenter_tests.rs`、`layout/block_tests.rs`、`layout/types_tests.rs`、`render_tests.rs`、`mindmap_view_tests.rs`、`mmf/` 各子模块、`paste/` 各子模块均自带大量单元测试。

### 集成测试（`tests/`）
| 测试文件 | 覆盖范围与用途 |
|---|---|
| `tests/system_boundary_audit.rs` | WYSIWYG 端到端矩阵审计（跨 `\n`/`\r\n`、段落/加粗/分割线/代码/表格、Enter/Backspace 等多维组合），使用 `tests/support/editing_audit.rs` |
| `tests/wysiwyg_chain_regressions.rs` | 连续多步编辑与光标移动链路的回归测试 |
| `tests/wysiwyg_leading_spaces.rs` | 段首空格保留与编辑行为 |
| `tests/table_creation_projection_acceptance.rs` | 表格创建后的源码投影与布局验收 |
| `tests/block_spacing.rs` | 块间距、标题缩放、分割线等几何位置断言（验证 `DrawList` / `LaidOutDoc` 像素坐标） |
| `tests/embedded_rendering.rs` | 公式（RaTeX）与 Mermaid 图表光栅化、滚动裁剪、编辑器展开等 |
| `tests/ascii_diagram_public_api.rs` | ASCII 图表识别公开行为 |
| `tests/laid_out_block_api.rs` | 布局输出公开类型 API 检查 |

## 2. 编写新测试的惯例

- **复现 Bug 先写测试**：
  - 涉及单函数/单模块逻辑（如 `augmenter` 分支、`paste` 解析、`mmf` 布局），放在对应 `src/` 文件的 `#[cfg(test)]` 中。
  - 涉及跨解析→空段注入→布局→光标/选区的完整编辑器链路，模仿 `tests/wysiwyg_chain_regressions.rs` 或接入 `tests/support/editing_audit.rs` 的 `EditingSession`。
  - 涉及渲染像素几何，模仿 `tests/block_spacing.rs` 通过 `MarkdownStyle::from_theme(&ui::theme::test_theme(), ...)` 或 `crate::test_utils::default_style()` 构造确定性样式并断言矩形坐标。

## 3. 性能基准与手工验证

- **Criterion 基准测试**（`benches/`）：
  - `benches/editing_perf.rs`：编辑与重排性能基准
  - `benches/paste_conversion.rs`：HTML/RTF 智能粘贴转换基准
- **手工测试协议**：仓库级 `docs/manual_test_protocol.md` 与 `docs/editor_performance_playbook.md` 包含端到端交互与性能排查指引。

## 4. 常用验证命令

```bash
# 运行本 crate 全部测试（默认开启 rich-markdown）
cargo test -p textora-markdown

# 运行指定集成测试或单个测试函数
cargo test -p textora-markdown --test system_boundary_audit
cargo test -p textora-markdown <test_name_substring>

# 验证关闭 rich-markdown 时的降级编译与 lite_tests
cargo test -p textora-markdown --no-default-features

# 运行性能基准
cargo bench -p textora-markdown

# 提交前仓库级全量验证（架构边界检查 + fmt + clippy -D warnings + 全工作区测试）
./scripts/verify.sh
```
