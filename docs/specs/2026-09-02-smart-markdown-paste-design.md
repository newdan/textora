# Markdown WYSIWYG 智能粘贴设计

日期：2026-09-02

## 背景

当前文档粘贴链路只通过 `ui::core::Clipboard::read_text()` 读取系统剪贴板的纯文本表示。`appkit-shell` 使用 `arboard::Clipboard::get_text()`，app 层随后仅执行 BOM 去除和 CRLF/CR 到 LF 的规范化，再直接插入文档。

这条链路有两个直接后果：

1. 浏览器和办公软件同时写入剪贴板的 HTML、RTF、来源 URL 等富表示没有被读取，因此标题、段落、列表、引用、代码、表格、链接和行内样式在进入 Textora 前已经丢失。
2. `EditCommand::Paste` 当前不能转换为 `EditIntent`，粘贴仍走旧命令路径，没有进入 WYSIWYG 的统一编辑事务。

问题根因是缺少“多表示剪贴板采集 → 富内容转换 → 视图策略选择”的完整链路，不是 Markdown 渲染器丢失了已经插入的样式。

## 目标

1. Markdown WYSIWYG 中的普通粘贴自动选择剪贴板的最佳表示，并转换为与当前渲染器兼容的 Markdown。
2. 支持来自浏览器、办公软件和 Markdown 编辑器的内容。
3. 源码视图、纯文本编辑器和“粘贴为纯文本”始终忽略 HTML、RTF 等样式表示。
4. 富格式转换失败或无法证明文字完整时，可靠回退到原始纯文本。
5. 粘贴通过统一编辑事务完成，选区替换、光标更新、Undo/Redo、dirty 状态和插件 generation 同步保持原子性。
6. 保留网络图片 URL，不保存剪贴板内嵌图片，不发起网络请求。

## 非目标

- 不保存 `data:`、`file:`、`cid:` 或剪贴板二进制图片为本地附件。
- 不保留字体、字号、颜色、背景色、文字对齐、下划线等没有稳定 Markdown 对应的视觉属性。
- 不追求任意 HTML/CSS 的像素级还原。
- 不在第一版引入异步粘贴以及相应的文档 generation 竞态处理。
- 不修改 Markdown 渲染、布局、source projection 或 hit-test 规则。
- 不让通用 TextBox、SearchBar 等 UI 控件读取富剪贴板表示。

## 设计原则

### 分层

- `appkit-shell` 是平台剪贴板所有者，只负责采集平台提供的各种表示。
- `textora-markdown` 是 Markdown 语义所有者，只负责富内容解析、选择和 Markdown 输出。
- `app` 是策略编排者，根据当前视图和用户命令决定采用智能粘贴还是纯文本粘贴，并执行编辑事务。
- `ui` 只暴露纯数据能力，不依赖 `DocumentView`、Workspace 或平台剪贴板实现。

### 内容优先

样式保真不能以文字缺失、重复或乱序为代价。有纯文本表示时，HTML/RTF 转换候选必须通过可见文字等价校验；校验失败即回退纯文本。来源应用显式声明的 Markdown 是原始源码表示，不按渲染后的可见文字校验。

### 确定性

格式选择依赖剪贴板表示和内容结构，不依赖来源应用名称。同一组剪贴板表示必须得到相同结果。

## 架构

```text
系统剪贴板
  │
  ▼
appkit-shell::ClipboardSnapshot
  │  markdown / html / rtf / plain / source_url
  ▼
app 粘贴策略
  ├─ PlainText ───────────────────────────────┐
  └─ SemanticMarkdown                        │
          │                                  │
          ▼                                  │
    textora-markdown::prepare_paste()         │
          │                                  │
          └─ Markdown / HTML / RTF / fallback│
                                             ▼
                                  EditIntent::InsertText
                                             │
                                             ▼
                                      EditTransaction
                                             │
                                             ▼
                                    重新解析与 WYSIWYG 渲染
```

## 组件设计

### 1. 视图粘贴能力

在 `ui::plugin` 定义纯数据枚举：

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PastePreference {
    PlainText,
    SemanticMarkdown,
}
```

`ViewPlugin` 新增默认方法：

```rust
fn paste_preference(&self) -> PastePreference {
    PastePreference::PlainText
}
```

只有 `MarkdownEditorView` 覆盖为 `SemanticMarkdown`。基础源码编辑器、`.txt` 编辑器及其他插件沿用默认值。app 不通过插件名称字符串判断粘贴行为。

### 2. 多表示剪贴板快照

`appkit-shell` 新增文档编辑专用的富剪贴板入口。现有 `ui::core::Clipboard` 纯文本 trait 保持不变，继续服务通用 UI 控件。

```rust
pub struct ClipboardSnapshot {
    pub markdown_text: Option<String>,
    pub html_text: Option<String>,
    pub rtf_bytes: Option<Vec<u8>>,
    pub plain_text: Option<String>,
    pub source_url: Option<String>,
}
```

所有表示必须在一次平台剪贴板会话中读取，避免读取过程中剪贴板所有者变化，导致不同字段来自不同复制操作。

平台实现负责把本机 MIME/UTI/注册格式映射到上述字段：

- macOS：从 NSPasteboard 读取 Markdown、HTML、RTF、UTF-8 纯文本和可用来源 URL。
- Windows：读取已注册 Markdown 格式、CF_HTML、RTF 和 Unicode 纯文本；解析 CF_HTML 的片段边界及 `SourceURL`。
- Linux：读取剪贴板提供的 `text/markdown`、`text/html`、`text/rtf` 与 `text/plain` target。

某个平台或桌面环境不提供某种表示时，对应字段为 `None`，不视为错误。

### 3. 富内容中间树

`textora-markdown` 新增独立粘贴转换模块。HTML 和 RTF 先解析到统一的内部树，再由一个 Markdown writer 输出，避免两套转换器产生不同的转义、空行和嵌套规则。

```text
RichDocument
├── Heading
├── Paragraph
├── BlockQuote
├── OrderedList / UnorderedList
├── CodeBlock
├── Table
├── HorizontalRule
└── Inline
    ├── Text
    ├── Strong / Emphasis / Strikethrough
    ├── InlineCode
    ├── Link
    ├── RemoteImage
    └── LineBreak
```

每个节点只表达 Markdown 能承载的语义，不存储字体、颜色等不可输出属性。节点命名和互斥状态使用 enum，不使用多组布尔字段组合样式状态。

### 4. 准备结果

转换入口返回带来源信息的枚举，供测试、日志和后续诊断使用：

```rust
pub enum PreparedPaste {
    Markdown(String),
    HtmlConverted(String),
    RtfConverted(String),
    PlainTextFallback {
        text: String,
        reason: PasteFallbackReason,
    },
    Empty,
}
```

`PasteFallbackReason` 至少区分：富表示缺失、无语义 HTML、解析失败、文字不等价和转换结果为空。生产 UI 不因正常降级弹出提示。

## 命令与数据流

### 命令

保留 `EditCommand::Paste`，新增 `EditCommand::PastePlainText`：

- `Cmd/Ctrl+V` → `Paste`
- macOS `Cmd+Shift+V`、其他平台对应快捷键 → `PastePlainText`

菜单项使用平台合适的文案，例如“粘贴并匹配样式”或“粘贴为纯文本”，但最终都映射为 `PastePlainText`。

### 普通粘贴

1. app 查询活动插件的 `paste_preference()`。
2. `PlainText`：只读取 `text/plain`，不请求、不解析 HTML/RTF。
3. `SemanticMarkdown`：读取一次 `ClipboardSnapshot`，交给 `textora-markdown` 准备内容。
4. 对最终字符串执行现有 BOM/EOL 规范化。
5. 内容非空时构造 `EditIntent::InsertText`，进入插件编辑策略和统一事务。
6. 内容为空或剪贴板不可读时，不修改选区和文档。

### 粘贴为纯文本

`PastePlainText` 无视活动插件的偏好，只读取 `text/plain`，执行既有 BOM/EOL 规范化后通过同一编辑事务插入。不读取 HTML、RTF 或 Markdown 自定义格式。

在 Markdown WYSIWYG 中，纯文本内本来就存在的 Markdown 标记仍属于用户插入的原始字符；该命令只保证不从富剪贴板表示推导额外样式，不额外转义用户文本。

### 原子性

- 必须在读取和准备内容成功后才构建选区替换事务。
- 一次粘贴只产生一个独立 undo entry，不与前后连续输入合并。
- 事务完成后统一更新光标、selection、dirty 状态、content revision 和插件 source generation。
- 转换、校验或剪贴板读取失败不得提前删除选区。

## 最佳格式选择

`SemanticMarkdown` 使用以下确定性优先级：

1. 非空的显式 Markdown 表示。
2. 含真实语义结构且可成功转换的 HTML。
3. 可成功转换的 RTF。
4. 原始纯文本。

### 显式 Markdown

显式 `text/markdown` 或平台等价格式直接采用，不重新排版、不二次转义，也不与可能代表渲染文本的 `text/plain` 做可见文字等价校验。仍执行 BOM/EOL 规范化。

### HTML 语义判断

以下内容视为语义信号：

- 标题、段落、块引用、列表、代码块、表格、分隔线。
- 粗体、斜体、删除线、行内代码、链接、图片和显式换行。
- Office HTML 中可确定映射为上述语义的行内样式或列表标记。

若 HTML 只有 `html/body/div/span` 包装、语法高亮 class、字体或颜色样式，而没有可映射语义，则使用纯文本。这样可避免从源码编辑器复制 Markdown 时，把高亮 HTML 当成渲染内容二次转换。

如果 Markdown 编辑器复制的是渲染预览并提供语义 HTML，则按 HTML 转换；如果复制的是源码并只提供纯文本或装饰性高亮 HTML，则保留源码。

### RTF

RTF 仅在没有可用 HTML 时参与转换。第一版 RTF fallback 明确支持：

- 段落与显式换行。
- 粗体、斜体和删除线。
- Unicode 字符、转义字符和声明代码页下的文本。
- 可识别的超链接字段。
- 可可靠识别的简单有序/无序列表。

无法可靠还原的复杂 RTF 表格、浮动对象、文本框和内嵌图片退化为有序的可见文字。只要纯文本存在且两者不等价，就使用纯文本。

## HTML/RTF 到 Markdown 映射

| 富内容语义 | Markdown 输出 |
|---|---|
| H1–H6 | `#`–`######` ATX 标题 |
| Paragraph | 块间一个空行 |
| Strong / Bold | `**text**` |
| Emphasis / Italic | `*text*` |
| Strikethrough | `~~text~~` |
| Inline code | 安全长度的反引号围栏 |
| Ordered / unordered list | 编号或 `-`，保留嵌套层级 |
| Block quote | 每个逻辑行添加 `> ` |
| Preformatted code | 安全长度的 fenced code block |
| Table | GFM pipe table；无法表达的单元格内容安全扁平化 |
| Link | `[label](url "title")`，title 可省略 |
| Remote image | `![alt](http-or-https-url)` |
| Horizontal rule | `---` |
| Explicit line break | Markdown hard break |

HTML 的 `<strong>`、`<b>`、`<em>`、`<i>`、`<del>`、`<s>` 等语义标签直接映射。Office 常见的 `font-weight`、`font-style` 和 `text-decoration: line-through` 行内 CSS 也映射到对应行内语义。其余视觉 CSS 被忽略但保留可见文字。

Markdown writer 必须：

- 对普通文本中的 Markdown 控制字符做上下文相关转义。
- 根据内容中连续反引号的最大长度选择更长的行内或块级代码围栏。
- 保证块间空行确定且不随输入标签包装差异漂移。
- 保持纯文本中的 Unicode 原值，不做兼容分解或排版替换。
- 为嵌套列表、引用内列表和列表内代码生成当前 `pulldown-cmark` 配置可解析的结构。

## 链接和图片

- `http:` 和 `https:` 图片生成 Markdown 图片语法。
- `data:`、`file:`、`cid:` 及剪贴板二进制图片不保存、不引用；有 alt 时保留 alt 文字。
- 相对链接和图片仅在 `source_url` 是有效绝对 URL 时解析为绝对地址。
- 不读取链接目标，不下载图片，不探测远端 MIME 类型。
- 危险或不可表达的 URL scheme 不生成可点击 Markdown 链接，只保留标签文字。

## 可见文字等价校验

当快照同时包含纯文本和 HTML/RTF 时，转换器分别取得：

1. 富内容树按阅读顺序抽取的可见文字。
2. 剪贴板提供的纯文本。

比较前只执行用于校验的规范化：

- 去除开头 BOM。
- 将 CRLF/CR 统一为 LF。
- 将不换行空格按普通空格比较。
- 在非代码上下文中折叠连续空白；代码上下文保持内容和顺序。

文字序列不等价时，立即返回 `PlainTextFallback::TextMismatch`，不继续尝试同一快照中的其他富表示。忽略 `script`、`style`、隐藏元数据和被拒绝的内嵌图片不算文字缺失；图片 alt 参与可见文字比较。

没有纯文本表示时，成功解析的 Markdown、HTML 或 RTF 可以直接采用。解析结果为空则返回 `Empty`。

## 安全和错误处理

- HTML 解析完全离线，不执行脚本、事件处理器或 CSS。
- `script`、`style`、`template`、元数据和不可见控制内容不进入输出。
- RTF 解析器必须限制递归/分组深度和单个控制字长度，限制值使用语义化常量。
- 畸形 HTML、未知标签、非法 RTF 控制字和截断输入不得 panic。
- 未知 HTML 标签默认保留其可见子文本；未知 RTF destination 默认跳过该 destination，不跳过已确认的普通文字。
- 所有 URL 解析失败均降级为标签文字，不拼接猜测地址。
- 正常降级只记录可测试的结构化原因，不打扰用户。

## 性能

第一版同步执行剪贴板快照读取和线性转换，以保持粘贴时光标、选区和文档 generation 的直观语义。实现必须避免二次方字符串拼接，Markdown writer 使用预分配缓冲区。

建立 HTML 与 RTF 大文本基准，至少覆盖：

- 大量短段落。
- 深度受控的嵌套列表。
- 大表格。
- 长代码块。
- 大量行内 span 的 Office/网页内容。

若基准或真机测试证明同步转换超过交互预算，再单独设计后台转换。异步版本必须捕获源 generation、选区和光标，并在应用前验证它们未变化；本设计不提前引入该复杂度。

## 测试设计

### 转换器单元测试

- 标题、段落、软/硬换行、嵌套列表、引用、代码块、表格和分隔线。
- 粗体、斜体、删除线、链接、行内代码及组合嵌套。
- Markdown 特殊字符转义和包含不同长度反引号的代码。
- 网络图片、相对图片、内嵌图片和无效 scheme。
- Office 行内 CSS、列表 HTML 和常见冗余包装。
- 畸形 HTML、非法/截断 RTF、未知标签与 destination。
- 中文、Emoji、组合字符、NBSP、LF、CRLF 和 CR。

### 格式选择测试

- 显式 Markdown 优先于其他表示。
- 有语义 HTML 优先于 RTF 和纯文本。
- 只有语法高亮 span 的 HTML 回退纯文本。
- HTML 解析失败后尝试 RTF，RTF 失败后回退纯文本。
- 富格式出现文字遗漏、重复或乱序时回退纯文本。
- 没有纯文本时采用成功的富格式转换。
- 普通纯文本的所有换行原样保留。

### app 集成测试

- WYSIWYG 的 `Paste` 执行智能转换。
- 源码视图和 `.txt` 的 `Paste` 只读取纯文本。
- 所有编辑视图的 `PastePlainText` 强制纯文本。
- 粘贴正确替换向前和向后选区，光标落到插入内容末尾。
- 一次 Undo 完整撤销粘贴，一次 Redo 完整恢复。
- 剪贴板读取失败、转换结果为空或全部表示为空时选区不变。
- 粘贴后 dirty、content revision、source generation 和 WYSIWYG 重绘同步。

### 平台适配测试

- 平台读取器依赖可注入后端，主要测试不操作真实系统剪贴板。
- macOS、Windows、Linux 分别覆盖格式映射和缺失表示。
- CF_HTML 片段边界和 `SourceURL` 使用固定 fixture 测试。
- 真实系统剪贴板集成测试只作为平台专用、可跳过测试，不承担核心逻辑覆盖。

### 手工验收矩阵

- 浏览器：Safari、Chrome 的文章正文、列表、代码、表格、链接与网络图片。
- 办公软件：Word、Pages、飞书的段落、标题、列表、链接和常用行内样式。
- Markdown 工具：VS Code、Typora 等的源码选择与渲染预览选择。
- 模式：Markdown WYSIWYG、Markdown 源码视图、`.txt`、`PastePlainText`。

重大修改完成后运行 `./scripts/verify.sh`。转换器另运行大文本基准并记录结果。

## 验收标准

1. 浏览器内容的段落、标题、列表、引用、代码、表格、常用行内样式和链接能转换为可重新解析的 Markdown。
2. Office 内容优先使用 HTML；没有可用 HTML 时，RTF 能恢复已定义的基础结构和样式。
3. Markdown 源码不会因装饰性高亮 HTML 被二次转换。
4. 源码视图、纯文本编辑器和 `PastePlainText` 不读取或推导富格式样式。
5. 普通纯文本不做段落重排，原始单换行和空行均保留，仅执行既有 BOM/EOL 规范化。
6. 富转换不能证明文字完整时自动回退纯文本。
7. 粘贴选区替换原子完成，一次 Undo 可完整撤销。
8. 不创建图片附件、不请求网络、不执行剪贴板中的活动内容。
9. `./scripts/verify.sh` 通过，新增测试和基准覆盖上述规则。

## 实施阶段边界

该功能会修改超过三个文件，实施时按项目约定拆为独立子任务：

1. 平台剪贴板快照模型与三平台适配。
2. `textora-markdown` 富内容树、HTML/RTF 解析、Markdown writer 和格式选择。
3. ViewPlugin 粘贴能力、`PastePlainText` 命令与 app 统一事务接入。
4. 跨层集成测试、平台 fixture、手工验收和全面验证。

每个阶段保持单一职责并独立编译通过；后续阶段只依赖前一阶段公开的纯数据接口。
