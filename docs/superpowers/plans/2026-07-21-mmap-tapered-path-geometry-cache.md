# mmap Tapered Path Geometry Cache Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 用单个可缓存的渐细三角网格替换 mmap 连接线的每像素圆点串，在保持中心线、渐细、圆角转弯与圆头视觉不变的前提下，将滚动帧连接线顶点降至数千级。

**Architecture:** `textora-ui` 提供纯数据 `TaperedMesh` 与无 GPU tessellator，`DrawList` 只携带共享网格、平移和颜色；`textora-app` 只负责屏幕变换、颜色合成、NDC 转换和精确裁剪；`textora-markdown` 保留既有中心线算法并在 `MindmapView` 的 Ready 布局旁按 zoom 缓存静态连接线网格。动态拖拽引导线使用同一网格接口但不缓存。

**Tech Stack:** Rust 2024、textora-ui 纯几何、textora-app wgpu 顶点后端、textora-markdown MMF 画布、Cargo 单元测试。

## Global Constraints

- 全程遵守 `crates/ui` 不依赖 app 状态的跨层红线；UI 输入只能是纯数据。
- 不新增专用 GPU Shader、渲染管线、绑定状态或第三方依赖。
- 中心线、全程渐细宽度、圆角转弯和圆头必须肉眼一致；仅允许 0.5px 羽化边缘存在极小差异。
- 静态连接线滚动时必须复用几何缓存；zoom、布局、主题几何、DPI 或源码变化必须失效。
- 颜色、悬停、光标闪烁和拖拽置灰不得使静态几何缓存失效。
- 严禁新增 mmap 专用 app 状态或分支。
- 所有生产代码必须先有能观察到正确失败原因的测试。
- 命名必须精准自解释；不得使用 `data`、`info`、`temp`、`res`、`flag` 等宽泛名称。
- 不得使用无说明的 `.unwrap()`；确知不失败处使用带具体理由的 `.expect(...)`。
- 每个子任务提交前运行 `cargo fmt --check` 和相应编译/测试；最终运行 `./scripts/verify.sh`。
- 设计依据：`docs/superpowers/specs/2026-07-21-mmap-tapered-path-geometry-cache-design.md`。

---

## File Map

- Create `crates/ui/src/tapered_path.rs`: 纯路径归一化、渐细宽度、主体三角带、圆头、圆形连接补片和羽化网格。
- Modify `crates/ui/src/lib.rs`: 公开 `tapered_path` 模块。
- Modify `crates/ui/src/core/paint.rs`: 新增共享 `TaperedMesh` 绘制命令和应用容器偏移的 helper。
- Modify `crates/app/src/paint_backend.rs`: 提交共享网格，合成颜色，提供包围盒裁剪快速路径和边界三角形精确裁剪。
- Modify `crates/markdown/src/mmf/canvas.rs`: 保留中心线算法，迁移静态连接线及动态拖拽引导线，提供按 zoom 构建的连接线网格集合。
- Modify `crates/markdown/src/mindmap_view.rs`: 在 Ready 布局旁持有缓存，统一复用和失效。

---

### Task 1: 实现纯 UI 渐细网格几何

**Files:**
- Create: `crates/ui/src/tapered_path.rs`
- Modify: `crates/ui/src/lib.rs`
- Test: `crates/ui/src/tapered_path.rs`

**Interfaces:**
- Consumes: `ui::core::geom::Rect`。
- Produces:

```rust
pub const TAPERED_PATH_FEATHER_PX: f32 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaperedMeshVertex {
    pub position: [f32; 2],
    pub alpha_multiplier: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TaperedMesh {
    pub vertices: Box<[TaperedMeshVertex]>,
    pub bounds: Rect,
}

#[derive(Clone, Copy, Debug)]
pub struct TaperedPathInput<'a> {
    pub centerline: &'a [[f32; 2]],
    pub head_width: f32,
    pub tail_width: f32,
    pub scale: f32,
    pub feather_width: f32,
}

pub fn tessellate_tapered_path(input: TaperedPathInput<'_>) -> Option<TaperedMesh>;
```

- [ ] **Step 1: 写入长短路径、圆头、渐细、羽化和退化输入失败测试**

在新文件的 `#[cfg(test)] mod tests` 中先写测试，并在 `crates/ui/src/lib.rs` 加入 `pub mod tapered_path;` 使测试参与编译：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn straight_path(length: f32) -> [[f32; 2]; 2] {
        [[0.0, 0.0], [length, 0.0]]
    }

    fn mesh_for(centerline: &[[f32; 2]]) -> TaperedMesh {
        tessellate_tapered_path(TaperedPathInput {
            centerline,
            head_width: 10.0,
            tail_width: 2.0,
            scale: 1.0,
            feather_width: TAPERED_PATH_FEATHER_PX,
        })
        .expect("a finite two-point centerline must tessellate")
    }

    #[test]
    fn straight_path_vertex_count_does_not_depend_on_pixel_length() {
        let short = mesh_for(&straight_path(100.0));
        let long = mesh_for(&straight_path(1_000.0));

        assert_eq!(short.vertices.len(), long.vertices.len());
        assert!(long.vertices.len() < 512);
    }

    #[test]
    fn straight_path_keeps_round_caps_taper_and_feather() {
        let mesh = mesh_for(&straight_path(1_000.0));

        assert!((mesh.bounds.x + 5.5).abs() < 0.01);
        assert!((mesh.bounds.right() - 1_001.5).abs() < 0.01);
        assert!((mesh.bounds.y + 5.5).abs() < 0.01);
        assert!((mesh.bounds.bottom() - 5.5).abs() < 0.01);
        assert!(mesh.vertices.iter().any(|vertex| vertex.alpha_multiplier == 0.0));
        assert!(mesh.vertices.iter().any(|vertex| vertex.alpha_multiplier == 1.0));
    }

    #[test]
    fn curved_and_reversing_paths_emit_only_finite_vertices() {
        for centerline in [
            vec![[0.0, 0.0], [50.0, 0.0], [50.0, 50.0]],
            vec![[0.0, 0.0], [50.0, 0.0], [0.0, 0.0]],
            vec![[0.0, 0.0], [0.0, 0.0], [50.0, 0.0]],
        ] {
            let mesh = mesh_for(&centerline);
            assert!(mesh.vertices.iter().all(|vertex| {
                vertex.position[0].is_finite()
                    && vertex.position[1].is_finite()
                    && vertex.alpha_multiplier.is_finite()
            }));
        }
    }

    #[test]
    fn invalid_path_inputs_do_not_emit_meshes() {
        for input in [
            TaperedPathInput {
                centerline: &[[0.0, 0.0]],
                head_width: 10.0,
                tail_width: 2.0,
                scale: 1.0,
                feather_width: TAPERED_PATH_FEATHER_PX,
            },
            TaperedPathInput {
                centerline: &[[0.0, 0.0], [f32::NAN, 1.0]],
                head_width: 10.0,
                tail_width: 2.0,
                scale: 1.0,
                feather_width: TAPERED_PATH_FEATHER_PX,
            },
            TaperedPathInput {
                centerline: &[[0.0, 0.0], [10.0, 0.0]],
                head_width: 0.0,
                tail_width: 2.0,
                scale: 1.0,
                feather_width: TAPERED_PATH_FEATHER_PX,
            },
        ] {
            assert!(tessellate_tapered_path(input).is_none());
        }
    }
}
```

- [ ] **Step 2: 运行测试并确认 RED**

Run: `cargo test -p textora-ui --lib -- tapered_path`

Expected: FAIL，缺少 `TaperedMesh`、`TaperedPathInput` 和 `tessellate_tapered_path`；失败原因必须是新接口尚未实现，而不是模块路径或语法错误。

- [ ] **Step 3: 实现输入归一化、主体、圆头、连接补片和羽化网格**

实现时使用以下职责清晰的私有类型与函数，避免单个函数超过 50 行：

```rust
const ZERO_LENGTH_EPSILON: f32 = 0.01;
const MIN_CAP_SEGMENTS: usize = 8;
const MAX_CAP_SEGMENTS: usize = 32;
const MAX_MITER_RATIO: f32 = 2.0;

#[derive(Clone, Copy)]
struct PathSample {
    center: [f32; 2],
    tangent: [f32; 2],
    half_width: f32,
}

fn normalized_centerline(input: TaperedPathInput<'_>) -> Option<Vec<[f32; 2]>>;
fn path_samples(centerline: &[[f32; 2]], head_width: f32, tail_width: f32) -> Vec<PathSample>;
fn append_body_triangles(vertices: &mut Vec<TaperedMeshVertex>, samples: &[PathSample]);
fn append_round_join_triangles(vertices: &mut Vec<TaperedMeshVertex>, samples: &[PathSample]);
fn append_round_cap_triangles(
    vertices: &mut Vec<TaperedMeshVertex>,
    sample: PathSample,
    outward_direction: [f32; 2],
);
fn append_feather_triangles(
    vertices: &mut Vec<TaperedMeshVertex>,
    samples: &[PathSample],
    feather_width: f32,
);
fn mesh_bounds(vertices: &[TaperedMeshVertex]) -> Option<Rect>;
```

实现约束：

```rust
let scaled_head_width = input.head_width * input.scale;
let scaled_tail_width = input.tail_width * input.scale;
let scaled_centerline = normalized_centerline(input)?;
let samples = path_samples(&scaled_centerline, scaled_head_width, scaled_tail_width);

// 主体和内侧圆头 alpha_multiplier = 1.0。
// 羽化内缘 alpha_multiplier = 1.0，外缘 = 0.0。
// 宽度按全路径累计长度插值，不得按段重新开始。
// miter 超过 MAX_MITER_RATIO * half_width 时使用圆形连接补片。
```

不要按像素长度增加中心线采样；直线路径无论 100px 还是 1000px 都使用相同拓扑。圆头细分只由端点半径决定并限制在 `MIN_CAP_SEGMENTS..=MAX_CAP_SEGMENTS`。

- [ ] **Step 4: 运行 UI 几何测试并确认 GREEN**

Run: `cargo fmt --check && cargo test -p textora-ui --lib -- tapered_path && cargo check -p textora-app`

Expected: PASS；长短直线路径顶点数相同，所有顶点有限，羽化同时包含 alpha 0 与 1，app 继续编译。

- [ ] **Step 5: 提交纯 UI 几何**

```bash
git add crates/ui/src/tapered_path.rs crates/ui/src/lib.rs
git commit -m "feat(ui): tessellate tapered path meshes"
```

---

### Task 2: 增加共享 tapered mesh 命令并在 app 后端提交

**Files:**
- Modify: `crates/ui/src/core/paint.rs`
- Test: `crates/ui/src/core/paint.rs`
- Modify: `crates/app/src/paint_backend.rs`
- Test: `crates/app/src/paint_backend.rs`

**Interfaces:**
- Consumes: `ui::tapered_path::TaperedMesh`。
- Produces:

```rust
DrawCmd::TaperedMesh {
    mesh: Arc<TaperedMesh>,
    translation: [f32; 2],
    color: [f32; 4],
}

DrawList::tapered_mesh(
    &mut self,
    mesh: Arc<TaperedMesh>,
    translation: [f32; 2],
    color: [f32; 4],
)
```

- Consumes: `DrawCmd::TaperedMesh { mesh, translation, color }` 和 `TaperedMeshVertex`。
- Produces: `drain()` 对新命令输出经过颜色合成、屏幕变换和精确裁剪的 `GlyphVertex`；不新增 app 公共接口。

- [ ] **Step 1: 写共享 Arc 与偏移失败测试**

```rust
#[test]
fn tapered_mesh_command_shares_geometry_and_applies_draw_list_offset() {
    use crate::tapered_path::{
        TAPERED_PATH_FEATHER_PX, TaperedPathInput, tessellate_tapered_path,
    };

    let centerline = [[0.0, 0.0], [100.0, 0.0]];
    let mesh = Arc::new(
        tessellate_tapered_path(TaperedPathInput {
            centerline: &centerline,
            head_width: 10.0,
            tail_width: 2.0,
            scale: 1.0,
            feather_width: TAPERED_PATH_FEATHER_PX,
        })
        .expect("fixture must tessellate"),
    );
    let mut draw_list = DrawList::new();
    draw_list.offset = (7.0, 11.0);
    draw_list.tapered_mesh(Arc::clone(&mesh), [13.0, 17.0], [0.2, 0.4, 0.6, 0.8]);
    let cloned = draw_list.clone();

    match (&draw_list.cmds[0], &cloned.cmds[0]) {
        (
            DrawCmd::TaperedMesh { mesh: first, translation, color },
            DrawCmd::TaperedMesh { mesh: second, .. },
        ) => {
            assert!(Arc::ptr_eq(first, &mesh));
            assert!(Arc::ptr_eq(first, second));
            assert_eq!(*translation, [20.0, 28.0]);
            assert_eq!(*color, [0.2, 0.4, 0.6, 0.8]);
        }
        _ => panic!("expected one shared tapered mesh command"),
    }
}
```

- [ ] **Step 2: 运行测试并确认 RED**

Run: `cargo test -p textora-ui --lib -- tapered_mesh_command_shares_geometry`

Expected: FAIL，`DrawCmd::TaperedMesh` 和 `DrawList::tapered_mesh` 尚不存在。

- [ ] **Step 3: 添加命令与 helper**

在 `DrawCmd` 中加入设计接口，并实现：

```rust
pub fn tapered_mesh(
    &mut self,
    mesh: Arc<TaperedMesh>,
    translation: [f32; 2],
    color: [f32; 4],
) {
    self.cmds.push(DrawCmd::TaperedMesh {
        mesh,
        translation: [translation[0] + self.offset.0, translation[1] + self.offset.1],
        color,
    });
}
```

`paint.rs` 顶部显式导入 `crate::tapered_path::TaperedMesh`，继续复用现有 `std::sync::Arc`，不要引入 app 类型。

- [ ] **Step 4: 写完全包含、完全排除和边界裁剪失败测试**

加入测试 helper 与三个测试：

```rust
fn triangle_mesh(vertices: [[f32; 2]; 3], alpha: [f32; 3]) -> std::sync::Arc<ui::tapered_path::TaperedMesh> {
    use ui::tapered_path::{TaperedMesh, TaperedMeshVertex};

    let min_x = vertices.iter().map(|point| point[0]).fold(f32::INFINITY, f32::min);
    let max_x = vertices.iter().map(|point| point[0]).fold(f32::NEG_INFINITY, f32::max);
    let min_y = vertices.iter().map(|point| point[1]).fold(f32::INFINITY, f32::min);
    let max_y = vertices.iter().map(|point| point[1]).fold(f32::NEG_INFINITY, f32::max);
    let mesh_vertices = vertices
        .into_iter()
        .zip(alpha)
        .map(|(position, alpha_multiplier)| TaperedMeshVertex {
            position,
            alpha_multiplier,
        })
        .collect::<Vec<_>>()
        .into_boxed_slice();
    std::sync::Arc::new(TaperedMesh {
        vertices: mesh_vertices,
        bounds: Rect::new(min_x, min_y, max_x - min_x, max_y - min_y),
    })
}

#[test]
fn tapered_mesh_inside_clip_preserves_vertices_translation_and_alpha() {
    let mesh = triangle_mesh([[10.0, 10.0], [90.0, 10.0], [50.0, 90.0]], [1.0, 0.5, 0.0]);
    let mut list = DrawList::new();
    list.clip(Rect::new(0.0, 0.0, 200.0, 200.0), |inner| {
        inner.tapered_mesh(mesh, [20.0, 30.0], [0.2, 0.4, 0.6, 0.8]);
    });

    let vertices = drain(list, Screen::new(200.0, 200.0), None, None);
    assert_eq!(vertices.len(), 3);
    assert_eq!(vertices[0].color, [0.2, 0.4, 0.6, 0.8]);
    assert_eq!(vertices[1].color, [0.2, 0.4, 0.6, 0.4]);
    assert_eq!(vertices[2].color, [0.2, 0.4, 0.6, 0.0]);
}

#[test]
fn tapered_mesh_outside_clip_emits_no_vertices() {
    let mesh = triangle_mesh([[10.0, 10.0], [90.0, 10.0], [50.0, 90.0]], [1.0; 3]);
    let mut list = DrawList::new();
    list.clip(Rect::new(300.0, 300.0, 20.0, 20.0), |inner| {
        inner.tapered_mesh(mesh, [0.0, 0.0], [1.0; 4]);
    });

    assert!(drain(list, Screen::new(400.0, 400.0), None, None).is_empty());
}

#[test]
fn tapered_mesh_crossing_clip_stays_inside_and_interpolates_alpha() {
    let mesh = triangle_mesh([[-20.0, 50.0], [120.0, 20.0], [120.0, 80.0]], [0.0, 1.0, 1.0]);
    let mut list = DrawList::new();
    let clip = Rect::new(0.0, 0.0, 100.0, 100.0);
    list.clip(clip, |inner| {
        inner.tapered_mesh(mesh, [0.0, 0.0], [0.3, 0.5, 0.7, 0.8]);
    });

    let screen = Screen::new(100.0, 100.0);
    let clip_ndc = screen.rect_to_ndc(clip);
    let vertices = drain(list, screen, None, None);
    assert!(!vertices.is_empty());
    assert!(vertices.iter().all(|vertex| {
        vertex.position[0] >= clip_ndc[0]
            && vertex.position[0] <= clip_ndc[1]
            && vertex.position[1] <= clip_ndc[2]
            && vertex.position[1] >= clip_ndc[3]
    }));
    assert!(vertices.iter().any(|vertex| vertex.color[3] > 0.0 && vertex.color[3] < 0.8));
}
```

- [ ] **Step 5: 运行 app 测试并确认 RED**

Run: `cargo test -p textora-app --lib -- tapered_mesh_`

Expected: FAIL，`drain()` 尚未覆盖 `DrawCmd::TaperedMesh`，或新命令未生成预期顶点。

- [ ] **Step 6: 实现包围盒关系、颜色合成与局部裁剪**

新增以下私有状态与 helper：

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MeshClipRelation {
    Inside,
    Outside,
    Intersecting,
}

fn translated_rect(rect: Rect, translation: [f32; 2]) -> Rect;
fn mesh_clip_relation(bounds: Rect, clip: Option<Rect>) -> MeshClipRelation;
fn tapered_mesh_vertices(
    mesh: &ui::tapered_path::TaperedMesh,
    translation: [f32; 2],
    color: [f32; 4],
    screen: &Screen,
) -> Vec<GlyphVertex>;
```

在 `drain()` 的 match 中加入：

```rust
DrawCmd::TaperedMesh { mesh, translation, color } => {
    let bounds = translated_rect(mesh.bounds, translation);
    let clip = clip_stack.last().copied();
    match mesh_clip_relation(bounds, clip) {
        MeshClipRelation::Outside => {}
        MeshClipRelation::Inside => {
            vertices.extend(tapered_mesh_vertices(&mesh, translation, color, &screen));
        }
        MeshClipRelation::Intersecting => {
            let tessellated = tapered_mesh_vertices(&mesh, translation, color, &screen);
            let clip = clip.expect("an intersecting mesh requires an active clip");
            append_clipped_triangles(&mut vertices, &tessellated, clip, &screen);
        }
    }
}
```

颜色合成必须使用：

```rust
let vertex_color = [
    color[0],
    color[1],
    color[2],
    color[3] * mesh_vertex.alpha_multiplier,
];
```

完全包含路径不得创建裁剪多边形临时数组；完全排除路径不得遍历网格顶点。

- [ ] **Step 7: 运行 UI 与 app 后端测试并确认 GREEN**

Run: `cargo fmt --check && cargo test -p textora-ui --lib && cargo test -p textora-app --lib -- paint_backend`

Expected: PASS；UI 命令测试、现有圆角裁剪测试和新增 tapered mesh 测试均通过。

- [ ] **Step 8: 编译 app 并提交完整命令能力**

Run: `cargo check -p textora-app`

Expected: PASS。

```bash
git add crates/ui/src/core/paint.rs crates/app/src/paint_backend.rs
git commit -m "feat(render): submit shared tapered meshes"
```

---

### Task 3: 将 mmap 连接线迁移为单网格命令

**Files:**
- Modify: `crates/markdown/src/mmf/canvas.rs`
- Test: `crates/markdown/src/mmf/canvas.rs`

**Interfaces:**
- Consumes: `ui::tapered_path::{TAPERED_PATH_FEATHER_PX, TaperedPathInput, tessellate_tapered_path}` 和 `DrawList::tapered_mesh`。
- Produces:

```rust
fn tapered_connector_mesh(
    centerline: &[(f32, f32)],
    head_width: f32,
    tail_width: f32,
    scale: f32,
) -> Option<std::sync::Arc<ui::tapered_path::TaperedMesh>>;
```

- [ ] **Step 1: 把旧圆点串断言改为单命令失败测试**

保留现有 `connector_uses_yesterdays_rounded_elbows` 与 `connector_centerline_uses_the_supplied_turn_axis`，替换依赖 `FillRect` 采样点的测试：

```rust
fn tapered_mesh_commands(draw_list: &DrawList) -> Vec<&std::sync::Arc<ui::tapered_path::TaperedMesh>> {
    draw_list
        .cmds
        .iter()
        .filter_map(|command| match command {
            DrawCmd::TaperedMesh { mesh, .. } => Some(mesh),
            _ => None,
        })
        .collect()
}

#[test]
fn connector_emits_one_length_independent_tapered_mesh_command() {
    let connector_node = |connector_to: (f32, f32), turn_x: f32| LayoutNode {
        x: connector_to.0,
        y: connector_to.1 - 22.0,
        w: 80.0,
        h: 44.0,
        node_idx: 1,
        source_node_index: 1,
        depth: 1,
        connector_from: (0.0, 0.0),
        connector_to,
        connector_turn_x: Some(turn_x),
    };
    let mut short = DrawList::new();
    let mut long = DrawList::new();
    let short_node = connector_node((120.0, 60.0), 60.0);
    let long_node = connector_node((1_200.0, 60.0), 600.0);
    let viewport = test_viewport(
        Rect::new(0.0, 0.0, 2_000.0, 200.0),
        Rect::new(0.0, 0.0, 2_000.0, 200.0),
    );

    draw_connector(&mut short, &short_node, [1.0; 4], CONNECTOR_REFERENCE_WIDTH_DP, viewport);
    draw_connector(&mut long, &long_node, [1.0; 4], CONNECTOR_REFERENCE_WIDTH_DP, viewport);

    let short_meshes = tapered_mesh_commands(&short);
    let long_meshes = tapered_mesh_commands(&long);
    assert_eq!(short_meshes.len(), 1);
    assert_eq!(long_meshes.len(), 1);
    assert_eq!(short_meshes[0].vertices.len(), long_meshes[0].vertices.len());
    assert!(!short.cmds.iter().any(|command| matches!(command, DrawCmd::FillRect { .. })));
}

#[test]
fn sibling_connectors_draw_independent_single_meshes() {
    // 保留既有测试中三个 LayoutNode、parent_joint、viewport 和 render_connectors() 调用。
    // 只用以下断言替换对 parent_joint FillRect 采样数的旧断言。
    let connector_meshes = tapered_mesh_commands(&draw_list);
    assert_eq!(connector_meshes.len(), layout.nodes.len());
    assert_eq!(connector_meshes.len(), 3);
    assert!(draw_list.cmds.iter().all(|command| {
        !matches!(command, DrawCmd::FillRect { color, .. }
            if *color == theme.mindmap.canvas.connector)
    }));
}

// 在既有 drag_preview_draws_valid_insertion_feedback_and_invalid_color_without_insertion
// 测试中，用以下断言替换 guide_samples 的 FillRect 圆点串检查：
let drag_guide_meshes = valid_draw_list
    .cmds
    .iter()
    .filter_map(|command| match command {
        DrawCmd::TaperedMesh { mesh, color, .. }
            if *color == theme.mindmap.canvas.connector_hover =>
        {
            Some(mesh)
        }
        _ => None,
    })
    .collect::<Vec<_>>();
assert_eq!(drag_guide_meshes.len(), 1, "drag guide must use one dynamic tapered mesh");
assert!(drag_guide_meshes[0].vertices.len() < 1_000);
```

- [ ] **Step 2: 运行 markdown 测试并确认 RED**

Run: `cargo test -p textora-markdown --lib -- connector_emits_one_length_independent_tapered_mesh_command`

Expected: FAIL，当前连接线仍生成多个 `FillRect`，没有 `TaperedMesh` 命令。

- [ ] **Step 3: 用单网格替换 `draw_tapered_sample()` 循环**

实现 `tapered_connector_mesh()`，将 `(f32, f32)` 中心线转换为 `Vec<[f32; 2]>` 后调用 UI tessellator：

```rust
fn tapered_connector_mesh(
    centerline: &[(f32, f32)],
    head_width: f32,
    tail_width: f32,
    scale: f32,
) -> Option<Arc<TaperedMesh>> {
    let centerline = centerline.iter().map(|&(x, y)| [x, y]).collect::<Vec<_>>();
    tessellate_tapered_path(TaperedPathInput {
        centerline: &centerline,
        head_width,
        tail_width,
        scale,
        feather_width: TAPERED_PATH_FEATHER_PX,
    })
    .map(Arc::new)
}
```

本阶段保持现有调用面：`draw_connector()` 先得到屏幕空间中心线，再以 `scale: 1.0` 构建网格并使用 `[0.0, 0.0]` 平移提交。随后删除：

- `CONNECTOR_SAMPLE_STEP_RATIO`
- `CONNECTOR_PREFERRED_MAX_SAMPLE_STEP`
- `draw_tapered_path()` 的按像素循环
- `draw_tapered_sample()`
- `lerp_point()`（若无其他调用）
- `tapered_path_renders_when_tail_width_exceeds_preferred_sample_step` 旧采样步长测试

拖拽引导线也调用 `tapered_connector_mesh()` 并提交一个 `TaperedMesh` 命令。

- [ ] **Step 4: 运行 markdown MMF 测试并确认 GREEN**

Run: `cargo fmt --check && cargo test -p textora-markdown --lib -- mmf`

Expected: PASS；中心线既有测试保持原断言，连接线测试只观察一个共享网格命令。

- [ ] **Step 5: 编译 app 并提交 mmap 单路径迁移**

Run: `cargo check -p textora-app`

Expected: PASS。

```bash
git add crates/markdown/src/mmf/canvas.rs
git commit -m "perf(markdown): render mmap connectors as tapered meshes"
```

---

### Task 4: 缓存静态 mmap 连接线几何并统一失效

**Files:**
- Modify: `crates/markdown/src/mmf/canvas.rs`
- Modify: `crates/markdown/src/mindmap_view.rs`
- Test: `crates/markdown/src/mindmap_view.rs`

**Interfaces:**
- Consumes: Task 4 的 `tapered_connector_mesh()` 与 Task 2 的共享命令。
- Produces:

```rust
pub(crate) struct ConnectorMeshCache {
    zoom_bits: u32,
    meshes_by_layout_node: Vec<Option<Arc<TaperedMesh>>>,
}

impl ConnectorMeshCache {
    pub(crate) fn build(
        layout: &LayoutTree,
        constants: &LayoutConstants,
        zoom: f32,
    ) -> Self;

    pub(crate) fn matches_zoom(&self, zoom: f32) -> bool;

    pub(crate) fn mesh_for_layout_node(
        &self,
        layout_node_index: usize,
    ) -> Option<&Arc<TaperedMesh>>;
}
```

- [ ] **Step 1: 写重复渲染、滚动复用和 zoom 失效失败测试**

在 `mindmap_view.rs` 现有画布测试 helper 附近加入：

```rust
// 将测试模块现有 `use std::borrow::Cow;` 改为：
use std::{borrow::Cow, sync::Arc};

fn first_tapered_mesh(draw_list: &DrawList) -> Arc<ui::tapered_path::TaperedMesh> {
    draw_list
        .cmds
        .iter()
        .find_map(|command| match command {
            ui::core::paint::DrawCmd::TaperedMesh { mesh, .. } => Some(Arc::clone(mesh)),
            _ => None,
        })
        .expect("mmap fixture must render at least one connector")
}

#[test]
fn mmap_static_connector_mesh_cache_survives_render_and_pan_but_not_zoom() {
    let source = "# Root\n## Child\n";
    let (mut view, doc) = view_with_source(source);
    let theme = Theme::from_definition(&ThemeDefinition::default_dark());
    let mut shaper = Shaper::new().expect("test shaper should initialize");
    let metrics = view
        .prepare_canvas(&doc, &theme, &mut shaper, 1.0)
        .expect("fixture must prepare canvas");
    let viewport = resolve_viewport(CanvasViewportInput::positioned(
        Rect::new(0.0, 0.0, 800.0, 600.0),
        metrics.content_bounds,
        CanvasViewPosition { zoom: 1.0, scroll: CanvasPoint::ZERO },
        CanvasViewportConfig::DEFAULT,
    ));

    let first = first_tapered_mesh(&view.render_canvas(&doc, &viewport, &theme, &mut shaper, 1.0));
    let second = first_tapered_mesh(&view.render_canvas(&doc, &viewport, &theme, &mut shaper, 1.0));
    assert!(Arc::ptr_eq(&first, &second));

    let mut panned = viewport;
    panned.scroll = CanvasPoint::new(20.0, 10.0);
    let after_pan = first_tapered_mesh(&view.render_canvas(&doc, &panned, &theme, &mut shaper, 1.0));
    assert!(Arc::ptr_eq(&first, &after_pan));

    let mut zoomed = viewport;
    zoomed.zoom = 2.0;
    let after_zoom = first_tapered_mesh(&view.render_canvas(&doc, &zoomed, &theme, &mut shaper, 1.0));
    assert!(!Arc::ptr_eq(&first, &after_zoom));
}

#[test]
fn mmap_layout_invalidation_drops_connector_mesh_cache() {
    let (mut view, doc) = view_with_source("# Root\n## Child\n");
    render_test_view(&mut view, &doc);

    let MindmapDocumentState::Ready { connector_mesh_cache, .. } = &view.document_state else {
        panic!("fixture must be ready");
    };
    assert!(connector_mesh_cache.is_some());

    view.clear_layout();

    let MindmapDocumentState::Ready { connector_mesh_cache, .. } = &view.document_state else {
        panic!("fixture must remain ready");
    };
    assert!(connector_mesh_cache.is_none());
}

#[test]
fn mmap_non_geometry_render_changes_keep_connector_mesh_cache() {
    let source = "# Root\n## Child\n";
    let (mut view, mut doc) = view_with_source(source);
    let first = first_tapered_mesh(&render_test_draw_list(&mut view, &doc));

    view.handle_message(PluginMessage::SetCursorVisible(false), &mut doc);
    view.handle_message(
        PluginMessage::SetCanvasPointer(Some(CanvasPoint::new(200.0, 120.0))),
        &mut doc,
    );
    let after_pointer = first_tapered_mesh(&render_test_draw_list(&mut view, &doc));
    assert!(Arc::ptr_eq(&first, &after_pointer));

    let mut recolored_theme = Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
    recolored_theme.mindmap.canvas.connector = [0.8, 0.2, 0.1, 0.3];
    let after_color = first_tapered_mesh(&render_test_draw_list_with_theme(
        &mut view,
        &doc,
        &recolored_theme,
    ));
    assert!(Arc::ptr_eq(&first, &after_color));
}
```

- [ ] **Step 2: 运行缓存测试并确认 RED**

Run: `cargo test -p textora-markdown --lib -- mmap_static_connector_mesh_cache`

Expected: FAIL，Ready 状态尚无 `connector_mesh_cache`，重复渲染得到不同 `Arc`。

- [ ] **Step 3: 在 canvas 层构建与提交缓存网格**

在 `canvas.rs` 实现 `ConnectorMeshCache`。构建缓存时使用内容空间中心线和 `scale: zoom`：

```rust
impl ConnectorMeshCache {
    pub(crate) fn build(
        layout: &LayoutTree,
        constants: &LayoutConstants,
        zoom: f32,
    ) -> Self {
        let meshes_by_layout_node = layout
            .nodes
            .iter()
            .map(|layout_node| {
                if layout_node.depth == 0 {
                    return None;
                }
                let turn_x = layout_node
                    .connector_turn_x
                    .expect("non-root mmap connector must have a turn axis");
                let centerline = connector_centerline(
                    layout_node.connector_from,
                    layout_node.connector_to,
                    turn_x,
                    constants.connector_width,
                );
                tapered_connector_mesh(
                    &centerline,
                    connector_head_width(layout_node.depth, constants.connector_width),
                    connector_tail_width(constants.connector_width),
                    zoom,
                )
            })
            .collect();
        Self { zoom_bits: zoom.to_bits(), meshes_by_layout_node }
    }

    pub(crate) fn matches_zoom(&self, zoom: f32) -> bool {
        self.zoom_bits == zoom.to_bits()
    }
}
```

静态渲染时网格已经包含 `content_coordinate * zoom`，本帧只使用：

```rust
let translation = viewport.content_to_screen(CanvasPoint::ZERO);
draw_list.tapered_mesh(Arc::clone(mesh), [translation.x, translation.y], color);
```

给 `render()`、`render_cards_and_connectors()` 和 `render_connectors()` 增加同名纯数据参数：

```rust
connector_mesh_cache: Option<&ConnectorMeshCache>
```

生产路径由 `MindmapView` 传入 `Some(cache)`。`render_connectors()` 使用 `enumerate()` 获得布局节点索引；命中缓存时提交上面的共享网格，`None` 只作为 canvas 单元测试和无 Ready 缓存时的安全回退，调用 Task 3 的即时单网格路径。继续使用 `connector_intersects_viewport()` 做布局空间粗裁剪。动态拖拽引导线不访问 `ConnectorMeshCache`。

- [ ] **Step 4: 在 MindmapView Ready 状态旁持有并统一失效缓存**

为 Ready 状态增加：

```rust
connector_mesh_cache: Option<mmf::canvas::ConnectorMeshCache>,
```

所有构造 Ready 状态的位置初始化为 `None`。`clear_layout()` 同时执行：

```rust
*layout = None;
*hit_map = None;
*connector_mesh_cache = None;
```

新增单一职责 helper：

```rust
fn ensure_connector_mesh_cache(&mut self, zoom: f32) {
    let MindmapDocumentState::Ready {
        layout: Some(layout),
        connector_mesh_cache,
        ..
    } = &mut self.document_state
    else {
        return;
    };
    if connector_mesh_cache.as_ref().is_some_and(|cache| cache.matches_zoom(zoom)) {
        return;
    }
    *connector_mesh_cache = Some(mmf::canvas::ConnectorMeshCache::build(
        layout,
        &self.constants,
        zoom,
    ));
}
```

`render_canvas()` 在建立 projection 和不可变 Ready 借用前调用该 helper，然后把缓存引用传给 `mmf::canvas::render()`。源码更新、预编辑、主题几何和 DPI 已统一经过 Ready 重建或 `clear_layout()`；不得再增加多个失效布尔字段。

- [ ] **Step 5: 运行 markdown 与 app mmap 测试并确认 GREEN**

Run: `cargo fmt --check && cargo test -p textora-markdown --lib -- mmap_ && cargo test -p textora-markdown --lib -- connector_ && cargo test -p textora-app --lib -- mmap_`

Expected: PASS；重复渲染和滚动复用同一 `Arc`，zoom 与布局失效生成新网格。

- [ ] **Step 6: 编译并提交缓存阶段**

Run: `cargo check -p textora-app`

Expected: PASS。

```bash
git add crates/markdown/src/mmf/canvas.rs crates/markdown/src/mindmap_view.rs
git commit -m "perf(markdown): cache mmap connector geometry"
```

---

### Task 5: 全面回归与性能验收

**Files:**
- Modify only if verification reveals a defect in Tasks 1–4; do not add unrelated cleanup.

**Interfaces:**
- Consumes: Tasks 1–4 的最终接口。
- Produces: 可交付、格式化、通过完整验证的实现。

- [ ] **Step 1: 运行分层目标测试**

Run:

```bash
cargo test -p textora-ui --lib
cargo test -p textora-app --lib -- paint_backend
cargo test -p textora-markdown --lib -- mmf
cargo test -p textora-app --lib -- mmap_
cargo check -p textora-app
```

Expected: 全部 PASS，无 warning。

- [ ] **Step 2: 运行重大修改验证脚本**

Run: `./scripts/verify.sh`

Expected: exit 0，所有格式、lint、测试和架构检查通过。

- [ ] **Step 3: 使用原复现文档执行 profiling/release 性能验收**

Run: `cargo run --profile profiling`

在同一 mmap、同一窗口尺寸和 zoom 下连续滚动，采集现有 `[drain]` 与 `[frame]` 日志。Expected：

- 主插件 `total_vertices` 不再接近 98 万，连接线相关顶点为数千级。
- 静态滚动帧不重新 tessellate 连接线；测试已证明 `Arc` 复用。
- 主 `drain` 进入 16.7ms 的 60Hz 帧预算；记录是否同时进入 8.3ms 的 120Hz 观察线。
- `cache_miss=0` 时不再出现 137–154ms 的连接线几何 drain。

- [ ] **Step 4: 检查工作区和最终差异**

Run:

```bash
git status --short
git diff --check 7ce36a88..HEAD
git diff --stat 7ce36a88..HEAD
```

Expected：只包含本实施计划文档和计划列出的实现文件；`docs/superpowers/plans/2026-07-21-mmap-branch-theme-styling.md` 保持原有未跟踪状态，不进入任何提交。

- [ ] **Step 5: 若验证阶段产生必要修正，单独提交**

只有 Step 1–3 暴露真实缺陷时才执行：先补失败测试、确认 RED、最小修复并重新运行 `./scripts/verify.sh`，然后：

```bash
git add crates/ui/src/tapered_path.rs crates/ui/src/core/paint.rs crates/ui/src/lib.rs crates/app/src/paint_backend.rs crates/markdown/src/mmf/canvas.rs crates/markdown/src/mindmap_view.rs
git commit -m "fix(render): preserve tapered connector regressions"
```

若没有必要修正，不创建空提交。
