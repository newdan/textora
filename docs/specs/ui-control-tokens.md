# 基础控件视觉 Token

## 目标

基础控件共享一套物理意义明确的逻辑像素 token；控件对外保留 style 覆盖能力，但默认值只从共享 token 和语义主题色派生。所有尺寸在布局或绘制边界乘一次 DPI。

## 尺寸映射

| 语义 token | 默认值（logical px） | 使用位置 |
|---|---:|---|
| `control_height` | 32 | Button、List 默认行高、表单控件外框 |
| `minimum_hit_target` | 32 | 小图标按钮和行尾动作的最小命中区域 |
| `corner_radius` | 8 | Button 和标准输入控件圆角 |
| `compact_corner_radius` | 4 | Checkbox、Tooltip 等紧凑控件圆角 |
| `focus_ring_width` | 2 | Button、List、Checkbox、Switch 的可见焦点环 |
| `content_spacing` | 8 | 图标/文字、并列控件的标准间距 |
| `compact_spacing` | 4 | 紧凑图标和标签间距 |
| `horizontal_padding` | 12 | 标准 Button/List 水平内边距 |
| `font_size` | 14 | Button/List 默认正文字号 |

## 颜色映射

| 控件状态 | 语义来源 |
|---|---|
| 默认表面/边框 | `ApplicationTheme.control_surface` / `control_border` |
| 主文字/次文字 | `ApplicationTheme.text_primary` / `text_secondary` |
| 选中表面/文字 | `selected_surface` / `navigation_selected_text` |
| hover | `hover_surface` 或调用场景的 navigation hover token |
| focus | `accent`（经 `SettingsTheme.focus_ring` 暴露） |
| Checkbox 勾选图标、Switch 开启态 thumb | `text_inverse` |
| List 关闭图标 | `text_secondary`；hover 使用 `text_primary` |
| disabled | 对相同语义色应用控件局部 alpha，不引入固定 RGB |

## 约束

- `ButtonStyle`、`ListStyle` 等外部 style 可以覆盖尺寸和颜色，但其标准构造器必须从上述 token 派生。
- light/dark 主题均从当前 `Theme` 解析语义色，控件实现中不得写固定白色或灰色。
- hover 混色仅允许在两个语义色之间进行，混合系数必须是有名称的常量。
- 绘制测试同时覆盖 light/dark 和 1x/2x DPI，验证 token 只缩放一次。

## 两端共享按钮规则

- `ui::button::ButtonStyle::from_theme` / `action` 是普通操作按钮的唯一标准构造入口。textora、notora、设置页、分段新建按钮、侧栏打开按钮和空状态操作均使用此入口。
- 普通操作采用 14px 字号、8px 圆角、1px 边框（均为逻辑像素）；文字与图标共享 `text_primary`，背景/边框采用 `control_surface` / `control_border`。
- `ButtonVisualState` 解析 Normal、Hovered、Pressed、Selected、Disabled；悬停与按下分别使用 `hover_surface`、`selected_surface`。选中按钮按下时保持成对的选中前景与背景，避免反色文字落在浅底上。
- 禁用文字、背景和边框应用 0.45 局部透明度；公共绘制方法再统一乘一次 `PaintCtx.global_alpha`。分段按钮的内部分隔线使用同一边框色与透明度。
- `category`、`segmented`、`ghost` 明确表示导航、分段选项、内嵌图标角色；这些角色可以使用透明背景和无边框，不复制普通操作按钮的样式实现。
- 产品层只映射交互状态与展示数据。分段按钮保留独立区域悬停/按下和菜单打开状态；原有手绘操作按钮保留业务触发方式，通过共享规则绘制反馈。
