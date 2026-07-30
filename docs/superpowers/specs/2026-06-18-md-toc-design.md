# MD Preview Table of Contents — Design

## Overview

Add a table of contents panel for markdown preview. Toggle via TitleBar button or `Cmd+Shift+T`. Panel appears as a Dock Left child, showing heading hierarchy with indentation. Click to jump; scroll tracking highlights current heading.

## Architecture

### Data Flow

```
MarkdownDoc (BlockKind::Heading { level })
  → MarkdownPreview::collect_headings() → Vec<HeadingEntry>
  → UiShell (TocInput per frame)
  → TocWidget (Dock Left child, render + click)
```

### Components

| Component | File | Role |
|---|---|---|
| `HeadingEntry` | `crates/markdown/src/builder.rs` | text, level, block_idx, y_offset |
| `MarkdownPreview` extensions | `crates/app/src/md_preview.rs` | `headings()`, `current_heading_index(scroll_y)`, `scroll_to_heading(index)` |
| `TocWidget` | `crates/ui/src/widgets/toc.rs` (new) | Renders hierarchical list, handles click/hover/scroll |
| TitleBar toggle | `crates/ui/src/widgets/title_bar.rs` | New toggle button + `ToggleToc` action |
| `TocInput` | `crates/ui/src/widgets/toc.rs` | headings, active_index, visible |
| UiShell integration | `crates/app/src/ui_shell.rs` | Dock child insertion, event routing |
| App dispatch | `crates/app/src/app_dispatch.rs` | `ToggleToc` handler, keyboard shortcut |
| Settings | settings JSON | `toc.max_depth` (default 3), `toc.width` (default 200px) |

### Dock Layout

TOC visible → inserted as `DockChild::Left`, thickness = `toc.width`. Inserted after existing Sidebar so Sidebar remains outermost. TOC hidden → no Dock child. This means toggling the TOC triggers a `rebuild_dock_children()`.

## Interaction Spec

| Action | Behavior |
|---|---|
| Toggle on | Click TitleBar TOC icon, or `Cmd+Shift+T` |
| Toggle off | Click icon/shortcut again (no auto-close on preview click) |
| Click heading | Preview scrolls to heading Y (top-aligned) |
| Scroll preview | TOC auto-highlights nearest visible heading via `current_heading_index()` |
| TOC panel scroll | If active heading is off-screen, scroll TOC to reveal it |
| No headings | TOC shows empty state message |
| Hover heading | Background brightens slightly |

## Visual Spec

- **Indent**: 16px per level (h1=0, h2=16, h3=32, etc.)
- **Font**: 12pt, unified across all levels
- **Color**: h1 accent heading color, h2-h3 body color (theme-aware)
- **Active highlight**: 2px left vertical bar (accent color) + bold text
- **Hover**: subtle background brighten
- **Empty state**: "No headings" in dimmed color, centered in panel
- **Truncation**: heading text longer than panel width truncated with ellipsis
- **Panel bg**: slightly darker than content area (sidebar-equivalent)

## Settings

```json
{
  "toc": {
    "max_depth": 3,
    "width": 200
  }
}
```

## Key Implementation Notes

- Headings extracted from `LazyLayout` after layout completes — cheap because block types are already known
- `current_heading_index(scroll_y)`: binary search over `Vec<HeadingEntry>` by y_offset, O(log n)
- `scroll_to_heading(index)`: set `scroll_y` directly from cached y_offset, clamp to content height
- TOC toggle triggers Dock rebuild → redraw
- TocWidget uses `paint()` for list rendering and `on_event()` for click/hover handling, consistent with existing widget patterns
