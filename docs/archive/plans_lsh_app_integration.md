# 方案：App 接入 LSH 语法高亮渲染

## 现状

- ✅ `core::highlight` 模块已完成，提供 `Highlighter`, `HighlighterCache`, `HighlightKind`, `Language`
- ✅ `core::highlight::FILE_ASSOCIATIONS` 已生成，含文件扩展名→语言映射
- ✅ Theme 已有 `scopes: HashMap<String, [f32; 4]>` 和 `scope_color()` 方法（预留给语法高亮）
- ❌ App 渲染管线所有文字用单一 `theme.foreground` 颜色
- ❌ `DocumentView` 没有 language / highlighter_cache
- ❌ 编辑时没有 invalidate cache

---

## 实施计划（4 阶段）

### 阶段 1：DocumentView 接入语言检测 + 缓存

**文件：** `crates/app/src/document_view/mod.rs`

- 新增字段 `language: Option<&'static core::highlight::Language>`
- 新增字段 `highlighter_cache: core::highlight::HighlighterCache`
- `from_file()` 结尾：根据文件扩展名查询 `FILE_ASSOCIATIONS`，设置 `language`
- 新增辅助方法 `highlights_for_line()` 供渲染管线调用

### 阶段 2：Theme 填充 scope 颜色

**文件：** `crates/app/src/theme.rs`

- `dark()` 和 `light()` 中各添加 scope→color 映射
- 颜色映射：Comment=暗绿, String=亮红, KeywordControl=亮品红, KeywordOther=亮蓝, ConstantNumeric=亮绿, ConstantLanguage=亮蓝, Variable=亮青, Method=亮黄, MetaHeader/MarkupHeading=亮蓝

### 阶段 3：渲染管线接入高亮颜色

**文件：** `crates/app/src/render_pipeline.rs`

- `shape_visible_lines` 中：为每个 cluster 查找对应高亮 span 的颜色
- 改为按颜色分组生成顶点，替代硬编码 `theme.foreground`

**文件：** `crates/app/src/app.rs`

- 传递高亮数据流

### 阶段 4：编辑时 invalidate cache

**文件：** `crates/app/src/document_view/mod.rs`

- 在每个编辑操作后调用 `highlighter_cache.invalidate_from(line)`

---

## 不改的内容

- `crates/core/` 所有文件（阶段 1-4 已完成）
- `crates/lsh/`
- GPU / atlas / shaper 模块
- 状态栏、标签栏渲染

---

## 边界情况

1. **文件无扩展名或未知扩展名**：`language=None`，不高亮，不影响渲染
2. **文档为空**：cache 返回空 spans，fallback 到 foreground
3. **长行/软换行**：高亮按字节偏移，与视觉行无关
4. **大量编辑**：cache.invalidate_from 渐进式重建
5. **语言切换**：暂不支持手动切换，仅依赖文件扩展名
