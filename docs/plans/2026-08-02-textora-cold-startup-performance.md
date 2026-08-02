# textora 冷启动性能优化记录

日期：2026-08-02

## 目标

缩短 macOS 发布版 textora 从进程启动到窗口初始化完成的关键路径，同时保证测试构造器、字体加载失败回退和 GPU 兼容回退不发生行为退化。

## 启动链路审计

优化前的生产入口先创建 `EventLoop`，再同步执行 `App::new`。`App::new` 会在主线程加载字体缓存；窗口恢复阶段再同步请求 GPU adapter/device、创建 Surface 和文本渲染资源。因此字体、EventLoop 和 GPU 准备串行占据启动关键路径。

审计还发现两个会干扰首帧测量的窗口问题：

1. 窗口在首帧 Surface 获取前被设置为完全透明，仅在成功 present 后恢复不透明。这形成“透明窗口被判定为 Occluded，成功 present 后才恢复”的循环依赖。
2. 持久化坐标未校验当前显示器拓扑。拔掉外接屏后，窗口可能恢复到所有显示器之外。

对应修复均先添加失败测试，再移除透明启动路径，并要求持久化窗口至少有 `64 × 64` 物理像素落在任一当前显示器内。

## 实现

生产入口使用 `App::new_for_launch`，在 EventLoop 创建前启动两项互不依赖的准备工作：

- 字体线程加载磁盘字体缓存；窗口初始化时只汇合线程结果并建立共享 `FontSystem`。线程创建或执行失败时同步重试。
- `EditorRuntime` 提前请求 surface-independent GPU adapter/device；窗口创建后附加 Surface。不支持目标 Surface 时回退到原有 surface-aware 同步路径。

普通 `App::new` 仍同步提供共享字体系统，避免数百个无窗口单元测试改变语义或创建无意义的后台线程。

## 测量方法

- 构建：`cargo build --release -p textora-app`
- 启动：将发布版二进制装入仓库 `target/textora.app`，通过 macOS LaunchServices 启动
- 样本：优化前、优化后各 10 个独立进程
- 终点：日志出现 `init_window total` 后终止测试实例
- 环境：字体磁盘缓存命中，操作系统页缓存保持自然状态，不执行破坏性系统 cache purge

本机测量期间处于锁屏状态，Metal 正确返回 `CurrentSurfaceTexture::Occluded`，因此本轮不把 `first_frame_visible` 纳入数值对比。窗口初始化前的局部计时在锁屏下仍可重复；解锁后的真实可见首帧需作为发布前人工复测项。

## A/B 结果

以下均为 10 次发布版独立进程的中位数：

| 阶段 | 优化前 | 优化后 | 变化 |
|---|---:|---:|---:|
| `App::new total` | 6.50 ms | 0.98 ms | -84.9% |
| `window create + theme` | 28.83 ms | 23.08 ms | -19.9% |
| `init_window total` | 36.26 ms | 29.70 ms | -18.1% |
| 字体汇合等待 | 不适用 | 约 0.006 ms | 字体工作完全被隐藏 |

优化后字体线程自身中位耗时约 9.45 ms；由于它与 EventLoop/GPU 准备并行，窗口初始化时只等待约 6 微秒。字体线程与 GPU 线程并行会增加各自的瞬时 CPU 竞争，但显著缩短主线程关键路径。

## 回归保护

- 生产构造器必须延迟共享字体系统，并持有字体准备任务。
- 普通测试构造器必须立即提供共享字体系统。
- 生产构造器必须出现在 EventLoop 构造之前。
- 首帧 Surface 获取前不得把窗口设为完全透明。
- 只恢复在当前显示器拓扑中具有可见区域的持久化窗口坐标。

验证命令：

```text
cargo test -p textora-app
cargo build --release -p textora-app
./scripts/verify.sh
```
