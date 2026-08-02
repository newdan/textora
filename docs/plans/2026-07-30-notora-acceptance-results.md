# notora 首版验收记录

日期：2026-08-02

## 自动化验收

以下命令已于 2026-08-02 执行并以零退出状态通过：

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test -p notora-core`
- `cargo test -p notora-app`
- `cargo test -p notora-app --test recovery_flow`
- `cargo test -p notora-app --test trash_flow`
- `cargo test -p notora-app --test search_flow`
- `cargo test -p textora-app --lib`
- `cargo test -p textora-app --test smoke`
- `cargo test -p textora-app --test render_smoke`
- `bash scripts/check_architecture.sh`
- `./scripts/verify.sh`

覆盖要点：10,000 条目录搜索与虚拟卡片、分页查询、标签和 Trash 生命周期、session
恢复 DTO、runtime LRU、catalog backup/损坏恢复、工作区不可用、失败保存重试、异常退出
snapshot 恢复、watcher 断开诊断与删除延迟确认、dirty Trash 的保存失败与 revision 竞态
保护，以及 UI 与产品层的依赖边界。增量扫描另覆盖未变文件不读正文、watcher 目标路径
扫描、两阶段 missing、原子 rename 合批；标签 attach/rename/detach 与 FTS 更新保持同事务。

本轮 `./scripts/verify.sh` 已在允许本机 loopback 端口绑定的受控环境完整运行，架构、格式、
全 workspace clippy、workspace tests 和 doctests 均以零退出状态结束。notora 本轮为 141 个
单元测试，另含 open/recovery/save/search/smoke/trash 集成测试；`textora-sync` 的 27 项测试
（含 loopback mock server）全部通过。

启动恢复 smoke 断言首帧 `present` 成功后才恢复 session；外部文件路径验证、正文读取、
Save As canonicalize、冲突 reload/retry revision、catalog migration backup/recovery 以及
settings/session 写入均通过后台 channel/worker 回传，迟到结果按 selection、revision 或
workspace generation 丢弃。

`library_bench` 是可重复的测量入口，而不是性能承诺。本次在 Darwin arm64、macOS
26.3.1 上以 10 个样本采集到：

- 已建索引的 10,000 条中文搜索首页：17.094–17.185ms；
- 10,000 条工作区首页分页：83.369–83.902µs；
- 10,000 条数据中的可见卡片布局：895.35–903.55µs。

本轮新增的 tab 切换基准以 10 样本、100ms 预热和 100ms 测量快速采样，结果为
26.70–26.86ns。10,000 笔记扫描基准已编译，但单次采样超过此执行器时限，未生成有效
估算文件；需要在 CI 或开发机以完整 Criterion 参数重新采集。

每次新的发布基线仍应在目标机器上以 `cargo bench -p notora-app --bench library_bench`
重新采集，并附带硬件/系统信息。

## macOS 实机验收

当前自动化环境不能替代真实窗口、文件选择器和输入法的人工验收。发布前需要在 macOS
完成并记录：工作区创建与恢复、TXT/MD/MMAP.MD 编辑、800ms 自动保存、external Save As、
冲突提示、设置弹层、窄窗口导航/返回、Trash restore 冲突，以及重启后最后文档恢复。

本次已通过 `cargo run -p notora-app --bin notora` 启动原生窗口；运行日志确认 GPU
surface 初始化并完成首帧渲染。当前验收控制器无法枚举未打包 winit 可执行程序的窗口，
因而不能可靠执行后续点击、文件选择器和 IME 操作。

上述交互项在实机确认前保持未勾选，不以启动日志或自动化测试冒充手工结果。
