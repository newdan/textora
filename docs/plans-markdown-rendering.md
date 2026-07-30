# Markdown 预览渲染 - 本地执行计划

> 源计划: docs/superpowers/plans/2026-06-16-markdown-rendering.md

## 阶段划分

### Phase 1: Crate 脚手架 ✅
- [ ] Task 1: 创建 crates/markdown/ 骨架（Cargo.toml + 空模块文件）
- [ ] 编译验证 `cargo check -p edit-plus-markdown`

### Phase 2: Parser + Style
- [ ] Task 2: parser.rs — pulldown_cmark 封装，事件流输出
- [ ] Task 3: style.rs — MarkdownStyle 纯数据配置
- [ ] 编译验证 + 单元测试

### Phase 3: Builder
- [ ] Task 4: builder.rs — MarkdownBuilder，事件→MarkdownDoc 树
- [ ] 编译验证 + 单元测试

### Phase 4: Layout + Render + API
- [ ] Task 5: layout.rs — MarkdownDoc → LaidOutDoc（计算位置/尺寸）
- [ ] Task 6: render.rs — LaidOutDoc → DrawList
- [ ] Task 7: lib.rs — 公开 API `render_markdown()`
- [ ] 编译验证 + 单元测试

### Phase 5: App 集成
- [ ] Task 8: md_preview.rs — MarkdownPreview 视图（缓存/滚动）
- [ ] Task 9: ui_shell + app_renderer + 快捷键集成
- [ ] 编译验证 `cargo build -p edit-plus-app`

## 已知修正
- Task 1 Step 2: lib.rs 改为只用 `pub mod`（原计划有重复声明）

## 状态
所有阶段已完成 ✅

## 执行总结

| 阶段 | 状态 | 测试 |
|------|------|------|
| Phase 1: Crate 脚手架 | ✅ | 编译通过 |
| Phase 2: Parser + Style | ✅ | 9 tests |
| Phase 3: Builder | ✅ | 9 tests |
| Phase 4: Layout + Render + API | ✅ | 9 tests (layout 4 + render 5) |
| Phase 5: App 集成 | ✅ | 666 app tests pass |

总测试: 27 (markdown) + 666 (app) = 693
