# edit+ 编辑器效率启示（基于当前代码）

本文基于 `/Users/dan/proj/llmws/edit+` 现有实现，总结在「编辑器效率」上值得借鉴的做法，并给出后续可进一步强化的方向。重点从“打开文件”出发，延伸到“真实文档行 vs 虚拟可视行”和“超大文档/超长行”的处理。

## 1. 打开文件路径的效率做法（可借鉴）

### 1.1 流式加载 + 直写核心数据结构
`load_file` 会分块读取文件并直接写入 `GapBuffer`（`crates/core/src/file.rs#L76`），避免先拷贝到中间 `String/Vec<u8>` 再二次搬进 buffer。这个“一次落位”的思路对编辑器整体性能非常关键。

### 1.2 首块采样做高性价比检测
前 8KB 同时承担两项检测：
- 二进制文件拦截（首段出现 `` 就拒绝，`crates/core/src/file.rs#L83`）
- BOM 检测（`crates/core/src/file.rs#L89` 附近 `strip_bom`）

这类“小样本前置判断”很值得借鉴：成本低，却能避免后续大量无效计算。

### 1.3 行结束符检测与加载同趟完成
加载过程中同步统计 `LF / CRLF / CR`（`crates/core/src/file.rs#L133` 附近 `scan_line_endings`），最后一次性判定 `LineEnding`。避免再额外遍历一次文件内容。

## 2. 真实行 vs 虚拟行的分层设计（可借鉴）

### 2.1 真实行层：一次性建索引，后续 O(1) 访问
打开后会扫描一次 buffer 生成 `line_offsets/line_lengths`（`crates/app/src/document_view.rs#L1039` 附近 `rebuild_line_index_from_tb`），后续取任意一行只是数组下标访问，不重复遍历整个文档。

### 2.2 虚拟行层：只在可见窗口内展开 wrap
word wrap 不会预先展开整文档的虚拟行，而是在 `shape_visible_lines` 里按当前 viewport 宽度切分当前可见行的 clusters（`crates/app/src/app.rs#L745` 起）。这对“超长行 + 大文档”是关键：避免全量 wrap 带来的巨大 CPU 与内存压力。

### 2.3 行级 shaping 结果带缓存（按行字节区间哈希）
对每行用 `(offset, length)` 做 key 查 `shape_cache`，命中直接复用 shaping 结果（`crates/app/src/app.rs#L790` 附近 entry 逻辑）。这把最贵的 shaping 成本按行缓存，避免每帧重复。

## 3. 超大文档/超长行场景的现有策略（可借鉴）

### 3.1 滚动按“可视行粒度”控制
Viewport 维护了 `scroll_visual_offset`、`scroll_line_visual_count` 等字段（`crates/app/src/viewport.rs#L15`），支持在同一文档行内逐步滑动虚拟行，再跨到下一行；而不是粗暴地按文档行跳跃。

### 3.2 光标上下移动按虚拟行走，并用 advance cache 做精确定位
`move_cursor_visual`（`crates/app/src/app.rs#L343`）会用当前帧的 `advance_cache`（cluster 级别的 x 位移）找目标可视行上最接近 `sticky_x` 的字节偏移，避免反复重算整行布局。

### 3.3 单字符操作按“词类”局部扫描
`word_select` 从点击偏移向前后按字符类别推进，遇到类别变化/换行就停（`crates/core/src/buffer/navigation.rs#L202`），复杂度更贴近单词长度而不是整行长度。

## 4. 进一步强化建议（基于现有设计）

### 4.1 将 `advance_cache` 从“每帧重建”改为“增量更新”
当前 `shape_visible_lines` 开头会 `clear` 再逐行重建 `advance_cache`（`crates/app/src/app.rs#L754` 起），在“超长行 + 高帧率”下会成为热点。可以考虑：
- 以 doc_line 为粒度做脏标记，只重建变化行；
- 或以 `(offset, length)` 做 key 做短生命周期缓存，避免重复构建 cluster 级别数组。

### 4.2 对“超长行”增加 wrap 结果限流/分段缓存
`shape_visible_lines` 中会为首尾可见行缓存 visual_lines 并 `clone()`（`crates/app/src/app.rs#L835` 附近、后面 first/last line cache 赋值处）。极端长行下这会带来额外分配压力。可以：
- 对单行 wrap 结果做上限裁剪（比如最多缓存 N 段，超出按需重算）；
- 或改为引用计数/共享存储，减少 clone 开销。

### 4.3 编辑后行索引重建策略优化
`rebuild_line_index_from_tb` 在每次编辑后会重建行索引（`crates/app/src/document_view.rs#L1039`）。对超大文档，这里可能成为瓶颈。可考虑：
- 在 buffer 层维护行起始集合（增量插入/删除时更新）；
- 或仅对受影响区间做局部重建（需要 buffer 支持区间行偏移变更报告）。

## 5. 小结（可复用的工程模式）

- **一次落位**：加载路径尽量直接写入编辑器核心结构，避免中间拷贝。
- **前置小样本检测**：用极低代价拦截“不该走主路径”的输入。
- **同趟多目标**：在一次遍历里完成多个目标（读内容 + 检测换行类型）。
- **窗口优先**：只在可见区域做昂贵计算（wrap、cursor 定位等）。
- **按行缓存最贵步骤**：shaping 结果按行缓存，避免重复计算。
- **精细滚动单位**：用“可视行”而非“文档行”作为滚动与定位单位，适配长行场景。

---
参考文件（可直接跳转）：
- `crates/core/src/file.rs#L76`
- `crates/app/src/document_view.rs#L1039`
- `crates/app/src/viewport.rs#L15`
- `crates/app/src/app.rs#L343`
- `crates/app/src/app.rs#L745`
- `crates/core/src/buffer/navigation.rs#L202`


## 6. GapBuffer allocate_gap / commit_gap 的实现要点（“伪零拷贝”边界）

代码见 `crates/core/src/buffer/gap_buffer.rs#L105` 起。

- `allocate_gap(off, len, delete)` 会先移动 gap 到 `off`，再按需删除文本，最后扩展 gap；返回的是 **gap 区域可写切片**，长度为当前 `gap_len`（可能大于请求数）。
- 如果请求的 `len` 大于当前 `gap_len`，会走 `enlarge_gap`（`crates/core/src/buffer/gap_buffer.rs#L170`）。
- 对大 buffer（默认非 small）会先 `virtual_reserve` 一块很大的虚拟地址空间（`crates/core/src/buffer/gap_buffer.rs#L74`），再按需 `virtual_commit`（`crates/core/src/buffer/gap_buffer.rs#L197`）。这是“按需提交物理页”的思路，避免频繁 realloc。
- **边界/风险点**：`enlarge_gap` 在 `bytes_new > reserve` 时会直接 `return`（不扩），导致后续 `commit_gap` 可能只写入部分数据（取决于调用方是否处理返回切片长度）。这是“伪零拷贝”的关键：尽量避免搬移，但极端情况下会通过返回更短可写区域/失败路径来兜底。
- 小文件场景会命中 `Vec` 分支（`BackingBuffer::Vec`），走更普通的堆分配策略，适合碎片化/小容量场景。

## 7. 从 CLI 参数到 load_file 的调用链（启动开销在哪）

- 入口：`crates/app/src/main.rs#L3` 收集 `std::env::args()`，调用 `parse_args`。
- 参数解析：`crates/app/src/cli.rs#L20` 做极简遍历，产出 `CliArgs{file, headless}`，开销可忽略。
- App 构造：`App::new(cli.file)`（`crates/app/src/app.rs#L124` 附近）只保存 `file_path`，不在此处做重 IO。
- 窗口初始化后加载：`init_window` 会调 `self.init_text()?; self.load_file();`（`crates/app/src/app.rs#L172`）。
- `load_file`：`crates/app/src/app.rs#L261` 会取 `file_path`，算 `visible_rows`，再调用 `DocumentView::from_file`。
- `DocumentView::from_file`：`crates/app/src/document_view.rs#L69` 调用 `file::load_file(path)`，随后重建行索引（`rebuild_line_index_from_tb`）。

结论：启动链路本身很轻，真正的开销主要在“文件加载 + 行索引重建”，而不是 CLI/初始化。

## 8. advance_cache 的构建粒度与失效来源（为什么现在是每帧重建）

- 构建位置：`shape_visible_lines`（`crates/app/src/app.rs#L745` 起）会在渲染/形状阶段为每条可见可视行构建 `(doc_line_idx, Vec<(cluster_end, x)>)`，并 `push` 到 `advance_cache`（`crates/app/src/app.rs#L918` 附近）。
- 失效来源：任何会导致可视行集合或 cluster 布局变化的事件都会让旧 cache 失效，包括：resize、word wrap 开关、字体/字号/行高变化、文档内容变化、滚动导致可见行集合变化。
- 当前策略：每帧开头 `advance_cache.clear()` 后重建，简单且正确，但对“超长行 + 高帧率 + 高频输入”会带来额外 CPU/内存带宽消耗。

## 9. advance_cache 改成增量更新的可行方案（建议）

建议分两步做：
1) 行级脏标记：为每条 doc_line 维护 `dirty` 标（内容变化、设置变化时置脏），只重建脏行的 cluster advance。
2) 可视窗口索引：保持一个 `visible_line_keys: Vec<(offset,length), cache_index>` 映射，帧间比较 keys，未变化的行直接复用上一帧 advance 数据。

这样可以把“每帧 O(visible_lines * clusters)”降到“每帧 O(changed_lines * clusters)”。

## 10. 超长行极端 case 的压力测试点（建议）

- **1 行 10MB ASCII**：验证 wrap 计算与 advance_cache 不会导致帧时间暴涨。
- **1 行 1M 字符（含 CJK/变宽字体）**：验证 shaping + wrap 的 cluster 切分正确性与性能。
- **单行极大 + 频繁输入**：在长行中连续打字/删除，观察帧率与卡顿。
- **单行极大 + 频繁滚动**：鼠标滚轮/触控板快速滚动，验证 `scroll_visual_step_*` 稳定性。
- **resize 高频抖动**：快速拖拽窗口边缘，验证 `scroll_line_visual_count` 与 `scroll_visual_offset` 的 clamp 行为。
- **超大文档 + 超长行混合**：例如 100k 行文档，其中若干行为 1MB 级别，验证整体流畅度。
