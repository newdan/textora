# 渲染能耗优化改造方案 v2

> 将 edit+ 从「持续 60fps 刷新」改造为「按需渲染」，大幅降低空闲功耗。

---

## 一、问题诊断

### 当前架构：伪持续刷新

`crates/app/src/app.rs:2324-2336` 的 `about_to_wait()` 中：

```rust
if self.needs_redraw {
    window.request_redraw();
} else if self.gpu.is_some() && self.text.is_some() {
    // 计算了 _next_transition 但从未使用
    let _next_transition = ((blink_ms / 500) + 1) * 500;
    window.request_redraw();   // ← 永远走这里
}
```

**根因**：光标闪烁需要每 500ms 切换可见/不可见，但代码没有使用 `WaitUntil` 定时器，而是每帧 (~16ms) 都 `request_redraw`，导致：

1. **CPU 永不休眠** — `ControlFlow::Wait`（在 `resumed()` 中设置）被 `about_to_wait` 的无条件 `request_redraw` 完全架空
2. **GPU 每帧 submit** — 即使 `RenderCache` 命中也无法跳过 `queue.submit()` + `present()`
3. **macOS App Nap 被阻止** — 系统无法将进程降频到能效核心
4. **空闲功耗 ≈ 活跃功耗** — 笔记本上额外消耗 ~3-6W，电池续航减少 15-30%

### 量化能耗分析

| 资源 | 空闲时的实际行为 | 估算功耗开销 |
|------|-----------------|-------------|
| CPU | 每帧 `render()` 完整路径：shaping 查询 + vertex 生成 + uniform 计算 | ~1-3W |
| GPU | 每帧执行 render pass + `queue.submit()` + `surface.present()` | ~1-2W |
| 系统调度 | App Nap 被阻止，无法降到效率核心 | ~0.5-1W |
| 合计 | | **~3-6W 额外功耗** |

---

## 二、目标

| 指标 | 改造前 | 改造后 |
|------|--------|--------|
| 空闲帧率 | ~60fps | 0fps（完全不调 request_redraw） |
| 光标闪烁帧率 | ~60fps | ~2fps（每 500ms 渲染一帧） |
| 空闲 CPU 唤醒频率 | ~60 次/秒 | <5 次/秒 |
| 空闲 CPU 占用 | ~5-15% | ~0% |
| macOS App Nap | 被阻止 | 正常进入 |
| 输入响应延迟 | 不变 | 不变 |
| 渲染正确性 | 不变 | 不变 |

---

## 三、核心设计决策

### 3.1 简化 RenderDemand：只用 None / FullRender 二级

原方案设计了三级 `RenderDemand`（None / PresentOnly / FullRender），`PresentOnly` 用于光标闪烁时跳过 shaping 和 vertex 重建、仅做 GPU present。

**取消 PresentOnly 的理由**：

1. wgpu 的 `SurfaceTexture` 每次 `get_current_texture()` 返回内容未定义的新 texture，无法"仅 present"
2. 即使保留 vertex buffer 重录 render pass，也需要完整 GPU 管线（节省的只是 CPU 侧 shaping）
3. 光标闪烁仅 2fps，每次 FullRender 开销 ~7ms CPU/秒（占总 CPU < 0.1%），优化收益可忽略
4. PresentOnly 引入 vertex buffer 生命周期管理的额外复杂度

**简化后的模型**：

```
空闲时：完全不渲染 → CPU/GPU 功耗 ≈ 0
光标闪烁：2fps × FullRender ≈ 14ms/秒 CPU → 可忽略
输入/滚动：60fps FullRender → 与改造前相同
```

核心洞察：当前问题不是"每帧渲染太慢"，而是"不该渲染的时候也在渲染"。从 60fps 降到 2fps 就已获得 97% 能耗降低。

### 3.2 Reshape Worker 唤醒机制

当前 `drain_reshape_results()` 在 `render()` 中调用。改为按需渲染后，空闲时不调 `render()`，reshape 结果到达后需要主动唤醒事件循环。

方案：引入 `winit::event_loop::EventLoopProxy`，reshape worker 完成后通过 proxy 发送自定义事件唤醒主线程。

### 3.3 光标闪烁触发渲染的时序

`WaitUntil(blink_deadline)` 到期后，winit 调用 `about_to_wait()`。此时 `needs_redraw` 是 false（无用户输入），需要在 `about_to_wait()` 中**主动检测光标闪烁状态是否变化**来触发渲染。

---

## 四、总体架构变更

```
改造前:
  about_to_wait → 总是 request_redraw → RedrawRequested → render() → GPU submit+present
                                                                         ↑
                                                        always (for cursor blink)

改造后:
  about_to_wait → 检查:
    │  1. needs_redraw?           → request_redraw() + FullRender
    │  2. has_active_animation?   → request_redraw() + FullRender
    │  3. 光标闪烁 phase 变化?    → needs_redraw=true, request_redraw() + FullRender
    │  4. 都不满足?               → 不调 request_redraw()
    │
    └→ compute_next_wake_time() → ControlFlow::WaitUntil(deadline) 或 Wait

  新增: EventLoopProxy → reshape worker 结果到达 → UserEvent → needs_redraw=true
```

---

## 五、分阶段实施计划

### 阶段 A：渲染调度重构（核心）

**目标**：消除无条件 `request_redraw`，引入 `WaitUntil` 精确调度，让 CPU/GPU 空闲时真正休眠

**改动范围**：`crates/app/src/app.rs`

---

#### A.1 新增 `last_cursor_phase` 字段

在 `App` 结构体中新增，用于检测光标闪烁 phase 是否发生变化：

```rust
// App struct 新增字段
/// 上一帧的光标可见状态，用于 about_to_wait 检测 phase 变化
last_cursor_visible: bool,
```

初始化为 `true`（与 `cursor_blink_instant = Instant::now()` 一致）。

---

#### A.2 新增 `compute_cursor_phase()` 辅助函数

独立函数，计算当前光标可见性和下一次切换时间：

```rust
/// 计算光标当前是否可见，以及下一次切换的时间点。
fn compute_cursor_phase(cursor_blink_instant: Instant) -> (bool, Instant) {
    let elapsed_ms = cursor_blink_instant.elapsed().as_millis() as u64;
    let period_ms: u64 = 500;
    let phase_in_period = elapsed_ms % (period_ms * 2);

    let currently_visible = phase_in_period < period_ms;
    let next_transition_ms = if currently_visible {
        period_ms - phase_in_period
    } else {
        period_ms * 2 - phase_in_period
    };

    // +5ms 容差，避免 WaitUntil 精度不足导致 phase 未变就被唤醒
    let next_deadline = Instant::now() + Duration::from_millis(next_transition_ms + 5);

    (currently_visible, next_deadline)
}
```

---

#### A.3 新增 `has_active_animation()` 方法

```rust
impl App {
    /// 是否有正在进行的动画（标签栏滚动）需要持续渲染。
    fn has_active_animation(&self) -> bool {
        // 标签栏滚动动画
        let tab_animating = (self.workspace.tab_scroll_target
            - self.workspace.tab_scroll_offset).abs() >= 0.5;
        tab_animating
    }
}
```

---

#### A.4 新增 `compute_next_wake_time()` 方法

收集所有需要未来唤醒的事件源，取最近时间：

```rust
impl App {
    /// 计算下一次需要唤醒事件循环的时间点。
    /// 返回 None 表示可以无限期休眠（完全空闲）。
    fn compute_next_wake_time(&self) -> Option<Instant> {
        let mut earliest: Option<Instant> = None;

        // 1. 光标闪烁 — 有文档且有光标时才需要
        if let Some(dv) = self.workspace.doc_views.get(self.workspace.active_index) {
            let (_, next_blink) = compute_cursor_phase(
                dv.cursor_render_state.cursor_blink_instant
            );
            earliest = Some(match earliest {
                Some(e) => e.min(next_blink),
                None => next_blink,
            });
        }

        // 2. 标签栏滚动动画 — 动画运行期间每 16ms 唤醒一帧
        if self.has_active_animation() {
            let next_frame = Instant::now() + Duration::from_millis(16);
            earliest = Some(match earliest {
                Some(e) => e.min(next_frame),
                None => next_frame,
            });
        }

        earliest
    }
}
```

---

#### A.5 重构 `about_to_wait()`

**这是核心改动**。完整替换当前 `about_to_wait()` 中 `request_redraw` 逻辑：

```rust
fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
    let _atw_t0 = std::time::Instant::now();

    // 1. Poll 菜单动作（不变）
    let actions: Vec<MenuAction> = if let Some(ref nm) = self.native_menu {
        std::iter::from_fn(|| nm.poll_action()).collect()
    } else {
        Vec::new()
    };
    for action in actions {
        if action == MenuAction::Quit {
            event_loop.exit();
            return;
        }
        self.dispatch_menu_action(action, event_loop);
    }

    let Some(window) = &self.window else { return };

    // 2. 检测光标闪烁 phase 变化 → 触发渲染
    if let Some(dv) = self.workspace.doc_views.get(self.workspace.active_index) {
        let (visible, _) = compute_cursor_phase(
            dv.cursor_render_state.cursor_blink_instant
        );
        if visible != self.last_cursor_visible {
            self.last_cursor_visible = visible;
            self.needs_redraw = true;
        }
    }

    // 3. 检测动画 → 触发渲染
    if self.has_active_animation() {
        self.needs_redraw = true;
    }

    // 4. 仅在有需要时 request_redraw
    if self.needs_redraw {
        window.request_redraw();
    }

    // 5. 设置 ControlFlow — 精确调度下一次唤醒
    let next_wake = self.compute_next_wake_time();
    match next_wake {
        Some(deadline) => {
            event_loop.set_control_flow(
                winit::event_loop::ControlFlow::WaitUntil(deadline)
            );
        }
        None => {
            event_loop.set_control_flow(
                winit::event_loop::ControlFlow::Wait
            );
        }
    }

    // perf 日志（保留现有逻辑）
    let _atw_us = _atw_t0.elapsed().as_micros();
    if _atw_us > 2000 {
        let _ = std::fs::OpenOptions::new().create(true).append(true)
            .open("/tmp/perf.log")
            .and_then(|mut f| {
                use std::io::Write;
                writeln!(f, "[event] AboutToWait={}us needs_redraw={}", _atw_us, self.needs_redraw)
            });
    }
    self.update_ime_cursor_area();
}
```

**关键点说明**：

- **步骤 2** 解决了原方案遗漏的问题：`WaitUntil` 到期后 `needs_redraw` 是 false，必须主动检测 phase 变化
- **步骤 4** 取消了 `else if self.gpu.is_some() && self.text.is_some()` 分支——这是能耗问题的根源
- **步骤 5** 总是设置 `ControlFlow`，确保光标闪烁有精确的定时唤醒，无事件时真正进入 `Wait`

---

#### A.6 `render()` 尾部保持不变

`render()` 结尾的 `needs_redraw = false` 逻辑维持原样：

```rust
// app.rs:1711-1715 — 不变
if self.scrollbar_dragging {
    // 拖拽期间保持 needs_redraw 以接收异步 shaping 结果
} else {
    self.needs_redraw = false;
}
```

---

#### A.7 `scrollbar_dragging` 期间的策略

拖拽滚动条时内容在持续变化，且 `drain_reshape_results()` 需要在 `render()` 中被调用。
现有逻辑已经正确处理（拖拽期间保持 `needs_redraw = true`），无需额外改动。

---

### 阶段 B：异步唤醒机制

**目标**：确保 reshape worker 的异步结果能及时唤醒处于 `Wait` 状态的事件循环

**改动范围**：
- `crates/app/src/app.rs`
- `crates/app/src/reshape_worker.rs`
- `crates/app/src/main.rs`（或 event loop 创建位置）

---

#### B.1 定义自定义事件类型

```rust
// app.rs 或新文件 event_types.rs
#[derive(Debug, Clone)]
pub enum AppEvent {
    /// reshape worker 有结果就绪，需要唤醒主线程处理
    ReshapeResultsReady,
}
```

---

#### B.2 改用 `EventLoop::with_user_event()` 创建事件循环

在 `main.rs`（或 event loop 创建位置）中，将 `EventLoop::new()` 改为 `EventLoop::with_user_event::<AppEvent>()`。

把 `event_loop.create_proxy()` 获得的 `EventLoopProxy<AppEvent>` 传入 `App::new()`。

---

#### B.3 App 保存 EventLoopProxy

```rust
// App struct 新增字段
event_loop_proxy: Option<winit::event_loop::EventLoopProxy<AppEvent>>,
```

在 `App::new()` 中接收并存储。

---

#### B.4 ReshapeWorker 持有 EventLoopProxy 的 clone

修改 `ReshapeWorker::spawn()` 签名，接收 `EventLoopProxy<AppEvent>`：

```rust
pub fn spawn(
    font_family: String,
    proxy: winit::event_loop::EventLoopProxy<AppEvent>,
) -> Self {
    // ...
    let handle = thread::Builder::new()
        .name("reshape-worker".into())
        .spawn(move || {
            for cmd in cmd_rx {
                match cmd {
                    WorkerCommand::Shape(req) => {
                        // ... 现有处理逻辑 ...
                        let _ = result_tx.send(result);
                        // 唤醒主线程
                        let _ = proxy.send_event(AppEvent::ReshapeResultsReady);
                    }
                    WorkerCommand::Shutdown => break,
                }
            }
        })
        .expect("failed to spawn reshape worker");
    // ...
}
```

---

#### B.5 处理自定义事件

在 `ApplicationHandler` 实现中新增 `user_event` 处理：

```rust
impl ApplicationHandler<AppEvent> for App {
    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::ReshapeResultsReady => {
                // drain_reshape_results 在 render() 中调用
                // 这里只需标记需要重绘
                self.needs_redraw = true;
            }
        }
    }
    // ... 其余 handler 不变 ...
}
```

这样 reshape worker 完成时，主线程即使在 `Wait` 状态也会被唤醒，进入 `about_to_wait()` → 检测 `needs_redraw` → `request_redraw()` → `render()` → `drain_reshape_results()`。

---

#### B.6 处理 drain 但不在 render 中的情况

当前 `drain_reshape_results()` 只在 `render()` 的 `app.rs:1423` 处调用。
如果 `ReshapeResultsReady` 事件到达时 `needs_redraw` 已为 true（比如用户正在输入），
drain 会在下一帧 `render()` 时自然处理——无需特殊处理。

**无需额外改动**：现有 drain 路径已足够。

---

### 阶段 C：测试与验证

**目标**：确保改造不引入渲染 bug、不影响输入响应、能耗确实下降

---

#### C.1 功能回归测试

| 测试场景 | 验证点 |
|----------|--------|
| 打开文件 | 内容正常渲染 |
| 键盘输入 | 字符即时显示，无延迟 |
| 删除/撤销/重做 | 内容正确更新 |
| 光标闪烁 | 正常 500ms 周期闪烁，无抖动 |
| 鼠标点击定位 | 光标正确移动 |
| 选区拖拽 | 高亮实时更新 |
| 搜索高亮 | 匹配项正确高亮 |
| 滚动（鼠标/触摸板） | 平滑无卡顿 |
| 窗口 resize | 内容自适应 |
| 主题切换 | 颜色即时更新 |
| 标签栏切换 | 动画平滑 |
| 标签栏滚动动画 | 动画结束后 CPU 静默 |
| IME 输入 | 组合窗口正常 |
| 长时间空闲 | reshape 结果仍能被处理 |
| 滚动条拖拽 | 实时跟随，停手后停止渲染 |

---

#### C.2 能耗测试

| 测试方法 | 指标 |
|----------|------|
| `powermetrics` (macOS) | 空闲时的 CPU/GPU 功耗 (mW) |
| Activity Monitor → Energy | 12 小时平均能耗影响 |
| `sudo powermetrics --samplers tasks -n 1` | 进程唤醒频率 |
| Xcode Energy Gauge | 能耗评级 |

**验收标准**：
- 空闲时 CPU 唤醒频率 < 5 次/秒（改造前 ~60 次/秒）
- 空闲时 GPU 处于 idle 状态（改造前持续 active）
- App Nap 正常工作（可通过 `pmset -g assertions` 验证）

---

#### C.3 性能测试

确保输入响应延迟不退化：
- 按键到屏幕更新 < 16ms（1 帧内）
- 滚动帧率保持 60fps
- 大文件（10 万行）滚动不掉帧

---

## 六、风险与缓解

| 风险 | 影响 | 缓解措施 |
|------|------|----------|
| `WaitUntil` 精度不足 | 光标闪烁抖动 | 加 5ms 容差；必要时退回到稍高频轮询 |
| 遗漏 `needs_redraw` 设置点 | 某些变化不触发渲染 | 全局搜索所有 `needs_redraw = true` 路径确认覆盖 |
| reshape 结果延迟处理 | 打开大文件后 word-wrap 更新慢 | `ReshapeResultsReady` 事件确保及时唤醒 |
| winit 0.30 的 `WaitUntil` 行为差异 | 跨平台不一致 | macOS 优先验证，Linux/Windows 后续跟进 |
| `ApplicationHandler<AppEvent>` 泛型改动 | 编译错误范围扩大 | event loop 创建和 App 定义一起改，确保一次编译通过 |
| 菜单 poll 在 Wait 中不响应 | native_menu 动作延迟 | `about_to_wait` 已有 poll 逻辑，WaitUntil 的 500ms 上限确保最多半秒延迟 |

---

## 七、仅改动的文件

| 文件 | 改动内容 | 阶段 |
|------|----------|------|
| `crates/app/src/app.rs` | `last_cursor_visible` 字段、`compute_cursor_phase()`、`has_active_animation()`、`compute_next_wake_time()`、`about_to_wait()` 重构、`AppEvent` 定义、`user_event()` handler | A + B |
| `crates/app/src/reshape_worker.rs` | `spawn()` 接收 `EventLoopProxy`，完成后 `send_event` | B |
| `crates/app/src/main.rs` | `EventLoop::with_user_event()`、`create_proxy()` | B |

**总计 3 个文件**，改动集中可控。`crates/ui/` 无需改动。

---

## 八、实施优先级与分工

| 优先级 | 阶段 | 预估工作量 | 改造收益 |
|--------|------|-----------|---------|
| **P0** | 阶段 A (渲染调度重构) | 中 | 获得 ~90% 的能耗降低 |
| **P0** | 阶段 B (异步唤醒机制) | 小 | 确保 reshape 结果不丢失 |
| **P1** | 阶段 C (测试验证) | 中 | 确保改造质量 |

建议：阶段 A + B 一起做（一次提交），阶段 C 跟进验证。

---

## 九、预期收益

### 改造后的能耗模型

| 场景 | CPU 唤醒频率 | GPU 状态 | 预估功耗 |
|------|------------|----------|---------|
| 完全空闲（无光标） | 0 次/秒 | idle | ~0W |
| 光标闪烁中 | 2 次/秒 | 2 次 render pass | ~0.1W |
| 打字中 | ~30-60 次/秒 | 活跃 | ~2-4W（同改造前） |
| 滚动中 | 60 次/秒 | 活跃 | ~3-5W（同改造前） |

### 用户可感知的改善

- **电池续航**：空闲编辑器场景延长 **15-30%**
- **风扇噪音**：空闲时风扇完全停止
- **系统温度**：空闲时 CPU 温度降低 **5-10°C**
- **App Nap**：正常工作，系统可将 edit+ 降到最低优先级
