# Windows x86、x64、ARM64 支持计划

## 目标与验证层级

textora 的 Windows 目标为 `i686-pc-windows-msvc`、`x86_64-pc-windows-msvc`、`aarch64-pc-windows-msvc`。先建立源码编译和原生 Windows 构建门禁，再在真实设备上完成编辑、保存、输入和 GPU 渲染验收。macOS 的交叉 `cargo check` 只能证明类型检查通过，不能证明链接、启动或交互正常。

## 阶段 1：构建边界

- 移除应用入口的 macOS 专有限制，保留 macOS 原生菜单与打开文档处理的条件编译。
- 为 Windows 实现系统 ICU 动态加载，使用系统目录的 `icu.dll`；该合并库要求 Windows 10 1903 或更新版本，见 [Microsoft 文档](https://learn.microsoft.com/en-us/windows/win32/intl/international-components-for-unicode--icu-)。文件保存改用标准库的跨设备错误分类。
- 字体缓存使用平台缓存目录，并监测 Windows 系统与用户字体目录。
- CI 分别在 Windows x64 和 ARM64 runner 构建三个 MSVC 目标，并运行现有 ICU 首次加载测试。Windows runner 标签见 [GitHub 官方文档](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)。

## 阶段 2：原生运行验收

在 x86、x64、ARM64 Windows 上分别验证：冷启动、窗口绘制与缩放、中文输入、剪贴板、文件对话框、打开和保存 UTF-8/CRLF 文件、Markdown 视图、大文件编辑和无 GPU 可用时的报错。此阶段需要原生 Windows 运行环境；CI 构建通过后再记录实际结果。

## 阶段 3：平台功能补齐

- 当前同步 API 密钥存储仅实现 macOS Keychain，Windows 会返回不支持；需要 Windows 凭据存储实现及安全测试。
- 当前原生菜单仅实现 macOS；需确认 Windows 键盘快捷键与窗口命令的可达性，再决定 Windows 菜单设计。
- 完成图标、安装包、签名与发布流水线。发布格式和最低 Windows 版本需在原生验收后确定。

## 完成标准

三目标 CI 构建与 ICU 测试稳定通过，三架构原生手测无阻塞问题，平台功能补齐后才能宣称 Windows 正式支持。
