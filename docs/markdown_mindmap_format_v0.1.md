# Markdown Mindmap Format 规范说明

版本：v0.1  
简称：MMF  
定位：一种基于 Markdown 的思维导图文本存储格式。

---

## 1. 设计目标

Markdown Mindmap Format 的目标是使用普通 Markdown 文件保存思维导图的结构、内容和必要属性。

本格式优先保证：

1. **标题可读**：节点标题不混入属性、标签、ID 等元信息。
2. **结构清晰**：使用 Markdown 标题层级表示思维导图层级。
3. **属性统一**：使用 TOML 代码区保存全局属性和节点属性。
4. **普通兼容**：在普通 Markdown 编辑器中仍然可以阅读和编辑。
5. **易于解析**：程序可以稳定地将 Markdown 解析为思维导图 AST。

本格式不追求直接保存复杂视觉布局，例如节点精确坐标、连线曲率、缩放比例等。这些信息应由渲染器自动计算，或在后续版本中作为可选扩展处理。

---

## 2. 基本结构

一个 MMF 文件由三部分组成：

```text
全局属性块，可选
标题层级，也就是节点树
节点属性块，可选
```

基本示例：

````markdown
```toml mindmap
version = 1
layout = "auto"
```

# 产品规划

## 数据同步

```toml node
priority = "P1"
status = "todo"
owner = "Dan"
```

需要支持本地文件、云端同步和冲突解决。

## AI 生成

支持从 Prompt 生成大纲、导图和文档。
````

---

## 3. 标题与节点

### 3.1 标题即节点

Markdown 标题表示思维导图节点：

```markdown
# 中心主题

## 一级节点

### 二级节点

#### 三级节点
```

映射规则：

```text
#      根节点
##     一级子节点
###    二级子节点
####   三级子节点
```

### 3.2 根节点

一个 MMF 文件建议只包含一个一级标题 `#`，该标题作为思维导图的中心主题。

推荐：

```markdown
# 产品规划
```

不推荐在同一文件中出现多个一级标题：

```markdown
# 产品规划

# 技术方案
```

如果确实存在多个一级标题，解析器可以选择以下策略之一：

1. 将第一个 `#` 作为根节点，其余 `#` 作为同级顶层节点。
2. 自动创建一个虚拟根节点，将所有 `#` 作为其子节点。
3. 报告格式警告。

推荐默认策略：**一个文件只允许一个显式根节点**。

---

## 4. 全局属性块

### 4.1 写法

全局属性使用 TOML 代码区：

````markdown
```toml mindmap
version = 1
layout = "auto"
```
````

### 4.2 位置

全局属性块应位于文件开头，在第一个标题之前。

推荐：

````markdown
```toml mindmap
version = 1
layout = "auto"
```

# 产品规划
````

### 4.3 常用字段

```toml
version = 1
layout = "auto"
theme = "default"
direction = "auto"
```

字段说明：

| 字段 | 类型 | 说明 |
|---|---|---|
| `version` | integer | 格式版本 |
| `layout` | string | 布局方式，例如 `auto`、`right`、`radial` |
| `theme` | string | 主题名称 |
| `direction` | string | 导图方向，例如 `auto`、`left`、`right`、`both` |

### 4.4 是否必需

全局属性块是可选的。

最小合法文件可以只有标题：

```markdown
# 产品规划

## 数据同步

## AI 生成
```

---

## 5. 节点属性块

### 5.1 写法

节点属性使用 TOML 代码区：

````markdown
## 数据同步

```toml node
priority = "P1"
status = "todo"
owner = "Dan"
collapsed = false
```

需要支持本地文件、云端同步和冲突解决。
````

### 5.2 绑定规则

节点属性块绑定到它前面的最近标题节点。

例如：

````markdown
## 数据同步

```toml node
priority = "P1"
```
````

表示 `priority = "P1"` 属于 `数据同步` 这个节点。

### 5.3 位置规则

节点属性块必须出现在标题之后、节点正文之前。

允许标题和属性块之间存在空行：

````markdown
## 数据同步

```toml node
priority = "P1"
```

节点备注。
````

不推荐在正文之后再写节点属性：

````markdown
## 数据同步

需要支持多端同步。

```toml node
priority = "P1"
```
````

解析器可以将后者视为普通代码块，或报告格式警告。

### 5.4 常用字段

```toml
id = "sync"
priority = "P1"
status = "todo"
owner = "Dan"
collapsed = false
tags = ["core", "v1"]
color = "blue"
```

字段说明：

| 字段 | 类型 | 说明 |
|---|---|---|
| `id` | string | 节点稳定 ID |
| `priority` | string | 优先级，例如 `P0`、`P1`、`P2` |
| `status` | string | 状态，例如 `todo`、`doing`、`done` |
| `owner` | string | 负责人 |
| `collapsed` | boolean | 是否默认折叠 |
| `tags` | array | 标签列表 |
| `color` | string | 节点颜色或主题色名称 |

### 5.5 节点属性是否必需

节点属性块是可选的。

普通节点可以只写标题和正文：

```markdown
## AI 生成

支持从 Prompt 生成大纲、导图和文档。
```

只有需要保存属性的节点才需要写 `toml node`。

---

## 6. 节点正文

标题之后，除节点属性块之外的普通 Markdown 内容，视为该节点的备注内容。

示例：

````markdown
## 数据同步

```toml node
priority = "P1"
status = "todo"
```

需要支持：

- 本地文件
- 云端同步
- 冲突解决
````

解析结果：

```json
{
  "title": "数据同步",
  "priority": "P1",
  "status": "todo",
  "note": "需要支持：\n\n- 本地文件\n- 云端同步\n- 冲突解决"
}
```

---

## 7. 子节点与正文边界

子标题表示当前节点的子节点。

```markdown
## 数据同步

这是数据同步节点的备注。

### 本地文件

这是本地文件子节点的备注。

### 云端同步

这是云端同步子节点的备注。
```

结构为：

```text
数据同步
├── 本地文件
└── 云端同步
```

其中：

```text
“这是数据同步节点的备注。” 属于 数据同步
“这是本地文件子节点的备注。” 属于 本地文件
“这是云端同步子节点的备注。” 属于 云端同步
```

---

## 8. 代码区识别规则

MMF 使用代码区信息串区分特殊代码块。

### 8.1 全局属性代码区

````markdown
```toml mindmap
version = 1
layout = "auto"
```
````

含义：整张思维导图的属性。

### 8.2 节点属性代码区

````markdown
```toml node
priority = "P1"
status = "todo"
```
````

含义：当前标题节点的属性。

### 8.3 普通代码区

其他代码区均视为节点正文的一部分。

例如：

````markdown
## Markdown 示例

```markdown
# 标题

## 子标题
```
````

这里的 `markdown` 代码区不是节点属性，而是当前节点的备注内容。

---

## 9. 推荐字段规范

### 9.1 `id`

节点 ID 用于稳定识别节点。

推荐写法：

```toml
id = "data-sync"
```

建议：

1. 在协同编辑、评论、引用、历史追踪场景中使用。
2. 普通个人导图可以省略。
3. ID 应在同一文件内唯一。

### 9.2 `priority`

推荐值：

```toml
priority = "P0"
priority = "P1"
priority = "P2"
priority = "P3"
```

含义建议：

| 值 | 含义 |
|---|---|
| `P0` | 最高优先级 |
| `P1` | 高优先级 |
| `P2` | 中优先级 |
| `P3` | 低优先级 |

### 9.3 `status`

推荐值：

```toml
status = "todo"
status = "doing"
status = "done"
status = "blocked"
status = "canceled"
```

含义建议：

| 值 | 含义 |
|---|---|
| `todo` | 待处理 |
| `doing` | 进行中 |
| `done` | 已完成 |
| `blocked` | 受阻 |
| `canceled` | 已取消 |

### 9.4 `tags`

标签使用字符串数组：

```toml
tags = ["core", "v1", "important"]
```

不建议在标题中写 `#tag`，以避免影响标题可读性。

### 9.5 `owner`

负责人使用字符串：

```toml
owner = "Dan"
```

如果需要多人负责人，可以使用数组扩展：

```toml
owners = ["Dan", "Alice"]
```

### 9.6 `collapsed`

是否默认折叠：

```toml
collapsed = true
```

### 9.7 `color`

颜色建议使用语义名称，而不是直接使用十六进制颜色值：

```toml
color = "blue"
color = "warm"
color = "warning"
```

不推荐：

```toml
color = "#ff9900"
```

原因是颜色值属于视觉主题层，直接写死会降低跨主题适配能力。

---

## 10. 完整示例

````markdown
```toml mindmap
version = 1
layout = "auto"
theme = "default"
```

# AI 文档工具

## 创作入口

```toml node
id = "creation-entry"
priority = "P1"
status = "doing"
tags = ["core", "ai"]
```

用户不再从空白页面开始，而是通过意图、资料和模板启动创作。

### Prompt 生成

用户输入目标、背景和约束，系统生成初稿或大纲。

### 资料生成

用户上传资料，系统提取结构并生成文档内容。

## 编辑过程

```toml node
id = "editing-process"
priority = "P1"
status = "todo"
collapsed = false
```

编辑过程从手动修改文字，转向对结构和意图进行调整。

### 改写

支持调整语气、风格和长度。

### 结构重组

支持重新组织章节、合并节点、拆分节点。

## 导出能力

```toml node
id = "export"
priority = "P2"
status = "todo"
tags = ["export"]
```

第一版支持 SVG 和 PNG，后续支持 XMind、OPML 和 Markdown。
````

---

## 11. 解析流程建议

解析器可以按以下流程处理 MMF 文件：

1. 读取 Markdown 文档。
2. 检查文件开头是否存在 `toml mindmap` 代码区。
3. 解析全局属性。
4. 扫描 Markdown 标题，构建节点树。
5. 对每个标题节点，检查标题后的第一个有效块。
6. 如果该块是 `toml node`，解析为节点属性。
7. 其余内容作为节点备注。
8. 输出思维导图 AST。

示例 AST：

```json
{
  "type": "mindmap",
  "version": 1,
  "layout": "auto",
  "root": {
    "title": "AI 文档工具",
    "children": [
      {
        "id": "creation-entry",
        "title": "创作入口",
        "priority": "P1",
        "status": "doing",
        "tags": ["core", "ai"],
        "note": "用户不再从空白页面开始，而是通过意图、资料和模板启动创作。",
        "children": [
          {
            "title": "Prompt 生成",
            "note": "用户输入目标、背景和约束，系统生成初稿或大纲。"
          },
          {
            "title": "资料生成",
            "note": "用户上传资料，系统提取结构并生成文档内容。"
          }
        ]
      }
    ]
  }
}
```

---

## 12. 设计约束

### 12.1 不在标题行写属性

推荐：

````markdown
## 数据同步

```toml node
priority = "P1"
status = "todo"
```
````

不推荐：

```markdown
## 数据同步 [P1] @Dan #todo
```

原因：标题行应优先保持自然可读。

### 12.2 不使用 HTML 注释保存属性

不推荐：

```markdown
## 数据同步
<!-- node: {"priority":"P1","status":"todo"} -->
```

原因：HTML 注释不易手写，也不够直观。

### 12.3 不默认要求每个节点都有属性块

推荐：

```markdown
## 普通节点

这是一个普通节点。
```

不推荐：

````markdown
## 普通节点

```toml node
```

这是一个普通节点。
````

原因：属性块应按需出现，而不是强制出现。

### 12.4 不保存复杂视觉布局

不推荐：

```toml
x = 120
y = 300
width = 180
height = 60
```

原因：坐标和尺寸属于渲染结果，不属于思维导图的核心语义。

如果必须保存布局信息，建议放入后续扩展字段：

```toml
[layout]
x = 120
y = 300
```

---

## 13. 最小合法示例

```markdown
# 产品规划

## 数据同步

## AI 生成

## 导出能力
```

---

## 14. 推荐示例

````markdown
```toml mindmap
version = 1
layout = "auto"
```

# 产品规划

## 数据同步

```toml node
priority = "P1"
status = "todo"
owner = "Dan"
```

需要支持本地文件、云端同步和冲突解决。

## AI 生成

```toml node
priority = "P2"
status = "doing"
```

支持从 Prompt 生成大纲、导图和文档。

## 导出能力

第一版支持 SVG 和 PNG。
````

---

## 15. 文件扩展名

推荐使用普通 Markdown 扩展名：

```text
.md
```

也可以在产品内部使用专用扩展名：

```text
.mmap.md
.mindmap.md
```

推荐优先使用：

```text
.mmap.md
```

原因是它仍然是 Markdown 文件，同时可以被产品识别为思维导图文件。

---

## 16. 版本演进建议

当前版本为 v0.1，只定义核心结构：

```text
标题层级
toml mindmap
toml node
节点正文
```

后续版本可以扩展：

1. 节点之间的非树状关联。
2. 附件与资源引用。
3. 评论与协同编辑信息。
4. 任务管理字段。
5. 可选布局信息。
6. 与 OPML、XMind、FreeMind 的导入导出映射。

---

## 17. 核心原则总结

MMF 的核心原则是：

```text
标题表达结构
正文表达备注
toml mindmap 表达全局属性
toml node 表达节点属性
```

也就是：

````markdown
```toml mindmap
version = 1
layout = "auto"
```

# 中心主题

## 节点标题

```toml node
priority = "P1"
status = "todo"
```

节点备注。
````

该格式在人类可读性、Markdown 兼容性、程序可解析性之间取得平衡，适合作为思维导图的轻量文本存储格式。
