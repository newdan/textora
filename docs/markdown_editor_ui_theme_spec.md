# Markdown 编辑器 UI 配色与尺寸精简规范

适用对象：macOS 风格 Markdown / 文档编辑器。  
目标：保持当前产品结构不变，提升精致感、可读性与专业度。

---

## 1. 基础尺寸规范

| 区域 / 元素 | 建议值 |
|---|---:|
| App 窗口基准 | 1440 × 1024 px |
| 左侧边栏宽度 | 248 px |
| 顶部标题栏高度 | 64 px |
| 底部状态栏高度 | 32 px |
| 主内容外边距 | 24–32 px |
| 正文卡片最大宽度 | 960 px |
| 正文卡片内边距 | 44 px 48 px |
| 正文最大阅读宽度 | 760–820 px |
| 文件列表项高度 | 34–36 px |
| 文件图标尺寸 | 16 × 16 px |
| 顶部工具图标尺寸 | 18 × 18 px |
| 正文标题前图标 | 20 × 20 px |
| Step 数字徽标 | 24 × 24 px |
| 主内容卡片圆角 | 14 px |
| 信息块 / 代码块圆角 | 10–12 px |
| 文件选中项圆角 | 8 px |

---

## 2. 字体层级

| 层级 | 字号 | 行高 | 字重 |
|---|---:|---:|---:|
| H1 主标题 | 36 px | 46 px | 700 |
| H2 二级标题 | 23 px | 32 px | 650 |
| H3 三级标题 | 17 px | 26 px | 600 |
| 正文 / 列表 | 16 px | 28 px | 400 |
| 辅助说明 / 路径 | 12–13 px | 16–20 px | 400 |
| 代码 | 13.5 px | 22 px | 400 |

排版原则：

- 正文行高不低于 1.65，保证中文阅读舒适。
- 路径、状态栏、辅助说明需要弱化。
- 代码块使用等宽字体，例如 `SF Mono`、`JetBrains Mono`、`Menlo`。

---

## 3. 浅色模式配色

### 3.1 色彩 Token

```css
:root[data-theme="light"] {
  /* Background */
  --bg-app: #F7F6F3;
  --bg-main: #FAF9F7;
  --bg-sidebar: #F4F1EC;
  --bg-card: #FFFFFF;
  --bg-hover: rgba(255, 255, 255, 0.58);

  /* Border */
  --border-default: #E6E1D8;
  --border-subtle: #EFEAE3;

  /* Text */
  --text-primary: #111827;
  --text-secondary: #374151;
  --text-muted: #6B7280;
  --text-faint: #9AA3AF;
  --text-disabled: #C1C7D0;

  /* Accent */
  --accent-blue: #2F7CF6;
  --accent-blue-hover: #1F6FEB;
  --accent-blue-soft: #EAF2FF;
  --accent-blue-border: #CFE0FF;

  --accent-orange: #E97924;
  --accent-orange-soft: #FFF3E8;
  --accent-orange-border: #F7D7BE;

  --success: #4CBF73;

  /* Code */
  --code-bg-command: #F3F7FD;
  --code-border-command: #DCE8F7;
  --code-bg-python: #F4FAF8;
  --code-border-python: #DDEDEA;
}
```

### 3.2 使用规则

| 用途 | 推荐颜色 |
|---|---|
| App 外层背景 | `#F7F6F3` |
| 侧边栏背景 | `#F4F1EC` |
| 主内容背景 | `#FAF9F7` |
| 文档卡片背景 | `#FFFFFF` |
| 主标题 | `#111827` |
| 正文 | `#374151` |
| 弱文字 / 路径 | `#9AA3AF` |
| 分隔线 | `#E6E1D8` |
| 文件选中底色 | `#FFF3E8` |
| 文件选中图标 | `#E97924` |
| 操作 / 步骤强调色 | `#2F7CF6` |
| 成功状态 | `#4CBF73` |

---

## 4. 暗夜模式配色

### 4.1 色彩 Token

```css
:root[data-theme="dark"] {
  /* Background */
  --bg-app: #111318;
  --bg-main: #0F1117;
  --bg-sidebar: #17191F;
  --bg-card: #181B22;
  --bg-elevated: #20242D;
  --bg-hover: #252A34;
  --bg-active: #2B313D;

  /* Border */
  --border-default: #2A2F3A;
  --border-subtle: #232832;
  --border-strong: #3A4150;

  /* Text */
  --text-primary: #F2F4F8;
  --text-secondary: #D4D9E2;
  --text-body: #C4CAD5;
  --text-muted: #8E97A8;
  --text-faint: #687283;
  --text-disabled: #4F5868;

  /* Accent */
  --accent-blue: #5B9CFF;
  --accent-blue-hover: #7EB2FF;
  --accent-blue-soft: rgba(91, 156, 255, 0.14);
  --accent-blue-border: rgba(91, 156, 255, 0.34);

  --accent-orange: #F59A4A;
  --accent-orange-hover: #FFB36C;
  --accent-orange-soft: rgba(245, 154, 74, 0.14);
  --accent-orange-border: rgba(245, 154, 74, 0.34);

  --success: #5FD18B;

  /* Code */
  --code-bg-command: #172236;
  --code-border-command: #273A57;
  --code-bg-python: #142923;
  --code-border-python: #24483D;
}
```

### 4.2 使用规则

| 用途 | 推荐颜色 |
|---|---|
| App 外层背景 | `#111318` |
| 主内容背景 | `#0F1117` |
| 侧边栏背景 | `#17191F` |
| 文档卡片背景 | `#181B22` |
| 主标题 | `#F2F4F8` |
| 正文 | `#C4CAD5` |
| 弱文字 / 路径 | `#687283` |
| 分隔线 | `#2A2F3A` |
| 文件选中底色 | `rgba(245, 154, 74, 0.14)` |
| 文件选中图标 | `#F59A4A` |
| 操作 / 步骤强调色 | `#5B9CFF` |
| 成功状态 | `#5FD18B` |

暗夜模式避免使用纯黑 `#000000` 和纯白 `#FFFFFF`，否则阅读时会刺眼，也会显得廉价。

---

## 5. 关键组件样式

### 5.1 左侧文件选中态

浅色模式：

```css
.sidebar-item.active {
  background: #FFF3E8;
  color: #9A4D12;
  font-weight: 500;
}

.sidebar-item.active .icon {
  color: #E97924;
}
```

暗夜模式：

```css
.sidebar-item.active {
  background: rgba(245, 154, 74, 0.14);
  color: #F2D1B6;
  font-weight: 500;
}

.sidebar-item.active .icon {
  color: #F59A4A;
}
```

---

### 5.2 主内容卡片

浅色模式：

```css
.document-card {
  max-width: 960px;
  margin: 0 auto;
  padding: 44px 48px;
  border-radius: 14px;
  background: #FFFFFF;
  border: 1px solid #E6E1D8;
  box-shadow:
    0 1px 2px rgba(15, 23, 42, 0.04),
    0 12px 36px rgba(15, 23, 42, 0.06);
}
```

暗夜模式：

```css
.document-card {
  max-width: 960px;
  margin: 0 auto;
  padding: 44px 48px;
  border-radius: 14px;
  background: #181B22;
  border: 1px solid #2A2F3A;
  box-shadow:
    0 1px 2px rgba(0, 0, 0, 0.28),
    0 18px 48px rgba(0, 0, 0, 0.32);
}
```

---

### 5.3 元信息块

浅色模式：

```css
.meta-card {
  padding: 18px 20px;
  border-radius: 12px;
  background: #FFFDFC;
  border: 1px solid #F0E2D2;
}
```

暗夜模式：

```css
.meta-card {
  padding: 18px 20px;
  border-radius: 12px;
  background: #211D1A;
  border: 1px solid #3B2B21;
  color: #D8C7B8;
}
```

---

### 5.4 代码块

通用尺寸：

```css
.code-block {
  position: relative;
  margin: 12px 0 18px;
  padding: 16px 18px;
  border-radius: 10px;
  font-family: "SF Mono", "JetBrains Mono", Menlo, Consolas, monospace;
  font-size: 13.5px;
  line-height: 22px;
}
```

浅色模式：

```css
.code-block.command {
  background: #F3F7FD;
  border: 1px solid #DCE8F7;
}

.code-block.python {
  background: #F4FAF8;
  border: 1px solid #DDEDEA;
}
```

暗夜模式：

```css
.code-block.command {
  background: #172236;
  border: 1px solid #273A57;
  color: #D8E6FF;
}

.code-block.python {
  background: #142923;
  border: 1px solid #24483D;
  color: #D9F1EA;
}
```

复制按钮：

```css
.copy-button {
  position: absolute;
  top: 10px;
  right: 10px;
  height: 28px;
  padding: 0 10px;
  border-radius: 7px;
  font-size: 12px;
}
```

---

### 5.5 Step 数字徽标

浅色 / 暗夜模式均可使用：

```css
.step-badge {
  width: 24px;
  height: 24px;
  border-radius: 6px;
  background: linear-gradient(180deg, #6EADFF 0%, #3F83F8 100%);
  color: #FFFFFF;
  font-size: 13px;
  font-weight: 600;
}
```

---

## 6. 最小落地建议

优先改这 6 项即可明显提升质感：

1. 将侧边栏宽度固定为 `248px`，文件项高度改为 `36px`。
2. 顶部栏高度统一为 `64px`，标题和路径分层显示。
3. 正文使用居中卡片，最大宽度 `960px`，内边距 `44px 48px`。
4. 正文字号使用 `16px / 28px`，H1 使用 `36px / 46px`。
5. 文件选中态改为低饱和暖橙底色。
6. 代码块增加浅色底、圆角、边框、复制按钮和足够内边距。

---

## 7. 设计原则总结

- 用留白和层级提升精致感，不靠大面积强色。
- 浅色模式使用暖中性色，避免死白。
- 暗夜模式使用深灰蓝，避免纯黑。
- 文件选中态使用暖橙，功能操作和步骤标识使用蓝色。
- 正文区域控制阅读宽度，避免横向铺满。
- 状态栏、路径、辅助信息要弱化，不抢正文注意力。
