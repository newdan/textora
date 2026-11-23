# baoyu-design — 设计相关 skill 安装记录

来源: https://github.com/JimLiu/baoyu-design
Commit: 0c257e5e9def6723fdc69a95ec59fc8c414a8282 (v2.6.1, 2026-11-01)
安装位置: `.dsh/skills/`（工作区副本优先级高于 npm 插件副本）

| 目录 | Skill |
|---|---|
| `claude-design/` | 画布式设计稿：多 artboard 并排、可交互原型 |
| `claude-design-system/` | 设计系统：tokens、组件预览、字体、品牌手册 |
| `dataviz/` | 画任何图表前的选型/配色规范 + 调色板校验脚本 |

未安装（如需要再补）: `claude-slides`（幻灯片/导出 PPTX）、`claude-dashboard`（数据看板）。

## 使用

直接说需求，或用 `/` 选 `claude-design` / `claude-design-system`。产物默认落在工作区 `designs/<slug>/`，
随后会给出一个 `http://127.0.0.1:` 的本地编辑器链接。

## 更新 / 卸载

```bash
# 更新
cd /Users/dan/proj/llmws/textora/textora
rm -rf .dsh/skills.tmp && git clone --depth 1 https://github.com/JimLiu/baoyu-design.git .dsh/skills.tmp
for s in claude-design claude-design-system dataviz; do rm -rf .dsh/skills/$s; cp -a .dsh/skills.tmp/skills/$s .dsh/skills/; done
rm -rf .dsh/skills.tmp

# 卸载
rm -rf /Users/dan/proj/llmws/textora/textora/.dsh/skills
```

更新后需新开一个会话才会重新加载 skill。
