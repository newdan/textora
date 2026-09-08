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
| `horizontal_padding` | 12 | List 等非按钮控件水平内边距 |
| `font_size` | 14 | Button/List 默认正文字号 |

## 颜色映射

| 控件状态 | 语义来源 |
|---|---|
| 输入控件表面/边框 | `ApplicationTheme.control_surface` / `control_border` |
| 操作按钮表面/边框 | `ApplicationTheme.button_surface` / `button_border` |
| 主文字/次文字 | `ApplicationTheme.text_primary` / `text_secondary` |
| 选中表面/文字 | `selected_surface` / `navigation_selected_text` |
| 普通操作 hover | `hover_surface` / `button_hover_surface`（同一最终色） |
| 普通操作按下、菜单展开 | `button_pressed_surface` |
| 导航、文档列表 hover | `navigation_hover_surface` |
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
- `ui::button::ButtonMetrics` 集中管理按钮横向尺寸：左右内边距 8px、图文间距 4px、并列操作间距 4px、下拉菜单区域宽 24px。固定中文标签宽度按 14px 全宽字形加两侧内边距计算，有图标时再加图标宽度和图文间距。
- 工具条新建按钮的下拉三角中心距菜单区域左边缘 6px，靠近主操作文字；菜单命中区域仍为 24px。
- 工具条、侧栏、设置分段、同步操作、新建与加密弹窗均按内容预留宽度，不拉伸按钮填满空余区域。两个字的普通按钮宽 44px，四个字宽 72px；14px 图标加两个字宽 62px，附带菜单区域的新建按钮宽 86px。编辑器工具条保留 28px 图标按钮及 2px 组内间距，组间距收紧到 8px。
- 按钮高度、图标尺寸、字号、圆角和视觉状态保持既有规则；导航项目、列表和输入框继续使用各自的控件布局。窄布局仍保留原有换行、图标替代和更多菜单机制，命中区域跟随最终矩形。
- 普通操作采用 14px 字号、8px 圆角、1px 边框（均为逻辑像素）；文字与图标共享 `text_primary`，背景/边框采用专用 `button_surface` / `button_border`，避免可用操作按钮呈现输入框的低对比外观。
- 按钮底色在浮层底色中混入 3.5% 主文字色；轮廓在强边框色中混入 12% 主文字色。颜色在主题的线性空间派生，深浅主题均保留清晰文字、底色和轮廓，输入框配色不受影响。
- `ButtonVisualState` 解析 Normal、Hovered、Pressed、Selected、Disabled；悬停与按下分别使用 `button_hover_surface`、`button_pressed_surface`。主题先将 `bg_hover` 覆盖色合成到按钮底色上，得到普通按钮、内嵌图标和弹出菜单共用的不透明悬停色；`hover_surface` 是同一最终色，不再表示待合成的覆盖层。按下色在悬停色中混入 6% 主文字色，让内置深浅主题的反馈逐级增强，避免把 `bg_active` 的内容选中强调色用于普通操作。选中按钮按下时保持成对的选中前景与背景，避免反色文字落在浅底上。
- 禁用文字、背景和边框应用 0.45 局部透明度；公共绘制方法再统一乘一次 `PaintCtx.global_alpha`。分段按钮的内部分隔线使用同一边框色与透明度。
- `category`、`segmented`、`ghost` 明确表示导航、分段选项、内嵌图标角色；这些角色可以使用透明背景和无边框，不复制普通操作按钮的样式实现。
- 产品层只映射交互状态与展示数据。分段按钮保留独立区域悬停/按下和菜单打开状态；原有手绘操作按钮保留业务触发方式，通过共享规则绘制反馈。

- `destructive` 表示回收站永久删除、清空及最终确认；图标与文字统一使用 `button_danger_foreground`（危险色混入 60% 主文字色，保证内置深浅主题启用态的文字对比度至少 4.5:1），底色与边框从普通按钮色和危险色派生，尺寸、圆角及禁用透明度沿用公共规则。恢复、取消和文件记录清空仍使用普通操作样式。

## 操作悬停一致性

| 组件/场景 | 悬停 | 按下或持续状态 |
|---|---|---|
| 普通按钮、工具条按钮、分体新建按钮 | 公共操作悬停色 | 按下色；菜单展开时只保持菜单区域的按下色 |
| 弹出菜单项（新建、设置、上下文、溢出菜单） | 通过 `ButtonStyle` 读取公共操作悬停色并应用全局透明度 | 键盘高亮与鼠标悬停一致；已有选中标记保留 |
| `ghost` 图标按钮 | 公共操作悬停色 | 按下用公共操作按下色；切换选中仍保留选中语义 |
| 编辑器工具栏、搜索栏、标题栏与编辑器标题操作 | 通过 `ButtonStyle::background_color` 读取公共操作悬停色并应用全局透明度 | 保留各组件现有的业务状态与命中区域 |
| 导航、目录树、文档卡片 | 导航悬停色 | 保留列表选中和导航选中语义 |
| 无边框文本框的编辑焦点 | 原有输入焦点底色 | 不属于操作按钮悬停，不迁移 |
| 危险操作 | 危险操作专用色 | 保留危险色及文字对比度要求 |

- 菜单展开是触发器的持续操作反馈，不能用内容选中强调色冒充悬停色。
- 工具条高亮保留外框内缩，菜单高亮保留项目内缩；两者共享颜色语义，几何随容器角色变化。半透明场景只保证高亮令牌及其透明度一致，不承诺不同底板上的最终合成像素相同；菜单底板及文字仍沿用现有绘制机制。
- 回归验证覆盖普通操作与选中强调色解耦、按钮与菜单高亮绘制指令的 RGBA 一致、内嵌操作一致性，以及深浅主题、1×/1.5×/2× DPI、全局透明度。

## 文档卡片横向布局

- 卡片水平内边距 8px、图标占位 18px、图文间距 6px，实际图标维持 14px。
- 标题、摘要、修改时间共享左对齐线，距卡片左边缘 32px，比原布局减少 14px。
- 标题可用宽度、换行高度和关闭按钮预留使用同一组布局常量计算，窄卡片仍保持标题、摘要、时间互不重叠。
