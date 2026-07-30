# mmap 光标与视口裁剪几何设计

## 目标

修复 mmap 视图中的两类几何不一致：标题光标须落在实际绘制文本的精确字形边缘；圆角节点被视口截断时，边界必须是裁剪产生的直边，不能把视口边界重新绘制成节点圆角。

## 根因

`mmf::layout::build_hit_map` 逐个 grapheme 调用 `Shaper::grapheme_advance`，而 `mmf::canvas::render_text` 对整个标题调用 `DrawList::text_shaped`。两次 shaping 的 cluster 与字距可能不同，因此 `grapheme_edges` 并不一定等于实际绘制位置。

`paint_backend::drain` 在处理带圆角的 `FillRect` 与 `StrokeRect` 时，先将原始矩形与 clip 相交，再对交集重新生成圆角顶点。被裁掉的一侧于是成为新的圆角边，而非真实几何被裁切后的直边。

## 方案

### 标题光标与命中

`grapheme_edges` 改为对完整标题调用一次 `Shaper::shape`，并对每个 Unicode grapheme 边界累加所有起点位于该边界之前的 shaped cluster advance。若 shaping 失败，保持当前逐 grapheme 测量作为回退。标题绘制、命中、选区、普通 caret 与 IME 候选窗因此共用一组整段 shaping 产生的 x 边缘。

### 通用圆角裁剪

无圆角矩形继续直接求矩形交集，以保持现有轻量路径。带圆角的填充和描边则先按原始矩形生成三角形；随后对生成的每个三角形执行轴对齐矩形裁剪并以扇形重新三角化。裁剪在 NDC 顶点空间完成，插值 `position`、`tex_coords` 和 `color`，因此同时保留抗锯齿羽化颜色。

该修复位于通用 paint 后端，所有使用 `PushClip` 的圆角组件都获得正确语义，不新增 mmap 专有绘制分支。

## 约束与边界

- 不改变 `ui` 与 `app` 的依赖方向；mmap 仍仅向 `ui::DrawList` 输出命令。
- 不改变未裁剪或直角矩形的顶点生成与外观。
- 所有新逻辑采用无 `unwrap()` 的错误处理；无法 shaping 时使用已有保守宽度回退。
- 只修改 `crates/markdown/src/mmf/layout.rs` 与 `crates/app/src/paint_backend.rs`，测试内联于其现有模块。

## 验收

- 对包含拉丁 kerning 的标题，mmap `grapheme_edges` 的末尾和中间边缘与整段 `Shaper::shape` 的 cluster 累计 advance 一致。
- 一张左侧越过 clip 的圆角矩形，在 clip 左边界具有完整高度的直切边；描边不会在该边界产生伪造的竖直边框。
- 现有 mmap 裁剪测试、markdown crate 测试、app paint 后端测试及相关编译均通过。
