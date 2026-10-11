# Notora 文内搜索实施计划

目标：补齐当前文档查找、高亮、结果导航和输入焦点闭环。

架构：应用层复用每文档 SearchState，向纯 UI 查找栏传入查询与计数。
FrameRuntime 组合搜索栏，正文视口统一扣除搜索栏高度。
技术栈：Rust、现有 SIMD 搜索、TextBox、Button、appkit 编辑器。

全局约束：保留未提交改动；UI 不依赖应用状态；普通文本查找；不加入替换；
中文输入法预编辑不修改正文；只读文档允许搜索；不提交用户既有改动。

## 子任务一：搜索控制（最多三个文件）

- [x] 在 `crates/notora-app/src/document_search.rs` 添加搜索控制和单元测试，
  在 `lib.rs` 注册模块。接口使用 `EditorRuntime`，提供 open、close、refresh、navigate。
- [x] 测试中文计数、循环导航、空查询、编辑后重算、每文档独立查询与只读查找。
- [x] 先运行失败测试，然后复用 SIMD 匹配与 source byte range 实现。
- [x] 运行 `cargo test -p notora-app --lib document_search -- --test-threads=1`。

## 子任务二：纯 UI 查找栏（最多三个文件）

- [x] 新建 `crates/ui/src/document_search_bar.rs`，在 `lib.rs` 注册。
  输入为纯数据：query、match_count、current_match、visible、focused。
  输出 QueryChanged、Next、Prev、Close、FocusRequested；封装 TextBox 和导航按钮。
- [x] 测试 Enter/Shift+Enter 导航、Esc 关闭、IME 提交、无焦点不接收文本、窄屏布局。
- [x] 先运行失败测试，再实现布局和事件转译；暴露 IME 坐标、光标闪烁和查询全选方法。
- [x] 运行 `cargo test -p textora-ui document_search_bar`。

## 子任务三：布局和焦点协议（最多三个文件）

- [x] `state.rs` 增加互斥焦点 DocumentSearch，`action.rs` 增加 DocumentSearchRequested。
- [x] `runtime/frame_runtime.rs` 持有搜索栏，向 UI 映射快照并处理布局、绘制、IME 与闪烁。
- [x] 搜索栏焦点变化即时同步，不等待下一帧才允许输入。

## 子任务四：产品路由（最多三个文件）

- [x] `runtime/document_search_runtime.rs` 统一搜索动作和产品输入消费。
- [x] `runtime.rs` 注册模块、接通请求动作和输入、同步搜索状态并调整正文视口。
- [x] `events.rs` 区分 Cmd/Ctrl+F 和 Cmd/Ctrl+Shift+F，阻止输入穿透到正文。

## 子任务五：工具栏与端到端回归（最多三个文件）

- [x] `render.rs` 增加文内查找入口，`runtime/frame_runtime.rs` 组装 toolbar action。
- [x] `runtime/document_search_tests.rs` 验证打开、IME 输入、关闭、读写权限、
  文档切换、键盘物理键映射、正文布局与库搜索独立性。

## 收尾验证

- [x] 代码审查：逐项核对已确认规格，审查输入消费、源偏移高亮和视口一致性。
- [x] `cargo fmt --all -- --check`。
- [x] `cargo build -p notora-app`。
- [x] 执行 `./scripts/verify.sh`，明确记录既有失败，不能误报全面通过。

## 子任务六：共享正文高亮（一个文件）

- [x] `crates/appkit-shell/src/editor_runtime/editor_painter.rs` 补齐纯文本搜索高亮，
  使用源码匹配范围与可见行的字形坐标，并在关闭搜索后清除高亮。
- [x] 使用实际深色主题添加回归测试，限制搜索覆盖色透明度，保证匹配文字可读。
- [x] 代码审查发现的弹层输入与绘制优先级问题已修复并复核。

## 验证记录（2026-10-10）

- `cargo build -p notora-app` 通过；最终源码的架构边界、格式和 Clippy 检查通过。
- Notora 全套测试通过：433 项单元测试及所有集成测试、文档测试。
- 完整验证在 Textora 设置页测试处失败：
  `app_lifecycle::tests::settings_overlay_text_box_receives_clicks_and_typed_input_end_to_end`。
  该测试仍按 `3.0` 圆角定位输入框，现有控件使用 `8.0`。
  在单独导出的 HEAD 基线中也复现同一失败，与本次搜索改动无关。
- 排除上述基线失败后，其余工作区测试、集成测试和文档测试全部通过。
  补跑命令：`cargo test --workspace --exclude notora-app -- --skip app_lifecycle::tests::settings_overlay_text_box_receives_clicks_and_typed_input_end_to_end`。
  同步模块测试需要监听本机回环地址，沙箱首次运行拒绝绑定端口；
  使用允许本机测试服务的执行方式补跑后通过。
- 新增搜索回归共 22 项：搜索控制与产品路由 13 项、纯 UI 组件 8 项、共享高亮 1 项。
- 本次未进行真实窗口和中文候选窗口的人工交互检查。
