# Markdown 快捷键一致性排查与修复

目标：按既有 `2026-08-19-markdown-editor-keybindings.md`，修复 textora 与 notora 的快捷键漏接、冲突和输入所有权问题。

## 子任务与接口

1. **共享映射与执行入口**（`appkit-shell/src/window_input.rs`、`editor_runtime/mod.rs`）
   - 将物理键映射收敛为 `markdown_semantic_shortcut(PhysicalKey, ui::Modifiers)`，逻辑键入口复用同一命令表。
   - 新增 `EditorRuntime::handle_key_input_with_physical_key`，保留现有逻辑键调用兼容性。
   - 格式化先于剪贴板执行，限制为可编辑 Markdown；输入法组合期间阻止修改，但保留导航。
   - 先补缺失命令、组合输入穿透的失败测试，再接线并执行定向测试。
2. **notora 窗口输入与回归测试**（`notora-app/src/events.rs`、`runtime.rs`、`runtime/keyboard_shortcut_tests.rs`）
   - 将物理键位保留到共享运行时；产品控件继续接收原始逻辑字符。
   - 切离编辑器及 IME 生命周期切换时清理组合态，禁止 Process 键的文本回退。
   - 表驱动验证全部格式命令、中文选区、撤销/重做、输入所有权、物理键与逻辑字符不一致的情况。
3. **textora 入口与快捷键规范**（`app/src/events.rs`、既有快捷键规范）
   - 使用共享映射，删除产品内重复命令表；明确两端行为与互斥规则。
4. **语义命令边界**（`markdown/src/commands.rs`）
   - 由独立子任务先复现再修复标记切片越界、空行命令无效等确定性问题。
5. **重做与逻辑键大小写**（`appkit-shell/src/input_mapper.rs`、`app/src/app_lifecycle.rs`）
   - 由独立子任务先复现再修复 Shift 产生大写逻辑字符后的快捷键和搜索白名单遗漏。

## 验收

- 各问题具备实际失败→通过的回归测试。
- 保持 UI 与应用状态解耦，格式化复用语义事务和撤销历史。
- 审查最终 diff，运行 `./scripts/verify.sh`；本地 mock server 需要端口时在允许绑定的环境运行。
- 构建 notora 和 textora。保留当前未提交修改，不自动提交、安装或重启用户应用。
