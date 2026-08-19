# Markdown 编辑性能整改实施记录

## 范围

本轮只调整 `MarkdownEditorView` 使用的 Markdown WYSIWYG 编辑链路，不改变纯源码编辑器的
`RenderCache` 路径，也不引入增量 Markdown parser。

## 已实施设计

### 长段落投影

- `ProjectedText` 提供带视觉 grapheme byte 索引的切片入口。
- 同一投影的折行循环只构造一次 grapheme 边界，所有 visual byte → grapheme 查询使用二分。
- 无 shaper 的估算折行也直接复用 grapheme 边界，避免逐折行从段首扫描。
- 便利切片入口仍保留，单次切片调用点无需持有缓存状态。

### 编辑增强解析短路

- 普通字符输入仅在当前源码行不含非空白字符时才进入完整 Markdown 上下文分类。
- 普通退格仅在光标位于源码行内容末尾时才进入 `TopLevelParagraphEnd` 分类。
- Enter 路径保持完整分类，以保留列表、代码块、引用等语义。

### LazyLayout 跨编辑复用

- 以块 kind 与块源码文本作为身份，计算顶层块公共前缀、变更区间和公共后缀。
- 未受影响的非精确布局块直接转移；后缀块的源码 anchors、source ranges、owner 与几何按新位置平移。
- `flat_lines` 与 retained projections 随复用块转移，变更块仍由现有布局逻辑重建；最终
  `SourceProjectionIndex` 从新旧合成后的权威 projections 发布。
- 旧光标块、新光标块、选区相交块、精确 shaping 块以及包含代码块的树不复用，避免编辑标记、
  preedit、选区和 ASCII diagram sidecar 泄漏旧状态。
- 样式或视口宽度变化始终走全量路径。
- 设置 `TEXTORA_DISABLE_INCREMENTAL_MARKDOWN_LAYOUT_REUSE` 可关闭跨编辑复用，便于线上问题二分。

## 正确性防护

- Unicode 切片等价性覆盖 NFD、ZWJ emoji、变体选择符、CJK 和 collapsed spans 的所有合法字符边界。
- 解析次数测试断言普通插入与行中退格不会启动 pulldown-cmark 分类解析，同时验证特殊候选仍放行。
- 编辑序列等价性覆盖段落输入、跨块插入、整块删除、列表边界、代码块修改、CJK 与 emoji；逐行比较
  几何、文本、projection，并逐源码字符边界比较 `SourceProjectionIndex` 上下游映射。

## 基准

正式 Criterion 基准位于 `crates/markdown/benches/editing_perf.rs`。

2026-08-19 release quick run：

| 长 CJK 段落 | layout |
|---|---:|
| 10 KB | 0.044 ms |
| 40 KB | 0.175 ms |
| 82 KB | 0.349 ms |
| 163 KB | 0.717 ms |
| 326 KB | 1.403 ms |

326 KB 的原始实测基线为 414 ms；整改后曲线近似线性。

混合结构文档的 parse + build + layout 完整管线 quick run：

| 文档大小 | 单键管线 |
|---|---:|
| 5 KB | 0.244 ms |
| 22 KB | 1.081 ms |
| 87 KB | 4.276 ms |
| 218 KB | 11.162 ms |

parser 与 `MarkdownDoc::build` 仍是全量操作，块级增量解析明确留待独立工程处理。
