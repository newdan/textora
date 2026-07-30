# mmap Tapered Path Join Gap Fix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 补齐 mmap 渐细连接线每个非共线采样点外弯侧的实心主体三角区域，消除白色缺口，同时保持现有曲线轮廓与缓存性能。

**Architecture:** 修复限定在 `ui::tapered_path` 的纯几何 tessellator。主体梯形生成保持不变；转角处理中先追加由采样点中心和前后外侧边缘点组成的实心三角形，再沿用现有 miter 或圆角外缘补片与羽化逻辑。

**Tech Stack:** Rust、textora-ui 纯几何模块、Cargo 单元测试。

## Global Constraints

- 不提高 mmap 圆弧的 8 段采样密度。
- 不修改中心线、渐细宽度插值、miter/圆角策略、羽化、连接线缓存或渲染后端接口。
- 每个非共线采样点最多增加一个三角形，即 3 个顶点。
- 遵守跨层解耦：`ui` 不依赖 mmap、`DocumentView`、Workspace、Commands 或 Events。
- Rust 代码不得新增 `.unwrap()`；确认不会失败时使用带明确理由的 `.expect(...)`。
- 修改后运行 `cargo fmt`；全面验证运行 `./scripts/verify.sh`。

## File Structure

- Modify and test: `crates/ui/src/tapered_path.rs` — 通用渐细路径 tessellation、转角主体补片及其纯几何回归测试。
- No change: `crates/markdown/src/mmf/canvas.rs` — mmap 继续提供相同中心线和宽度输入。
- No change: `crates/app/src/paint_backend.rs` — app 继续提交和裁剪相同的 `TaperedMesh` 数据结构。

---

### Task 1: 补齐非共线转角的实心主体覆盖

**Files:**
- Modify: `crates/ui/src/tapered_path.rs:142-248`
- Test: `crates/ui/src/tapered_path.rs:671-813`

**Interfaces:**
- Consumes: `PathSample { center: [f32; 2], half_width: f32, .. }`、`direction(start, end) -> [f32; 2]`、`cross(left, right) -> f32`、`offset_join_point(sample, direction, side, half_width) -> [f32; 2]`。
- Produces: 私有 helper `append_outer_turn_body_triangle(vertices: &mut Vec<TaperedMeshVertex>, sample: PathSample, previous_direction: [f32; 2], next_direction: [f32; 2], turn: f32)`；公开 API 与数据结构不变。

- [ ] **Step 1: 添加测试用的实心三角形覆盖判断**

在测试模块的 `mesh_for` 后加入：

```rust
fn solid_mesh_covers_point(mesh: &TaperedMesh, point: [f32; 2]) -> bool {
    mesh.vertices.chunks_exact(3).any(|triangle| {
        if !triangle.iter().all(|vertex| vertex.alpha_multiplier == 1.0) {
            return false;
        }

        let edge_sign = |start: [f32; 2], end: [f32; 2]| {
            (point[0] - end[0]) * (start[1] - end[1])
                - (start[0] - end[0]) * (point[1] - end[1])
        };
        let first = edge_sign(triangle[0].position, triangle[1].position);
        let second = edge_sign(triangle[1].position, triangle[2].position);
        let third = edge_sign(triangle[2].position, triangle[0].position);
        let has_negative =
            first < -f32::EPSILON || second < -f32::EPSILON || third < -f32::EPSILON;
        let has_positive =
            first > f32::EPSILON || second > f32::EPSILON || third > f32::EPSILON;

        !has_negative || !has_positive
    })
}

fn outer_turn_probe(centerline: &[[f32; 2]; 3]) -> [f32; 2] {
    let samples = path_samples(centerline, 10.0, 2.0);
    let joint = samples[1];
    let previous_direction = direction(centerline[0], centerline[1]);
    let next_direction = direction(centerline[1], centerline[2]);
    let outer_side = -cross(previous_direction, next_direction).signum();
    let previous_outer =
        offset_join_point(joint, previous_direction, outer_side, joint.half_width);
    let next_outer = offset_join_point(joint, next_direction, outer_side, joint.half_width);

    [
        (joint.center[0] + previous_outer[0] + next_outer[0]) / 3.0,
        (joint.center[1] + previous_outer[1] + next_outer[1]) / 3.0,
    ]
}
```

- [ ] **Step 2: 写入正向浅弯失败回归测试**

在现有曲线路径测试之前加入：

```rust
#[test]
fn positive_shallow_turn_has_solid_outer_body_coverage() {
    let centerline = [[0.0, 0.0], [100.0, 0.0], [119.615_71, 3.901_806_4]];
    let mesh = mesh_for(&centerline);
    let probe = outer_turn_probe(&centerline);

    assert!(
        solid_mesh_covers_point(&mesh, probe),
        "正向浅弯的外侧主体不得在分段接缝处露出背景"
    );
}
```

- [ ] **Step 3: 运行正向浅弯测试并确认按预期失败**

Run:

```bash
cargo test -p textora-ui --lib tapered_path::tests::positive_shallow_turn_has_solid_outer_body_coverage -- --exact
```

Expected: FAIL，断言信息包含“正向浅弯的外侧主体不得在分段接缝处露出背景”；失败原因是 probe 位于当前未覆盖的中心三角区域，而不是编译错误。

- [ ] **Step 4: 写入反向浅弯失败回归测试**

紧接正向测试加入镜像测试：

```rust
#[test]
fn negative_shallow_turn_has_solid_outer_body_coverage() {
    let centerline = [[0.0, 0.0], [100.0, 0.0], [119.615_71, -3.901_806_4]];
    let mesh = mesh_for(&centerline);
    let probe = outer_turn_probe(&centerline);

    assert!(
        solid_mesh_covers_point(&mesh, probe),
        "反向浅弯的外侧主体不得在分段接缝处露出背景"
    );
}
```

- [ ] **Step 5: 运行两个浅弯测试并确认均按预期失败**

Run:

```bash
cargo test -p textora-ui --lib shallow_turn_has_solid_outer_body_coverage
```

Expected: 2 tests FAILED；两个失败均来自各自的主体覆盖断言。

- [ ] **Step 6: 实现最小的外弯侧中心三角补片**

在 `append_round_join_triangles` 的非共线分支中，位于 `miter_within_limit` 判断之前加入调用：

```rust
append_outer_turn_body_triangle(
    vertices,
    samples[index],
    previous_direction,
    next_direction,
    turn,
);
```

在 `append_miter_join` 之前定义 helper：

```rust
fn append_outer_turn_body_triangle(
    vertices: &mut Vec<TaperedMeshVertex>,
    sample: PathSample,
    previous_direction: [f32; 2],
    next_direction: [f32; 2],
    turn: f32,
) {
    let outer_side = -turn.signum();
    let previous_outer =
        offset_join_point(sample, previous_direction, outer_side, sample.half_width);
    let next_outer = offset_join_point(sample, next_direction, outer_side, sample.half_width);
    append_triangle(vertices, sample.center, previous_outer, next_outer, 1.0);
}
```

不要修改 `append_miter_join`、`append_round_join`、羽化函数、中心线采样或公开类型。

- [ ] **Step 7: 格式化并验证两个回归测试转绿**

Run:

```bash
cargo fmt
cargo test -p textora-ui --lib shallow_turn_has_solid_outer_body_coverage
```

Expected: 2 tests PASSED。

- [ ] **Step 8: 运行 tapered-path 全部单元测试**

Run:

```bash
cargo test -p textora-ui --lib tapered_path
```

Expected: 所有 tapered-path 测试 PASSED；直线、直角 miter、锐角圆角、羽化和反向折返测试无回归。

- [ ] **Step 9: 运行 mmap 与渲染后端定向回归**

Run:

```bash
cargo test -p textora-markdown --lib mmf
cargo test -p textora-app --lib paint_backend
cargo check -p textora-app
```

Expected: 三条命令退出码均为 0，无编译错误或测试失败。

- [ ] **Step 10: 运行项目全面验证**

Run:

```bash
./scripts/verify.sh
```

Expected: 脚本退出码为 0；格式、编译、测试和项目约束检查全部通过。

- [ ] **Step 11: 检查差异并提交修复**

Run:

```bash
git diff --check
git status --short
git diff -- crates/ui/src/tapered_path.rs
git add crates/ui/src/tapered_path.rs
git commit -m "fix(ui): 补齐渐细路径转角主体缺口"
```

Expected: 提交只包含 `crates/ui/src/tapered_path.rs` 的测试与最小几何修复；提交成功。
