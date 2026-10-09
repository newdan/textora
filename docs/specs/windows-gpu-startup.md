# Windows GPU 启动与回退

## 默认行为

Windows 依次尝试 OpenGL 和 DX12，每次仅创建当前后端的 wgpu Instance。
OpenGL 成功时不会初始化 DX12。共享 appkit-shell 的 Windows 构建启用
`gles`、`dx12`，工作区启用 `wgpu/std`，保证环境变量读取与错误作用域有效。
macOS、Linux 保留原来的默认后端集合。

后台准备 adapter/device 后保留尚未尝试的后端。窗口创建后，表面附加、
交换链配置、MSAA 资源和文本渲染管线属于同一次后端尝试。
任何可恢复的初始化错误都会释放本次资源，继续尝试下一个后端。
Validation、Internal、OutOfMemory 错误通过 wgpu error scope 转换为 Result；
不通过 catch_unwind 捕获 panic，发布版仍使用 panic=abort。
驱动进程崩溃、后续绘制期间设备丢失不属于本次启动回退范围。

无窗口准备的 adapter 与窗口不兼容时，先保留既有的同后端 surface-aware
重选机制，失败后再继续后续后端。所有后端无 adapter 时仍返回 NoAdapter，
其他失败保留各后端原因。`--headless` 使用相同的后端选择策略。

## 显式覆盖

`WGPU_BACKEND=gl`、`dx12` 或 `vulkan` 严格限制候选后端，不自动添加其他后端。
逗号分隔值沿用 wgpu 的后端集合语义，不表示按字符串顺序尝试。
未设置该变量时才启用 OpenGL → DX12 默认回退。
`WGPU_POWER_PREF` 继续支持 `low`、`high`、`none`，默认 high。

## Windows 10 兼容边界

沿用现有 wgpu 版本及 Windows WGL / 系统 D3D12 路径，不引入 Win11 专用 API，
不要求 Agility SDK。Windows 10 是兼容目标，发布前须在 Windows 10 实机或 VM
验证；本机成功不等同于已完成 Windows 10 认证。

驱动仍需满足所选后端要求：OpenGL 3.3+ 且支持文本渲染使用的双源混合，
或者满足当前 wgpu DX12 后端的功能要求。当前依赖的 DX12 实现要求 Shader
Model 6 和 Resource Binding Tier 2；操作系统版本本身不能保证显卡支持。

参考：[wgpu 平台支持](https://github.com/zed-industries/wgpu/tree/357a0c56e0070480ad9daea5d2eaa83150b79e88)、
[微软 D3D12 文档](https://learn.microsoft.com/en-us/windows/win32/direct3d12/)。

## 验证

- 单元测试覆盖默认顺序、成功时不触发 DX12、不同初始化阶段的失败回退、
  全部失败的错误、显式覆盖和独立进程环境变量读取。
- Windows 显式 GPU 测试验证真实着色器验证错误能够回退 DX12，
  以及已经绑定 OpenGL 的同一 HWND 可以重新配置 DX12 并建立文本管线。
- 发布版检查默认 OpenGL 和显式 DX12 的首帧。
- 全面检查运行 `./scripts/verify.sh`。

```text
cargo test -p textora-appkit-shell --lib gpu::
cargo test -p textora-appkit-shell --lib gpu::tests::native_shader_failure_falls_back_to_dx12 -- --ignored --exact --nocapture
cargo test -p textora-appkit-shell --lib gpu::windows_tests::attached_opengl_failure_recreates_dx12_resources_on_the_same_window -- --ignored --exact --nocapture
cargo build --release -p textora-app
```

### 2026-10-09 本机验证记录

- 普通发布版构建通过；不设置后端变量时选中 OpenGL，显式 DX12 也成功提交首帧。
- 9 项 GPU 回归测试通过；两项需真实驱动的 ignored 测试已分别显式运行并通过。
- appkit-shell 库的 Clippy `-D warnings` 通过。
- `verify.sh` 未全绿：工作树 CRLF 与 rustfmt Unix 要求冲突；临时规范化后，
  架构和格式检查通过，随后被 core/path.rs 中 Windows 下未使用的 norm 测试函数阻断。
- 剩余工作区测试已单独运行，仍有换行、路径、编辑行为等失败，不能视为完整回归通过。
  Windows 10 实机验证尚未执行。
