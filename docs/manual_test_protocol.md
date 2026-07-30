# edit+ 手动测试协议

本文档对应 `plans.md` 各阶段的手动验收步骤。
每阶段包含：操作步骤、预期结果、边界 case。

## 当前 textora 运行命令

自 ER5 起，当前应用包名为 `textora-app`，使用以下命令启动：

```bash
cargo run -p textora-app
cargo run -p textora-app -- path/to/file.md
```

本文历史章节中的 `edit-plus-app` 命令保留作为历史记录，不代表当前包名。

---

## §3：winit + wgpu 空窗口

### 基本操作
1. `cargo run -p edit-plus-app`
   - **预期**：窗口弹出，灰色背景（#1a1a1f），无文字
2. 拖拽窗口边缘调整大小
   - **预期**：窗口平滑 resize，无闪烁、无撕裂
3. 最小化再恢复
   - **预期**：恢复正常显示
4. 关闭窗口（点 × 或 Cmd+Q）
   - **预期**：进程正常退出，无 panic、无僵尸进程

### 边界 case
- 外接显示器 + 不同 DPI —— 窗口不崩溃
- 全屏模式（macOS 绿色按钮）—— 进出正常
- 快速反复 resize 10 次 —— 无 crash

---

## §4：cosmic-text + 静态文本渲染

### 基本操作
1. `cargo run -p edit-plus-app`
   - **预期**：窗口显示 "Hello, edit+ — 世界 👨‍👩‍👧"
2. 切换主题色（硬编码切换 `app.rs` 里的 `CLEAR_COLOR`）
   - **预期**：文字无 alpha 黑边、无锯齿
3. macOS 缩放 1×/2×/3×
   - **预期**：文字像素清晰，无模糊

### 字号测试
4. 修改 `FONT_SIZE` 为 8 → 重启
   - **预期**：小字可读，无破碎字形
5. 修改 `FONT_SIZE` 为 72 → 重启
   - **预期**：大字清晰，无锯齿

### 边界 case
- CJK + emoji + Latin 混排 —— 都有字形
- 指定不存在的字体家族 —— 走系统默认而非 □
- 基础 Latin "ffi" 软连字 —— 不渲染错位
- emoji 变体选择器（U+FE0F）—— 正确生效

---

## §5：只读显示一个文件

### 基本操作

| # | 命令 | 预期 |
|---|---|---|
| 1 | `cargo run --release -p edit-plus-app -- assets/samples/medium_ascii_5mb.txt` | 即时首屏（< 1 s） |
| 2 | `cargo run --release -p edit-plus-app -- assets/samples/large_cjk_50mb.txt` | 滚轮顺滑到底 |
| 3 | `cargo run --release -p edit-plus-app -- assets/samples/long_line_1mb.txt` | 单行不卡，可水平滚或 word wrap |
| 4 | `cargo run --release -p edit-plus-app -- assets/samples/small_illegal_utf8.bin` | lossy 渲染不崩 |
| 5 | `cargo run --release -p edit-plus-app -- /dev/null` | 空窗口 |
| 6 | `cargo run --release -p edit-plus-app -- nonexistent.txt` | 友好错误提示，非 panic |

### 滚动测试
7. 打开 `medium_ascii_5mb.txt`，鼠标滚轮向下滚动
   - **预期**：内容平滑滚动，无白屏、无跳帧
8. 滚到文件末尾再滚回开头
   - **预期**：首行正确显示，无残留
9. 拖动滚动条直接跳到末尾，再跳回头部
   - **预期**：即时响应，无卡顿

### 边界 case

| 场景 | 样本文件 | 预期 |
|---|---|---|
| CRLF / LF / CR 混排 | `small_mixed_eol.txt` | 行数正确，无多余空行 |
| 末尾无换行 | `tiny_no_eol.txt` | 最后一行正常显示 |
| 末尾仅有换行 | `tiny_empty.txt` | 无多余空行 |
| 单行 1 MB | `long_line_1mb.txt` | 渲染不卡（高亮可拒绝） |
| 含 BOM | `small_bom.txt` | BOM 不显示为乱码 |
| 非法 UTF-8 | `small_illegal_utf8.bin` | lossy 替换，不崩溃 |
| NFC / NFD 组合字符 | `small_combining.txt` | 显示正确 |
| ZWJ + 变体选择器 | `small_emoji_zwj.txt` | emoji 正确渲染 |
| 空文件（0 字节） | `tiny_empty.txt` | 空窗口，不崩溃 |
| 1 字节文件 | `tiny_one_byte.txt` | 正常显示 |
| 文件名含空格/中文/emoji | `path_with_spaces 中文 🌏.txt` | 正常打开 |
| 符号链接 | `symlink_to_small.txt` | 正常读取 |
| 二进制含 `\0` | `binary_with_nulls.bin` | 友好拒绝或 lossy |
| 只读文件 | `readonly.txt` | 正常显示 |

### 性能验证（手动粗测）
10. 打开 `large_ascii_50mb.txt`，Activity Monitor 查看内存
    - **预期**：RSS < 150 MB
11. 打开空窗口，Activity Monitor 抽 30 秒平均 CPU
    - **预期**：idle CPU < 0.5%
12. 持续滚动 60 秒，肉眼观察
    - **预期**：无明显丢帧

---

## 后续阶段占位

- §6：键盘输入 + 编辑 —— ✅ 已实现（TextBuffer 代理、grapheme-aware 编辑、Undo/Redo、Word movement）
## §7：选择 + 剪贴板 + 撤销/重做

### 前置条件
- 已完成阶段 6（键盘输入 + 编辑）
- 使用 `assets/samples/small_ascii.txt`（4 KB Lorem ipsum）

### M7.1 鼠标拖选 + 状态栏
1. 打开 `small_ascii.txt`，在文本区域按住鼠标左键拖动
   - **预期**：
     - ✅ 选区出现蓝色半透明高亮矩形
     - ✅ 状态栏显示 `Selected: N chars, M bytes`
     - ✅ 松开鼠标后选区保持
2. 在无选区状态下观察状态栏
   - **预期**：✅ 显示 `Ln X, Col Y`

### M7.2 Shift+方向键扩选
3. 光标在某位置，按住 Shift + → 多次
   - **预期**：✅ 选区逐步向右扩展，高亮跟随
4. Shift + ← 缩小选区
   - **预期**：✅ 选区左端收缩
5. Shift + ↓ 扩选到下一行
   - **预期**：✅ 跨行选区，两行都有高亮

### M7.3 Cmd+A 全选
6. 按 Cmd+A
   - **预期**：✅ 全文选中，状态栏显示总字符数和字节数

### M7.4 双击选词 / 三击选行
7. 双击一个英文单词（如 "Lorem"）
   - **预期**：✅ 整个单词被选中
8. 三击某一行
   - **预期**：✅ 整行被选中
9. 双击连续 CJK 文本
   - **预期**：⚠️ 按 byte-class 分词，CJK 整段视作一词（非 ICU 词边界）

### M7.5 剪贴板跨进程
10. 在编辑器中选中文字 → Cmd+C
11. 切换到 Safari → Cmd+V
    - **预期**：✅ 粘贴内容与编辑器选区一致
12. 在 Safari 中复制文字 → 切回编辑器 → Cmd+V
    - **预期**：✅ 文字插入到光标位置

### M7.6 撤销 / 重做
13. 输入若干字符 → Cmd+Z 撤销
    - **预期**：✅ 逐次撤销输入
14. Shift+Cmd+Z 重做
    - **预期**：✅ 逐次恢复
15. 连续 50 次 Cmd+Z + 50 次 Shift+Cmd+Z
    - **预期**：✅ 不丢历史，不 panic

### M7.7 选中后打字替换
16. 选中一段文字 → 直接输入新字符
    - **预期**：✅ 选区被删除，新字符插入到选区起始位置

### M7.8 外部 RTF 剪贴板
17. 从 Pages/Word 复制带格式文字 → 编辑器 Cmd+V
    - **预期**：✅ 只取 plain text，无 RTF 标记

### 边界 case

| 场景 | 操作 | 预期 |
|---|---|---|
| 选区跨多行 | 拖选跨越 3 行 | 高亮正确，状态栏计数准确 |
| 选区跨非法 UTF-8 | 选含 0xFF 的区域 copy | 不 panic，clipboard 含 U+FFFD |
| 剪贴板含 BOM | 从含 BOM 文件复制粘贴 | BOM 被剥离 |
| undo 越过 load 点 | 撤销到打开文件之前 | 不 panic |
| 连续打字合并 | 快速输入 20 字符后 Cmd+Z | 一步撤销全部 |
| 空选区 copy | 光标无选区时 Cmd+C | 无操作，不崩溃 |
- §8：文件 IO 闭环 —— 待实现
- §9：多 buffer + Tab UI —— 待实现
- §10：搜索（SIMD）—— 待实现
- §11：替换 + ICU 正则 —— 待实现
- §12：性能基线 + 优化 —— 待实现

## §10 Sidebar 双模式（2026-06-11）

### M10.1 默认 sidebar 启动
命令：`cargo run --release -p edit-plus-app -- assets/samples/medium_ascii_5mb.txt`
预期：
- ✅ 启动后红绿灯位于 sidebar header（macOS）
- ✅ 默认未钉住，不显示 sidebar；编辑区贴近左边
- ❌ 不允许任何 panic 或 GPU 警告

### M10.2 hover 弹出
预期：
- ✅ 鼠标进入窗口左 4px 热区，停 ~150ms 后 sidebar overlay 出现
- ✅ 鼠标离开 sidebar 区 ~300ms 后消失
- ✅ 按 Esc 立即收起 overlay

### M10.3 钉住 / 取消钉住
预期：
- ✅ Cmd+B 切钉住，编辑区水平让位 sidebar 宽度
- ✅ 再按 Cmd+B 取消钉住，编辑区还原
- ✅ 钉住状态下重启 app 仍钉住

### M10.4 边缘拖拽改宽
预期：
- ✅ 钉住状态下，鼠标到右边缘 4px 内显示 ↔ 光标
- ✅ 拖拽改宽，最小 160 * dpi、最大 400 * dpi
- ✅ 松手后重启 app 宽度保持

### M10.5 设置菜单切模式
预期：
- ✅ 点 ⚙ 设置弹菜单：「Sidebar 模式 ✓ / Tabs 模式 / 打开 settings.yaml」
- ✅ 选 Tabs 模式：红绿灯回原生位、Tab 栏出现、settings.yaml 写入新值
- ✅ 选 Sidebar 模式：反向恢复

### M10.6 100+ tab 列表
预期：
- ✅ Cmd+T 连按 100 次后 sidebar 文件列表内部纵向滚动顺滑
- ✅ 点击列表项切换正确

### M10.7 极窄窗口
预期：
- ✅ 把窗口缩到 < (sidebar_width + 100) px，sidebar 自动收起 / 禁止钉住
- ✅ 还原宽度后可重新钉住

### M10.8 全屏 / Stage Manager
预期：
- ✅ 全屏切换不破坏布局
- ✅ Stage Manager 切换不破坏 traffic_light_inset

## Sidebar 类型化新建（2026-07-17）

- 点击“新建”主体，出现 `未命名.md`，使用 Markdown 编辑器。
- 点击右侧箭头，菜单依次显示“新建 TXT / 新建 MMAP / 新建 MD”。
- 三个菜单项分别创建 `未命名.txt`、`未命名.mmap.md`、`未命名.md`。
- 新建 MMAP 直接显示可编辑思维导图视图。
- 点击菜单外部或按 Escape 关闭菜单，不创建文档。
- 首次保存的默认文件名与当前未命名类型一致；取消后名称不变。

## Markdown WYSIWYG Cursor Convergence

Run with:

```bash
EDIT_PLUS_WYSIWYG_CURSOR_LOG=1 cargo run -p textora-app -- /tmp/wysiwyg-cursor.md
```

Cases:

1. Heading interior Enter: `# hello| world` inserts newline at heading line end, not old visual cursor byte.
2. Empty bullet Enter: `- |` deletes `- ` and cursor remains at byte 0.
3. Empty blockquote Enter: `> |` deletes `> ` and cursor remains at byte 0.
4. Table cell Enter: cursor moves to next cell and redraws immediately.
5. Emoji click: clicking inside `x👨‍👩‍👧y` uses snapped byte for cursor and selection anchor.
6. Drag selection after source update: stale plugin selection does not reappear after mouse release.

Expected logs:

- `[wysiwyg:augment]` reports the plugin augmentation.
- `[wysiwyg:cursor] ... after_sync` cursor equals the visible cursor target.
- `[wysiwyg:sync] pull_plugin_selection` appears only for intentional plugin-owned selection.

## MMAP 风格面板与文件级主题（2026-07-21）

### 前置条件

- 已准备一个不含 `theme` 字段的旧版 MMAP 文件，以及两个可独立保存的 MMAP 文件。
- 通过标题栏中位于源码视图按钮左侧的调色板按钮打开风格面板。

1. 打开一个不含 `theme` 的旧版 MMAP 文件。
   - **预期**：面板显示“素纸”；文档不进入 dirty 状态，源文件也不被自动写入主题字段。
2. 打开风格面板。
   - **预期**：面板固定在右侧，占用 280 个逻辑像素；缩略图为两列排列，且不会遮住思维导图节点。
3. 选择“潮汐”。
   - **预期**：画布立即切换配色；文件进入 dirty 状态；全局 TOML 写入 `theme = "tide"`。
4. 保存并重新打开该文件。
   - **预期**：仍选中“潮汐”，画布使用潮汐配色。
5. 同时打开两个 MMAP 文件，为它们选择不同主题后切换标签页。
   - **预期**：每个画布和面板中的选中项保持各自独立，不会串扰。
6. 在应用浅色和深色模式间切换。
   - **预期**：已选 MMAP 的节点和连线配色保持不变，面板本身的界面色彩随应用主题变化。
7. 在缩放或平移画布后反复打开、关闭风格面板。
   - **预期**：缩放比例不变；面板改变宽度前，位于视口中心的内容点仍保持在新视口中心。
8. 打开一个包含未知主题 ID 的 MMAP 文件。
   - **预期**：画布回退为默认主题并显示警告；源文件中的未知 ID 不会被改写。
9. 故意破坏全局 TOML。
   - **预期**：仍显示诊断画布，所有主题卡片被禁用；切换到源码视图修复 TOML 后，主题选择恢复可用并反映当前值。
10. 面板打开时切换到源码视图，再切回 MMAP 视图。
    - **预期**：仅当前标签页恢复之前的面板开关和展开状态；其他标签页的会话状态不受影响。
