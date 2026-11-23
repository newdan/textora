# AppKit 复用边界收敛

## 第一阶段 · Textora 输入与导入

- 将 winit 按键到编辑命令的映射、Textora 的鼠标状态和源码指针分发移入 `crates/app`。编辑命令类型由 `appkit-core` 提供，共享的指针捕获和画布拖拽类型留在 `appkit-shell`。
- 删除 Textora crate 根的 20 条迁移期内部重导出。调用方直接导入实现 crate，本地的视口尺寸类型从标签页工厂导入。

## 第二阶段 · 跨产品逻辑

- `appkit-shell` 共享 macOS 打开文件桥接与交通灯布局。各产品提供打开文件事件发送入口、标题栏高度以及自己的窗口行为。
- `appkit-shell` 共享源码间距状态刷新。产品提供 Markdown 识别所需的补充信息和解析回退。`appkit-core` 统一文档匹配扫描，各产品保留自己的搜索交互。
- 在中立文件监控库复用 `notify` 后台线程与监控生命周期。Textora 保留动态监控根与 200 ms 尾沿去抖，Notora 保留固定 200 ms 批次和事件归一化。Notora 内核无需依赖 AppKit。

## 第三阶段 · 标签页会话与滚动

- 移除只读与可变会话的隐式解引用。通过 `document()`、`document_mut()` 和 `plugin()` 显式区分文档与视图，插件选区查询使用明确命名。
- 就地拆借 `DisplayState` 的 `viewport` 和 `display_map`。按原顺序执行滚动、锚点校正与滚动位置推导。以编译约束阻止整张映射再次被克隆。
- 检查跨层依赖与直接导入，运行 `./scripts/verify.sh`，覆盖格式、Clippy、Notora 测试及工作区测试。
