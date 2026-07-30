# 依赖重复管理政策

## 原则

1. **直接依赖集中管理** — 所有直接依赖必须声明在 `[workspace.dependencies]` 中，workspace 成员通过 `dep.workspace = true` 引用。
2. **同 major 优先统一** — 当同一 crate 存在多个相同 major 版本时，优先升级到最新 patch/minor，统一为单一版本。
3. **不同 major 传递重复必须记录** — 当上游传递依赖引入不同 major 版本时，不得强推 `[patch]` 或 `[replace]` 来消歧义，必须记录来源和预期淘汰路径。
4. **禁止不兼容 `[patch]`** — 禁止仅为消除 `cargo tree -d` 输出而使用不兼容的 `[patch]` 替换。这会掩盖真实冲突，破坏上游语义。

## 重复基线（2026-06-21 记录）

以下重复来自上游传递依赖，短期内不可消除，记录于此作为基线。

### `objc2` 0.5.2 / 0.6.4

- **0.5.2 来源**: `winit 0.30.13` → `objc2-app-kit 0.2.2` → `block2 0.5.1` → `objc2 0.5.2`
- **0.6.4 来源**: `wgpu-hal 29.0.3` (zed-industries fork) → `objc2-metal 0.3.2` → `objc2-foundation 0.3.2` → `objc2 0.6.4`；以及 `arboard 3.6.1`、`rfd 0.15.4` → `objc2-app-kit 0.3.2` → `objc2 0.6.4`
- **根因**: `winit 0.30` 尚未升级到 `objc2 0.6`，而 `wgpu` (zed fork) 和 `arboard`/`rfd` 已使用 `objc2 0.6`。
- **预期淘汰路径**: 等待 `winit` 发布支持 `objc2 0.6` 的版本，届时统一为单一版本。
- **复查日期**: 2026-07-19

### `ttf-parser` 0.20.0 / 0.21.1

- **0.20.0 来源**: `cosmic-text 0.12.1` → `fontdb 0.16.2` → `ttf-parser 0.20.0`
- **0.21.1 来源**: `cosmic-text 0.12.1` → `rustybuzz 0.14.1` → `ttf-parser 0.21.1`；以及 `edit-plus-shaping` 直接依赖
- **根因**: `cosmic-text 0.12.1` 内部同时依赖 `fontdb 0.16`（使用 `ttf-parser 0.20`）和 `rustybuzz 0.14`（使用 `ttf-parser 0.21`），两个子依赖的版本选择不一致。
- **预期淘汰路径**: 等待 `cosmic-text` 或 `fontdb` 发布新版本统一 `ttf-parser` 依赖，或 `cosmic-text` 升级到更新的 `fontdb`。
- **复查日期**: 2026-07-19

## 复查流程

每月运行 `./scripts/dependency-report.sh` 并对照本基线：

1. 若上游已修复，更新本文件并移除已解决条目。
2. 若出现新重复，按上述原则决定是否记录或修复。
3. 复查日期到期时，评估是否有可行的统一路径。

## 复查提醒

> **2026-07-19 复查已记录。** 请在到期前运行 `./scripts/dependency-report.sh` 并对照上方基线评估。
> 如项目管理工具有 TODO 板，建议同步添加对应条目。
