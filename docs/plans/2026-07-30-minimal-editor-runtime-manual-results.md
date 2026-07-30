# 最小 EditorRuntime 手工回归记录

日期：2026-07-30

平台：macOS（当前桌面会话锁屏，未取得 DPI 与窗口尺寸）

## 自动化验收

以下项目已通过：

- `bash scripts/check_architecture.sh`
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`：全部测试通过，2 个既有 ignored 测试
- `cargo test -p textora-appkit-shell --test editor_runtime_fake_product`
- `cargo test -p textora-app --test render_smoke`
- `cargo test -p textora-app --tests`
- `./scripts/verify.sh`

覆盖内容包括：假产品非零偏移 editor rect、焦点门、编辑、reshape、异步保存、外部修改竞态、关闭后迟到结果、workspace/runtime ID 双射、持久化格式和 smoke 启动/resize/shutdown。

## GUI 手工验收

未执行。Computer Use 检查到 macOS 处于锁屏状态，自动解锁失败；没有绕过锁屏或模拟用户凭据。待桌面解锁后，按 [`docs/manual_test_protocol.md`](../manual_test_protocol.md) 执行窗口启动、resize/DPI、输入/IME、Tab、Markdown/Mindmap、Save/Save As、外部修改冲突和关闭提示回归。
