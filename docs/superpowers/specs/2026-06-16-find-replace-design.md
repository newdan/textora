# Find & Replace 设计规格

## 概述

在现有 Find bar 基础上增加 Replace 功能。Inline 同行布局, 按钮 + 快捷键触发, 支持转义序列和正则表达式。

## 决策记录

| 决策 | 选择 |
|------|------|
| 布局 | Inline, find 和 replace 同行动态展开 |
| 触发 | 展开按钮 + Cmd+Shift+F / Cmd+Opt+F |
| 单次替换 | 替换当前匹配, 自动跳到下一个 |
| 全部替换 | 弹出确认对话框后执行 |
| 按钮文字 | 中文: 替换 / 全部 |
| 转义 | `\n` `\t` `\r` `\\` 始终生效 |
| 正则 | `.*` 按钮切换, 开启后使用 ICU regex, 替换支持 `$1` 引用 |
| 正则高亮 | 跟随主题 accent/selection 色 |
| 按钮提示 | 全部按钮有 tooltip |

---

## UI 规格

### Find-only 模式 (当前, 不变)

```
[/] [________________find input________________] [.*] [2/15] [◀][▶] [▼] [✕]
```

右侧按钮 (左到右):
- `.*` — 正则开关 (关闭状态)
- `2/15` — 匹配计数
- `◀` `▶` — 上一/下一匹配
- `▼` — 展开 Replace ("展开替换" tooltip)
- `✕` — 关闭

### Find+Replace 模式

```
[/] [___find___] [→] [___replace___] [替换] [全部] [2/15] [◀][▶] [▲] [✕]
```

- `→` — 分隔符, 不可交互
- `替换` — "替换当前并跳到下一个" tooltip
- `全部` — "替换全部 (需确认)" tooltip
- `▲` — 收起 Replace

### 正则模式

正则开启时:
- `.*` 按钮高亮 (匹配计数器左移到此位置)
- Find 输入框边框变主题 accent 色
- Replace 输入占位文字变为 "$1, $2, ..."

Find-only 正则:
```
[/] [________________find input________________] [2/15] [◀][▶] [.*] [▼] [✕]
                                                          ^^^ 高亮
```

Find+Replace 正则:
```
[/] [___find___] [→] [___$1, $2...___] [替换] [全部] [2/15] [◀][▶] [.*] [▲] [✕]
                                                                        ^^^ 高亮
```

### Tooltip

鼠标悬停 400ms 后显示 tooltip (macOS 标准延迟):
- `▼`: "展开替换"
- `.*` (关闭): "正则表达式"
- `.*` (开启): "关闭正则"
- `替换`: "替换当前并跳到下一个"
- `全部`: "替换全部 (需确认)"

### 键盘

- `Tab`: find ↔ replace 焦点切换
- `Enter` 在 find 框: 跳到下一匹配
- `Enter` 在 replace 框: 执行替换 + 跳到下一匹配
- `Esc`: 有内容先清空, 无内容关闭

---

## 数据模型

### SearchState 新增字段

```rust
pub struct SearchState {
    // ... 现有字段不变 ...
    pub replace_query: String,       // 替换文本
    pub replace_mode: bool,          // 是否展开 replace 区域 (UI 状态)
    pub focus_replace: bool,         // Tab 焦点: false=find, true=replace
}
```

### SearchBarSnapshot 新增字段

```rust
pub struct SearchBarSnapshot {
    // ... 现有字段不变 ...
    pub replace_query: String,
    pub replace_mode: bool,
    pub focus_replace: bool,
}
```

### SearchBarAction 新增变体

```rust
pub enum SearchBarAction {
    // ... 现有变体不变 ...
    ToggleReplace,    // 展开/收起 replace 模式
    ToggleRegex,      // 切换正则开关
    Replace,          // 替换当前匹配 + 跳到下一匹配
    ReplaceAll,       // 全部替换 (触发确认对话框)
    FocusFind,        // Tab 到 find
    FocusReplace,     // Tab 到 replace
}
```

### SearchOptions 现有字段已足够

```rust
pub struct SearchOptions {
    pub match_case: bool,   // 现有
    pub whole_word: bool,   // 现有
    pub use_regex: bool,    // 现有, 由 ToggleRegex 切换
}
```

---

## 行为流程

### 单次替换

```
1. 用户按 Enter (replace 框内) 或点 "替换" 按钮
2. 取当前匹配的 byte range
3. 若 replace_query 包含转义, 先解析 (\n→换行等)
4. 在 gap_buffer 中: delete_range + insert(replace_bytes)
5. 标记文档 dirty
6. 重新运行搜索
7. 定位光标到替换位置之后的第一个匹配
8. 若没有更多匹配: 显示 "0/0"
```

### 全部替换

```
1. 用户点 "全部" 按钮
2. 弹出 macOS 原生 NSAlert:
   标题: "Replace All"
   正文: "Replace N occurrences of \"pattern\" with \"replacement\"?"
   按钮: [Replace All] [Cancel]
3. 若 Cancel → 不做任何事
4. 若 Replace All:
   a. 遍历所有当前 matches (倒序, 避免偏移问题)
   b. 逐个 delete_range + insert
   c. 标记 dirty
   d. 记录替换计数 N
   e. 显示状态: "N replacements made" (在 match counter 位置短暂显示)
   f. 重新搜索 (此时应该 0 matches)
```

### 转义处理

查找和替换字段都处理:
- `\n` → 换行 (0x0A)
- `\t` → 制表符 (0x09)  
- `\r` → 回车 (0x0D)
- `\\` → 反斜杠 (0x5C)

在 query 传给搜索引擎前解析。replace 在应用前解析。

### 正则模式

- 开启时: 搜索使用 ICU regex (`core::buffer::search` 现成), 替代 SIMD 搜索
- 替换支持 `$1`..`$N` 分组引用 (core 已有 `RegexReplacement::Group`)
- 关闭时: 恢复 SIMD literal + 转义搜索

---

## 确认对话框

使用 rfd (已有依赖) 或 macOS NSAlert:

```rust
// 伪代码
let msg = format!("Replace all {} occurrences of \"{}\" with \"{}\"?",
    count, query, replace);
let confirmed = native_dialog::confirm("Replace All", &msg);
if confirmed { /* do replace all */ }
```

备选: 若 rfd 不支持 macOS 原生对话框, 用 in-bar 确认 (在 search bar 上覆写确认文字 + OK/Cancel 按钮)。

---

## Core 层变更

### SIMD 替换 (非正则模式)

新增函数 `core::buffer::edit::replace_range`:

```rust
pub fn replace_range(&mut self, range: Range<usize>, replacement: &[u8])
```

直接操作 gap_buffer: delete range, insert replacement.

### 全部替换 (非正则模式)

在 `App::perform_replace_all` 中:
1. 收集当前所有 match ranges
2. 从后往前遍历 (倒序), 每个调用 `replace_range`
3. 标记 dirty, 重新搜索

### 正则替换

使用 `core::buffer::search::find_and_replace` 和 `find_and_replace_all` (已实现, 需要接入 app 层)。

---

## 文件变更清单

| 文件 | 变更 |
|------|------|
| `crates/core/src/buffer/edit.rs` | 新增 `replace_range` |
| `crates/app/src/search_state.rs` | 新增 `replace_query`, `replace_mode`, `focus_replace` |
| `crates/ui/src/widgets/search_bar.rs` | 重构 paint + event, 新增按钮/输入框 |
| `crates/app/src/app_search.rs` | 新增 replace/ replace_all handler |
| `crates/app/src/app_dispatch.rs` | 处理新 SearchBarAction 变体 |
| `crates/app/src/input.rs` | 新增 `EditCommand::FindReplace` + 快捷键映射 |
| `crates/ui/src/theme.rs` | 可能新增 search_bar_accent 色 (若无现成可用) |

---

## 测试要点

- [ ] Find-only 模式行为不变 (回归)
- [ ] 展开/收起 replace 区域
- [ ] Tab 切换焦点
- [ ] 单次替换: 文本正确替换 + 跳到下一匹配
- [ ] 全部替换: 确认对话框 → 全部替换 → undo 回退
- [ ] 转义: `\n` 在查找和替换中正确解析
- [ ] 正则: `.*` 切换 → 正则搜索/替换生效
- [ ] 正则高亮跟随主题色
- [ ] 替换后 dirty flag 正确
- [ ] 替换后 match count 更新
- [ ] 0 匹配时 Replace/All 按钮不可交互
