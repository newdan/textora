一、结构性问题

  1. WrapIndex 不是事实来源，是"被动追加"的脏缓存（最关键）

  WrapIndex::new(line_count) 把所有行的 visual_line_count 初始化为 1。真正准确的值只有在 shape_visible_lines 当帧渲染过的行才会
  update（app.rs:1036）。也就是说：

  - 还没被滚动到的行，count=1（可能是 stale 的低估）
  - 一旦滚过去再回来，count 才真实
  - 文件刚加载时 wrap_index.total_display_rows() == total_lines

  直接后果：
  - clamp_scroll_top 上界不准 → 滚轮可以滚过末尾，需要之后帧"反弹"
  - display_to_doc 在未访问区域返回的 doc_line 是错的，scroll_to_doc_line_wrap 跳转目标行偏移
  - total_visual_lines 用 visible_total + remaining（app.rs:1259-1264）做估算，每滚一次都在变，scrollbar 永远飘
  - 用户从顶部滚到底部时，sum 会单调增大 → scroll_top（绝对 DisplayRow）的语义在中途变了 → 视图"漂移"
  
  viewport_architecture_analysis.md §1 说"WrapIndex 是 O(log n) 的精确映射"，但这只在所有行都被 wrap 过之后才成立。这是当前最重要的设计裂缝。

  根治方向：要么文件加载完做一次后台预热（per-line wrap，O(N) 但只做一次 + 增量更新），要么显式区分 "exact" / "estimated"
  状态，所有调用方明确处理估算路径，避免在不知情情况下用估算值做精确判断。

  2. shape_visible_lines 单函数 ~390 行，6 项职责耦合

  app.rs:906-1299 一个函数同时干：shape + word wrap + WrapIndex 更新 + advance_cache 构建 + cursor_pixel_x 计算 + autoscroll + first/last line
  缓存。任意一项 bug 都需要改这个函数；可读性、可测试性都被压坏。

  3. visible_doc_line_range_wrap 在循环中被反复调用，且 wrap_index 在循环里就被改写

  app.rs:916 拿 range_pre，循环里 1036 行 wrap_index.update(...)，1053 行又调 visible_doc_line_range_wrap 拿 range。如果当前 doc line 实际 wrap 数 >
   1，第二次调用返回的 range.end 就和 range_pre.end 不一致，但循环上界 vis_count（第 933 行）和 range_pre.start + i 仍按 range_pre 走。1114 行 let 
  doc_line_idx = range.start + i; 用了第二次 range — 一旦 range.start 因 wrap 改变（理论上不会，因为 update 只影响 doc
  自己往后；但语义上是脆弱的），doc_line_idx 就错。这种"循环中改索引、循环中再读索引"的模式应该被消除。

  4. Viewport 维护两份派生缓存（first_visible_doc_line + cached_visible_range），多处写入

  - Viewport::sync_doc_line_from_scroll_top（viewport.rs:193）写一个近似值
  - Viewport::resize 也调用它（viewport.rs:141）
  - App::shape_visible_lines 第 924-925 行又用 WrapIndex 精确值覆盖
  - visible_doc_line_range（viewport.rs:166）若 cached_visible_range 为空就 fallback 用第一个值

  效果：一帧之内同一个状态被三处不同精度的来源覆盖，bug 出现时极难定位（已有 docs/viewport_architecture_analysis.md 指出过同类问题）。根治方向：让
  first_visible_doc_line 完全不存字段，每次访问都通过 wrap_index.display_to_doc(scroll_top.floor()) 实时算（O(log n) 可承受）。cached_visible_range
  同理。
  
  5. advance_cache 的 display_row 是绝对值，但用法是相对索引

  hit_test（app.rs:434-438）：

  let vis_line = (adjusted_py / line_height) as usize;
  let entry = &self.advance_cache[vis_line];   // 数组索引 = 相对屏幕位置
  let display_row = entry.display_row;          // 字段 = 绝对 DisplayRow

  数组下标和字段语义不一致 — display_row 字段在大多数路径里其实没人读（hit_test 返回它，但只用做调试）。要么删字段（保持纯粹相对），要么把
  advance_cache 改成 BTreeMap<DisplayRow, Entry> 让字段成为索引键。当前是"两套都有，随便用一套"。

  6. WrapIndex 不感知 wrap width；窗口 resize 后所有行的 count 仍是旧值

  Viewport::resize 只重置 total_visual_lines = None（viewport.rs:139），WrapIndex 完全不知情。下一帧只更新可见行的 count，其余所有行的 count 仍是
  resize 前的值。导致：

  - resize 后立即 scroll_to_doc_line_wrap(some_line) 的目标 DisplayRow 是错的（用了旧的 prefix sum）
  - total_display_rows() 是错的，scrollbar 飘

  应该在 resize 时把所有可见行外的 count 标记为 dirty，或者增加一个 generation 版本号。

  7. WrapIndex::shift_lines 是 O(n) — 编辑插入/删除每行都全量 shift

  wrap_index.rs:194-216：

  for i in ((edit_line + 1)..self.len).rev() { ... self.tree[self.n + i] = src; ... }
  for i in (1..self.n).rev() { self.tree[i] = ...; }   // 全量重建内部节点

  18000 行的文件，每次 Enter / Backspace 跨行都做 ~36000 次内存写。CLAUDE.md 提到"提交要确保编译过"，这倒能编译，但延迟不容小觑。

  要么用 SegTree with lazy / explicit 数组的 splice（用 Vec::insert / Vec::remove 维护 leaf 数组，再 rebuild），要么干脆改用 piece-table-like /
  SumTree 实现。

  8. DocumentView::sync_after_edit_full / sync_after_edit_incremental 与 Viewport::new 强耦合

  document_view.rs:669 / 744 / 825 都做：

  self.viewport = Viewport::new(self.viewport.visible_rows, total);

  这会把 scroll_top、first_visible_doc_line、cached_visible_range 全清零。结果：multi-line edit、selection delete、undo/redo 都会让视图被重置到顶部
  — 用户体验很差。Viewport::set_total_lines 已经做了同样的事（viewport.rs:128-134），但被反复调用 + 字段直接覆盖，丢失了用户的滚动位置。

  应该改为只更新 total_lines，scroll_top 自行 clamp，不重置滚动位置。

  9. DisplayRow 的 Add<u32> / Sub<u32> 不饱和，AddAssign 饱和 — 不一致

  viewport.rs:71-89：+ 注释说"Panics in debug; wraps in release"；+= 是
  saturating。同一类型的两个运算符语义不同，在并发或重构时容易踩坑。建议要么统一 saturating（保持 checked_* 显式），要么删掉 Add/Sub，强制调用方用
  saturating_add。
  
  10. selection_vertices 每帧重建一个稀疏 Vec<usize> 大数组

  app.rs:678-687：用 max_doc_line + 1 大小开 Vec、HashSet 去重 doc_line，再 sparse 填。如果 last_doc_line = 100000，分配 800KB
  的临时数组就为了填几十个非零项。应该把 compute_selection_highlight_quads 改签名为 &dyn Fn(usize) -> usize 或直接传 &DocumentView，按需查
  line_byte_offset，去掉这个 O(max_doc_line) 临时分配。
  
  11. cursor_visual_line 用 usize::MAX 哨兵

  App 多处 if self.cursor_visual_line != usize::MAX（app.rs:704, 937, 955, 972, 1085, 1100, 1250），可读性差且跟"未初始化 / 在 skip 区 / cursor
  离开视口"几种语义混着。改成 Option<DisplayRow> + enum CursorVisibility { Hidden, Visible(DisplayRow) } 更清晰。

  12. move_cursor_visual 4a/4b/4c 用了五份 first_/last_line_* 缓存

  app.rs:98-108
  这五个字段（first_line_visual_lines、first_line_clusters、last_line_visual_lines、last_line_clusters、first_line_doc_offset）就是为了 4b/4c 的
  sticky_x 跨视口移动。每帧 shape 都拷贝（app.rs:1043, 1048）。属于很重的 hack。如果有 WrapIndex + 一份 per-doc-line 的 wrap 缓存，4b/4c
  可以直接重新 shape 那一行，不需要预存。
                                                                 
  二、性能问题

  P1. Word-wrap 每帧重算（已知）

  shape_visible_lines:1015-1032 每帧对所有可见行做 wrap。char_width 也每帧扫一次（1001-1007）。

  优化：
  - per-doc-line wrap 缓存：key = (line_offset, line_length, viewport_width)，命中即返回 Vec<(start, end, width)>。和 shape_cache 类似 LRU 即可。
  - char_width 在等宽字体下是常量，全局缓存一次。
  
  P2. visible_line / visible_lines 每行至少一次 heap 分配

  document_view.rs:141-184。visible_line 返回 Vec<u8>（拷贝），visible_lines 又把每行 from_utf8_lossy().to_string()（再次拷贝）。

  shape_visible_lines 第 978 行 dv.visible_line(i) 拷贝；之后 1018 / 1096 / 1163 三处又遍历同一份 line_bytes 做 whitespace 判断 — 重复扫描。

  优化：
  - 把 visible_line 改为返回 Cow<'_, [u8]>，TextBuffer 单 chunk 时返回引用，跨 chunk 时才拷贝。
  - 在 shape 阶段一次性算出 cluster 的 is_whitespace bitmap，下游直接读。
  
  P3. advance_cache / doc_line_map 每帧 alloc

  app.rs:928 advance_cache.clear() + 每个 entry 内 Vec<(usize, f32)> 重分配；doc_line_map: BTreeMap 每帧重建。

  优化：
  - advance_cache 用对象池：clear() 保留 capacity（已有），但内部 clusters: Vec 也需要保留 — 改用复用策略（pre-allocate slab）。
  - doc_line_map 用 Vec<(doc_line, cache_idx, vl_count)> 排序后二分，clear() 不释放底层。或者干脆删除 — 它只在 move_cursor_visual 4c (app.rs:579)
  一处被读，可以改成线性扫 advance_cache 找匹配 doc_line。
  
  P4. shape_cache key 32+32 拼接，跨文档碰撞 + 编辑后整片失效

  app.rs:983 let cache_key = (offset as u64) << 32 | (length as u64);

  - 切到第二个 buffer，相同 offset/length 直接命中 → 渲染错乱（注释里的 TODO）
  - 在文件中间插入一行后，所有后续行的 offset 都变了 → cache 全 miss → 一次大编辑下视口立刻被重 shape，卡顿

  优化：用 xxhash(line_content) 当 key（与 offset 解耦），编辑后只重 shape 实际变化的行；或者用 (doc_id, line_content_hash)。

  P5. 状态栏 extract_selected_text 每帧拷贝（已部分缓存）

  app.rs:746-774 已有 selection_counts_cache，但 cache miss 时仍要把整个选区拷贝一份再算 char count。1MB 选区每次选区变更都拷贝。可以在
  extract_selected_text 时同时返回 char_count，或单独提供 count_selection_chars(start, end) 走 chunked 扫描，不构造 Vec。

  P6. rebuild_line_index_from_tb 每次 multi-line edit / undo / redo 全量扫描

  document_view.rs:1092-1163 全文档线性扫。一次 undo 一个跨 1000 行的删除 → 重新扫整个文件。sync_after_edit_full 在 undo/redo/select-delete
  都走这条路。

  优化：
  - TextBuffer 内部应已知 line breaks（cosmic-text/edit 的 buffer 通常自带 line index）；如果没有，就维护增量 line_offsets：删除时根据 selection
  range 内的换行数 splice，插入时同理。
  - 至少 selection delete 不必走 full rebuild — 删除只影响 [start, ...]，可以从 start 所在行开始 rescan_lines_from（已有这个函数）。
  
  P7. cursor_line 在 hot path 反复二分

  每次按键都至少调用一次 cursor_line()（O(log N)）。shape 循环中也多次（1052 / 1054 / 935）。可以把 cursor_line 缓存成字段，cursor_offset
  改变时才失效。

  P8. WrapIndex 全文件加载就 2 * next_pow2(n) 大小

  18000 行 → tree size 65536 × 8 bytes = 512KB。1M 行 → 16MB。一般够用，但对很大文件可以考虑 sparse / chunked。

  P9. autoscroll 在 shape 末尾，导致最少 1 帧延迟

  app.rs:1268-1295 cursor 移动 → scroll_to_row → set needs_redraw = true → 下一帧才生效。也就是按方向键时视图至少滞后 16ms。Zed 的做法是 pre-layout
  autoscroll（先算 scroll，再 layout）。把 autoscroll 提到 shape 之前，本帧就能渲染正确位置。

