# TextBox Widget Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract a reusable TextBox component from SearchBarWidget's duplicated find/replace text input logic, with full IME encapsulation, selection, and clipboard support.

**Architecture:** TextBox is a non-Widget-trait component that owns text state, cursor, selection, and IME. It communicates with the parent via struct-level callbacks. IME events flow through the standard widget Event enum, not through direct app→TextBox calls.

**Tech Stack:** Rust, existing ui crate widget framework (DrawList, PaintCtx, LayoutCtx, TextMeasure)

---

## File Map

| File | Action | Responsibility |
|------|--------|---------------|
| `crates/ui/src/core/widget.rs` | Modify | Add `Modifiers`, IME variants to `Event`, update `KeyDown` |
| `crates/ui/src/core/dock.rs` | Modify | Handle new `Event::Ime*` variants in dispatch |
| `crates/ui/src/widgets/text_box.rs` | **Create** | TextBox struct, all methods, tests |
| `crates/ui/src/widgets/mod.rs` | Modify | Export `text_box` module |
| `crates/ui/src/widgets/search_bar.rs` | Modify | Replace inline text input with TextBox instances |
| `crates/app/src/ui_shell.rs` | Modify | Wire callbacks, IME cursor area, forward_key signature |
| `crates/app/src/app_lifecycle.rs` | Modify | Wrap winit IME events as `Event::Ime*` |
| `crates/app/src/app_renderer.rs` | Modify | Remove search bar preedit GPU vertex rendering |
| `crates/app/src/app_window.rs` | Modify | Update IME cursor area to read from TextBox |
| `crates/app/src/app.rs` | Modify | Remove `preedit_text`, `preedit_cursor`, `preedit_advance_px` fields |

---

### Task 1: Add Modifiers type and IME variants to Event

**Files:**
- Modify: `crates/ui/src/core/widget.rs`
- Modify: `crates/ui/src/core/dock.rs`
- Modify: `crates/ui/src/widgets/search_bar.rs`
- Modify: `crates/ui/src/widgets/popup_menu/mod.rs`
- Modify: `crates/ui/src/widgets/title_bar.rs`
- Modify: `crates/app/src/ui_shell.rs`

- [ ] **Step 1: Add Modifiers struct after KeyCode enum**

In `crates/ui/src/core/widget.rs`, after `KeyCode` (line 48), add:

```rust
/// Keyboard modifier flags.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub cmd: bool,
    pub alt: bool,
    pub ctrl: bool,
}

impl Modifiers {
    pub const NONE: Self = Modifiers { shift: false, cmd: false, alt: false, ctrl: false };
}
```

- [ ] **Step 2: Add IME variants to Event enum**

Replace the current `Event` enum:

```rust
/// 输入事件
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    MouseMove { px: f32, py: f32 },
    MouseDown { px: f32, py: f32, button: MouseButton },
    MouseUp { px: f32, py: f32, button: MouseButton },
    Wheel { dx: f32, dy: f32, px: f32, py: f32 },
    KeyDown(KeyCode, Modifiers),
    ImePreedit { text: String, cursor: Option<(usize, usize)> },
    ImeCommit(String),
    ImeEnable,
    ImeDisable,
}
```

- [ ] **Step 3: Update dock.rs dispatch to handle new IME variants**

In `crates/ui/src/core/dock.rs`, find the `dispatch` method (around line 241) that clones events. Update the match to handle new variants:

```rust
// The clone block (approx line 254) — update KeyDown pattern:
Event::KeyDown(ref kc, ref m) => Event::KeyDown(kc.clone(), *m),

// Add IME pass-through (same cloning block):
Event::ImePreedit { ref text, cursor } => Event::ImePreedit { text: text.clone(), cursor: *cursor },
Event::ImeCommit(ref s) => Event::ImeCommit(s.clone()),
Event::ImeEnable => Event::ImeEnable,
Event::ImeDisable => Event::ImeDisable,
```

- [ ] **Step 4: Update all existing Event::KeyDown pattern matches**

`crates/ui/src/widgets/search_bar.rs:162` — change `Event::KeyDown(kc)` to `Event::KeyDown(kc, _modifiers)`

`crates/ui/src/widgets/popup_menu/mod.rs:99` — change `Event::KeyDown(KeyCode::Escape)` to `Event::KeyDown(KeyCode::Escape, _)`

`crates/app/src/ui_shell.rs:364` — change `Event::KeyDown(key)` to `Event::KeyDown(key, Modifiers::NONE)`

`crates/ui/src/widgets/title_bar.rs:423` — change `Event::KeyDown(KeyCode::Char('a'))` to `Event::KeyDown(KeyCode::Char('a'), Modifiers::NONE)`

All search_bar test files (`crates/ui/src/widgets/search_bar.rs` lines 822, 831, 841, 851, 861, 871, 881, 883) — wrap KeyDown with `Modifiers::NONE` second arg.

- [ ] **Step 5: Export new types from ui::core**

In `crates/ui/src/core/mod.rs`, add `Modifiers` to the exports:

```rust
pub use widget::{Widget, WidgetId, LayoutCtx, PaintCtx, EventCtx, Event, MouseButton, KeyCode, Modifiers, WidgetAction};
```

- [ ] **Step 6: Build and verify**

Run: `cargo build -p ui 2>&1`
Expected: Compiles without errors after all pattern matches are updated.

- [ ] **Step 7: Run existing tests**

Run: `cargo test -p ui 2>&1`
Expected: All existing tests pass.

- [ ] **Step 8: Commit**

```bash
git add crates/ui/src/core/widget.rs crates/ui/src/core/dock.rs crates/ui/src/core/mod.rs \
        crates/ui/src/widgets/search_bar.rs crates/ui/src/widgets/popup_menu/mod.rs \
        crates/ui/src/widgets/title_bar.rs crates/app/src/ui_shell.rs
git commit -m "feat: add Modifiers type and IME variants to core Event enum"
```

---

### Task 2: Create TextBox struct with text editing

**Files:**
- Create: `crates/ui/src/widgets/text_box.rs`

- [ ] **Step 1: Create text_box.rs with struct and basic types**

```rust
//! TextBox — single-line text input component.
//! Manages text state, cursor, selection, IME preedit, and clipboard callbacks.

use crate::core::{Rect, LayoutCtx, PaintCtx, KeyCode, Modifiers};

/// IME event type — received by TextBox from the parent widget.
#[derive(Clone, Debug)]
pub enum TextBoxIme {
    Preedit { text: String, cursor: Option<(usize, usize)> },
    Commit(String),
    Enabled,
    Disabled,
}

/// Callback type aliases (using String to avoid lifetime issues in callbacks)
pub type ChangeCb = Box<dyn Fn(String)>;
pub type NotifyCb = Box<dyn Fn()>;
pub type FocusCb = Box<dyn Fn(bool)>;
pub type ClipboardGetCb = Box<dyn Fn() -> String>;
pub type ClipboardSetCb = Box<dyn Fn(String)>;

pub struct TextBox {
    rect: Rect,

    // Text state
    text: String,
    cursor_byte: usize,

    // Selection: (anchor_byte, cursor_byte), no ordering guarantee. None = no selection.
    selection: Option<(usize, usize)>,

    // IME
    preedit: String,
    preedit_cursor: Option<(usize, usize)>,

    // Visual
    placeholder: String,
    blink_on: bool,
    focused: bool,

    // Mouse drag
    dragging: bool,

    // Layout cache
    cursor_x: f32,
    preedit_width: f32,

    // Callbacks
    pub on_changed: Option<ChangeCb>,
    pub on_enter: Option<NotifyCb>,
    pub on_escape: Option<NotifyCb>,
    pub on_focus: Option<FocusCb>,
    pub on_copy: Option<ClipboardSetCb>,
    pub on_cut: Option<ClipboardSetCb>,
    pub on_paste: Option<ClipboardGetCb>,
}

impl TextBox {
    pub fn new() -> Self {
        Self {
            rect: Rect::ZERO,
            text: String::new(),
            cursor_byte: 0,
            selection: None,
            preedit: String::new(),
            preedit_cursor: None,
            placeholder: String::new(),
            blink_on: false,
            focused: false,
            dragging: false,
            cursor_x: 0.0,
            preedit_width: 0.0,
            on_changed: None,
            on_enter: None,
            on_escape: None,
            on_focus: None,
            on_copy: None,
            on_cut: None,
            on_paste: None,
        }
    }

    // ── Accessors ──

    pub fn text(&self) -> &str { &self.text }
    pub fn cursor_byte(&self) -> usize { self.cursor_byte }
    pub fn is_focused(&self) -> bool { self.focused }

    pub fn set_text(&mut self, text: &str) {
        self.text = String::from(text);
        self.cursor_byte = self.text.len();
        self.selection = None;
    }

    pub fn set_placeholder(&mut self, s: &str) {
        self.placeholder = String::from(s);
    }

    pub fn set_blink(&mut self, on: bool) {
        self.blink_on = on;
    }

    pub fn set_focus(&mut self, focused: bool) {
        if self.focused != focused {
            self.focused = focused;
            if !focused {
                self.selection = None;
                self.dragging = false;
            }
            if let Some(ref cb) = self.on_focus {
                cb(focused);
            }
        }
    }

    pub fn select_all(&mut self) {
        if !self.text.is_empty() {
            self.selection = Some((0, self.text.len()));
            self.cursor_byte = self.text.len();
        }
    }

    pub fn selection_text(&self) -> Option<&str> {
        self.selection.map(|(a, b)| {
            let start = a.min(b);
            let end = a.max(b);
            &self.text[start..end]
        })
    }

    // ── Helpers ──

    /// Find the char boundary before `pos` (for Backspace).
    fn prev_char_boundary(s: &str, pos: usize) -> usize {
        if pos == 0 { return 0; }
        let mut prev = pos - 1;
        while prev > 0 && !s.is_char_boundary(prev) {
            prev -= 1;
        }
        prev
    }

    /// Find the char boundary after `pos` (for Right arrow).
    fn next_char_boundary(s: &str, pos: usize) -> usize {
        if pos >= s.len() { return s.len(); }
        let mut next = pos + 1;
        while next < s.len() && !s.is_char_boundary(next) {
            next += 1;
        }
        next
    }

    /// Delete selected range and return true if something was deleted.
    fn delete_selection(&mut self) -> bool {
        if let Some((a, b)) = self.selection.take() {
            let start = a.min(b);
            let end = a.max(b);
            self.text.replace_range(start..end, "");
            self.cursor_byte = start;
            self.fire_changed();
            true
        } else {
            false
        }
    }

    fn fire_changed(&mut self) {
        if let Some(ref cb) = self.on_changed {
            cb(self.text.clone());
        }
    }
}
```

- [ ] **Step 2: Implement on_key with InsertChar and Backspace**

Add to `impl TextBox`:

```rust
/// Process a key event. Returns true if the event was consumed.
pub fn on_key(&mut self, kc: KeyCode, modifiers: Modifiers) -> bool {
    match kc {
        KeyCode::Char(c) => {
            self.delete_selection();
            let pos = self.cursor_byte;
            self.text.insert(pos, c);
            self.cursor_byte = Self::next_char_boundary(&self.text, pos);
            self.fire_changed();
            true
        }
        KeyCode::Backspace => {
            if !self.delete_selection() {
                if self.cursor_byte > 0 {
                    let prev = Self::prev_char_boundary(&self.text, self.cursor_byte);
                    self.text.replace_range(prev..self.cursor_byte, "");
                    self.cursor_byte = prev;
                    self.fire_changed();
                }
            }
            true
        }
        _ => false,
    }
}
```

- [ ] **Step 3: Write text editing tests**

Add at the bottom of `text_box.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_textbox_is_empty() {
        let tb = TextBox::new();
        assert_eq!(tb.text(), "");
        assert_eq!(tb.cursor_byte(), 0);
        assert!(tb.selection.is_none());
    }

    #[test]
    fn insert_char() {
        let mut tb = TextBox::new();
        tb.on_key(KeyCode::Char('a'), Modifiers::NONE);
        assert_eq!(tb.text(), "a");
        assert_eq!(tb.cursor_byte(), 1);
        tb.on_key(KeyCode::Char('b'), Modifiers::NONE);
        assert_eq!(tb.text(), "ab");
        assert_eq!(tb.cursor_byte(), 2);
    }

    #[test]
    fn backspace() {
        let mut tb = TextBox::new();
        tb.set_text("hello");
        tb.on_key(KeyCode::Backspace, Modifiers::NONE);
        assert_eq!(tb.text(), "hell");
        assert_eq!(tb.cursor_byte(), 4);
    }

    #[test]
    fn backspace_at_start_does_nothing() {
        let mut tb = TextBox::new();
        tb.set_text("a");
        tb.cursor_byte = 0;
        tb.on_key(KeyCode::Backspace, Modifiers::NONE);
        assert_eq!(tb.text(), "a");
        assert_eq!(tb.cursor_byte(), 0);
    }

    #[test]
    fn backspace_utf8_char() {
        let mut tb = TextBox::new();
        tb.set_text("ab中d");
        // cursor is at end (byte 6 for "ab中d")
        tb.on_key(KeyCode::Backspace, Modifiers::NONE);
        assert_eq!(tb.text(), "ab中");
        assert_eq!(tb.cursor_byte(), 5); // "中" is 3 bytes in UTF-8
    }

    #[test]
    fn set_text_resets_cursor_and_selection() {
        let mut tb = TextBox::new();
        tb.selection = Some((0, 3));
        tb.set_text("new");
        assert_eq!(tb.text(), "new");
        assert_eq!(tb.cursor_byte(), 3);
        assert!(tb.selection.is_none());
    }

    #[test]
    fn selection_text_returns_correct_slice() {
        let mut tb = TextBox::new();
        tb.set_text("hello world");
        tb.selection = Some((0, 5));
        assert_eq!(tb.selection_text(), Some("hello"));
        // Order doesn't matter
        tb.selection = Some((11, 6));
        assert_eq!(tb.selection_text(), Some("world"));
    }

    #[test]
    fn delete_selection_replaces_on_insert() {
        let mut tb = TextBox::new();
        tb.set_text("hello");
        tb.selection = Some((0, 5));
        tb.on_key(KeyCode::Char('x'), Modifiers::NONE);
        assert_eq!(tb.text(), "x");
        assert_eq!(tb.cursor_byte(), 1);
        assert!(tb.selection.is_none());
    }

    #[test]
    fn backspace_deletes_selection() {
        let mut tb = TextBox::new();
        tb.set_text("hello");
        tb.selection = Some((0, 3));
        tb.on_key(KeyCode::Backspace, Modifiers::NONE);
        assert_eq!(tb.text(), "lo");
        assert_eq!(tb.cursor_byte(), 0);
        assert!(tb.selection.is_none());
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p ui text_box 2>&1`
Expected: 8 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/src/widgets/text_box.rs
git commit -m "feat: add TextBox struct with text editing (insert/backspace)"
```

---

### Task 3: Add cursor movement and selection

**Files:**
- Modify: `crates/ui/src/widgets/text_box.rs`

- [ ] **Step 1: Add cursor movement to on_key**

Replace the `_ => false` arm in `on_key` with:

```rust
KeyCode::Left => {
    if modifiers.shift {
        // Extend/start selection
        if self.selection.is_none() {
            self.selection = Some((self.cursor_byte, self.cursor_byte));
        }
        let new_cursor = Self::prev_char_boundary(&self.text, self.cursor_byte);
        self.cursor_byte = new_cursor;
        // Update selection cursor end
        if let Some((anchor, _)) = self.selection {
            self.selection = Some((anchor, new_cursor));
        }
    } else {
        if self.selection.is_some() {
            // Collapse to the left edge of selection
            let (a, b) = self.selection.take().unwrap();
            self.cursor_byte = a.min(b);
        } else {
            self.cursor_byte = Self::prev_char_boundary(&self.text, self.cursor_byte);
        }
    }
    true
}
KeyCode::Right => {
    if modifiers.shift {
        if self.selection.is_none() {
            self.selection = Some((self.cursor_byte, self.cursor_byte));
        }
        let new_cursor = Self::next_char_boundary(&self.text, self.cursor_byte);
        self.cursor_byte = new_cursor;
        if let Some((anchor, _)) = self.selection {
            self.selection = Some((anchor, new_cursor));
        }
    } else {
        if self.selection.is_some() {
            let (a, b) = self.selection.take().unwrap();
            self.cursor_byte = a.max(b);
        } else {
            self.cursor_byte = Self::next_char_boundary(&self.text, self.cursor_byte);
        }
    }
    true
}
KeyCode::Home => {
    self.cursor_byte = if modifiers.shift {
        if self.selection.is_none() {
            self.selection = Some((self.cursor_byte, 0));
        } else {
            let (anchor, _) = self.selection.unwrap();
            self.selection = Some((anchor, 0));
        }
        0
    } else {
        self.selection = None;
        0
    };
    true
}
KeyCode::End => {
    let end = self.text.len();
    self.cursor_byte = if modifiers.shift {
        if self.selection.is_none() {
            self.selection = Some((self.cursor_byte, end));
        } else {
            let (anchor, _) = self.selection.unwrap();
            self.selection = Some((anchor, end));
        }
        end
    } else {
        self.selection = None;
        end
    };
    true
}
KeyCode::Char(_) if modifiers.cmd => {
    // Cmd+A will be handled by app layer keybinding; Cmd+C/X/V handled below
    false
}
_ => false,
```

- [ ] **Step 2: Add Cmd+A handling before the cmd catch**

In the `Char(c)` arm of on_key, add Cmd+A handling before the regular Char insertion:

Actually, update the `Char(c)` arm to handle modifiers:
```rust
KeyCode::Char(c) => {
    if modifiers.cmd {
        match c {
            'a' | 'A' => {
                self.select_all();
                return true;
            }
            'c' | 'C' => {
                if let Some(text) = self.selection_text() {
                    if let Some(ref cb) = self.on_copy {
                        cb(text.to_string());
                    }
                }
                return true;
            }
            'x' | 'X' => {
                if let Some(text) = self.selection_text() {
                    if let Some(ref cb) = self.on_cut {
                        cb(text.to_string());
                    }
                }
                self.delete_selection();
                return true;
            }
            'v' | 'V' => {
                if let Some(ref cb) = self.on_paste {
                    let pasted = cb();
                    if !pasted.is_empty() {
                        self.delete_selection();
                        let pos = self.cursor_byte;
                        self.text.insert_str(pos, &pasted);
                        self.cursor_byte += pasted.len();
                        self.fire_changed();
                    }
                }
                return true;
            }
            _ => return false,
        }
    }
    // Regular char insertion
    self.delete_selection();
    let pos = self.cursor_byte;
    self.text.insert(pos, c);
    self.cursor_byte = Self::next_char_boundary(&self.text, pos);
    self.fire_changed();
    true
}
```

- [ ] **Step 3: Add mouse handling methods**

```rust
/// Mouse down: position cursor, clear selection, begin drag.
pub fn on_mouse_down(&mut self, px: f32, py: f32) -> bool {
    if !self.rect.contains(px, py) {
        return false;
    }
    self.selection = None;
    self.dragging = true;
    // In a real implementation, px maps to a byte offset via layout cache.
    // For now, approximate: cursor goes to nearest position.
    // The actual byte-to-pixel mapping happens during layout.
    true
}

/// Mouse drag: extend selection.
pub fn on_mouse_drag(&mut self, px: f32, py: f32) {
    if !self.dragging { return; }
    if self.selection.is_none() {
        self.selection = Some((self.cursor_byte, self.cursor_byte));
    }
    // px → byte offset, update cursor_byte and selection end
}

/// Mouse up: end drag.
pub fn on_mouse_up(&mut self) {
    self.dragging = false;
}
```

Note: Precise mouse-to-byte mapping requires pixel measurements from layout. For Task 3, mouse handling is a skeleton. Full click-to-position is implemented in Task 6 after layout.

- [ ] **Step 4: Write cursor movement and selection tests**

```rust
#[test]
fn cursor_left_right() {
    let mut tb = TextBox::new();
    tb.set_text("abc");
    tb.cursor_byte = 0;
    tb.on_key(KeyCode::Right, Modifiers::NONE);
    assert_eq!(tb.cursor_byte(), 1);
    tb.on_key(KeyCode::Right, Modifiers::NONE);
    assert_eq!(tb.cursor_byte(), 2);
    tb.on_key(KeyCode::Left, Modifiers::NONE);
    assert_eq!(tb.cursor_byte(), 1);
}

#[test]
fn cursor_home_end() {
    let mut tb = TextBox::new();
    tb.set_text("abc");
    tb.cursor_byte = 1;
    tb.on_key(KeyCode::Home, Modifiers::NONE);
    assert_eq!(tb.cursor_byte(), 0);
    tb.on_key(KeyCode::End, Modifiers::NONE);
    assert_eq!(tb.cursor_byte(), 3);
}

#[test]
fn shift_right_creates_selection() {
    let mut tb = TextBox::new();
    tb.set_text("abc");
    tb.cursor_byte = 0;
    let shift = Modifiers { shift: true, ..Modifiers::NONE };
    tb.on_key(KeyCode::Right, shift);
    assert!(tb.selection.is_some());
    let (anchor, cursor) = tb.selection.unwrap();
    assert_eq!(anchor, 0);
    assert_eq!(cursor, 1);
    assert_eq!(tb.cursor_byte(), 1);
}

#[test]
fn shift_left_creates_selection() {
    let mut tb = TextBox::new();
    tb.set_text("abc");
    tb.cursor_byte = 3;
    let shift = Modifiers { shift: true, ..Modifiers::NONE };
    tb.on_key(KeyCode::Left, shift);
    assert!(tb.selection.is_some());
}

#[test]
fn left_collapses_selection() {
    let mut tb = TextBox::new();
    tb.set_text("hello");
    tb.selection = Some((0, 5));
    tb.on_key(KeyCode::Left, Modifiers::NONE);
    assert!(tb.selection.is_none());
    assert_eq!(tb.cursor_byte(), 0);
}

#[test]
fn right_collapses_selection() {
    let mut tb = TextBox::new();
    tb.set_text("hello");
    tb.selection = Some((0, 5));
    tb.on_key(KeyCode::Right, Modifiers::NONE);
    assert!(tb.selection.is_none());
    assert_eq!(tb.cursor_byte(), 5);
}

#[test]
fn select_all() {
    let mut tb = TextBox::new();
    tb.set_text("hello");
    tb.select_all();
    assert_eq!(tb.selection, Some((0, 5)));
    assert_eq!(tb.selection_text(), Some("hello"));
}

#[test]
fn selection_text_utf8() {
    let mut tb = TextBox::new();
    tb.set_text("a中b");
    tb.selection = Some((1, 4)); // "中" in UTF-8: a=byte0, 中=bytes 1-3, b=byte4
    assert_eq!(tb.selection_text(), Some("中"));
}

#[test]
fn set_focus_clears_selection() {
    let mut tb = TextBox::new();
    tb.set_text("hello");
    tb.selection = Some((0, 3));
    tb.set_focus(false);
    assert!(tb.selection.is_none());
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p ui text_box 2>&1`
Expected: All tests pass (8 old + new).

- [ ] **Step 6: Commit**

```bash
git add crates/ui/src/widgets/text_box.rs
git commit -m "feat: add cursor movement, selection, Cmd+A/C/V/X to TextBox"
```

---

### Task 4: Add IME handling

**Files:**
- Modify: `crates/ui/src/widgets/text_box.rs`

- [ ] **Step 1: Add IME methods**

```rust
/// Receive an IME event from the parent widget.
pub fn on_ime(&mut self, ev: &TextBoxIme) {
    match ev {
        TextBoxIme::Preedit { text, cursor } => {
            self.preedit = text.clone();
            self.preedit_cursor = *cursor;
        }
        TextBoxIme::Commit(text) => {
            self.preedit.clear();
            self.preedit_cursor = None;
            if !text.is_empty() {
                self.delete_selection();
                let pos = self.cursor_byte;
                self.text.insert_str(pos, text);
                self.cursor_byte = pos + text.len();
                self.fire_changed();
            }
        }
        TextBoxIme::Enabled | TextBoxIme::Disabled => {
            self.preedit.clear();
            self.preedit_cursor = None;
        }
    }
}

/// Pixel rect where the OS IME candidate window should appear.
pub fn ime_cursor_rect(&self) -> Rect {
    let cursor_h = self.rect.h * 0.6;
    let cursor_y = self.rect.y + (self.rect.h - cursor_h) * 0.5;
    Rect::new(self.rect.x + self.cursor_x, cursor_y, 2.0, cursor_h)
}

pub fn has_preedit(&self) -> bool {
    !self.preedit.is_empty()
}
```

- [ ] **Step 2: Write IME tests**

```rust
#[test]
fn ime_preedit_stored() {
    let mut tb = TextBox::new();
    tb.on_ime(&TextBoxIme::Preedit {
        text: "pinyin".into(),
        cursor: Some((0, 6)),
    });
    assert_eq!(tb.preedit, "pinyin");
    assert!(tb.has_preedit());
}

#[test]
fn ime_commit_inserts_text() {
    let mut tb = TextBox::new();
    tb.set_text("hello ");
    // Simulate IME: preedit then commit
    tb.on_ime(&TextBoxIme::Preedit {
        text: "世界".into(),
        cursor: Some((0, 6)),
    });
    tb.on_ime(&TextBoxIme::Commit("世界".into()));
    assert_eq!(tb.text(), "hello 世界");
    assert!(tb.preedit.is_empty());
    assert!(!tb.has_preedit());
}

#[test]
fn ime_commit_replaces_selection() {
    let mut tb = TextBox::new();
    tb.set_text("hello world");
    tb.selection = Some((6, 11));
    tb.on_ime(&TextBoxIme::Commit("earth".into()));
    assert_eq!(tb.text(), "hello earth");
    assert!(tb.selection.is_none());
}

#[test]
fn ime_enabled_clears_preedit() {
    let mut tb = TextBox::new();
    tb.on_ime(&TextBoxIme::Preedit {
        text: "composing".into(),
        cursor: Some((0, 9)),
    });
    tb.on_ime(&TextBoxIme::Enabled);
    assert!(tb.preedit.is_empty());
    assert!(!tb.has_preedit());
}

#[test]
fn ime_disabled_clears_preedit() {
    let mut tb = TextBox::new();
    tb.on_ime(&TextBoxIme::Preedit {
        text: "composing".into(),
        cursor: Some((0, 9)),
    });
    tb.on_ime(&TextBoxIme::Disabled);
    assert!(tb.preedit.is_empty());
}

#[test]
fn ime_cursor_rect_uses_layout_cache() {
    let mut tb = TextBox::new();
    tb.rect = Rect::new(10.0, 0.0, 200.0, 28.0);
    tb.cursor_x = 50.0;
    let r = tb.ime_cursor_rect();
    assert!(r.x > 10.0);
    assert!(r.w > 0.0);
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p ui text_box 2>&1`
Expected: All tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/ui/src/widgets/text_box.rs
git commit -m "feat: add IME preedit/commit handling to TextBox"
```

---

### Task 5: Add layout and paint

**Files:**
- Modify: `crates/ui/src/widgets/text_box.rs`

- [ ] **Step 1: Add layout method**

```rust
use crate::core::measure::TextMeasure;

/// Compute layout: measure text widths for cursor positioning.
/// Called by parent during set_rect / layout phase.
pub fn layout(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
    self.rect = rect;
    let font_size = 14.0 * ctx.dpi;

    // Measure text up to cursor for cursor_x
    let measure = ctx.ui_measure.as_mut().unwrap_or(&mut ctx.measure);
    self.cursor_x = measure.measure(&self.text[..self.cursor_byte], font_size);

    // Measure preedit text width
    if !self.preedit.is_empty() {
        self.preedit_width = measure.measure(&self.preedit, font_size);
    } else {
        self.preedit_width = 0.0;
    }
}
```

- [ ] **Step 2: Add paint method**

```rust
/// Paint the text input: background, border, text/placeholder, selection, cursor, preedit.
pub fn paint(&self, ctx: &mut PaintCtx) {
    if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
        return;
    }
    let dpi = ctx.dpi;
    let font_size = 14.0 * dpi;
    let baseline = self.rect.y + self.rect.h * 0.5 + font_size * 0.35;
    let corner_radius = 3.0 * dpi;

    // 1. Background
    let bg = {
        let mut c = ctx.theme.search_bar_bg;
        c[0] = (c[0] + 0.03).min(1.0);
        c[1] = (c[1] + 0.03).min(1.0);
        c[2] = (c[2] + 0.03).min(1.0);
        c
    };
    ctx.list.fill_rounded(self.rect, bg, corner_radius);

    // 2. Border
    let border_color = if self.focused {
        ctx.theme.sidebar_accent
    } else {
        ctx.theme.search_bar_border
    };
    ctx.list.fill_rounded(self.rect, border_color, corner_radius);

    // 3. Selection highlight — skipped for v1 (requires pixel-to-byte mapping)

    // 4. Text or placeholder
    let text_x = self.rect.x + 4.0 * dpi;
    if !self.text.is_empty() {
        ctx.list.text(text_x, baseline, font_size, ctx.theme.search_bar_fg, &self.text);
    } else if self.preedit.is_empty() && !self.placeholder.is_empty() {
        let ph_color = {
            let mut c = ctx.theme.search_bar_fg;
            c[3] *= 0.4;
            c
        };
        ctx.list.text(text_x, baseline, font_size, ph_color, &self.placeholder);
    }

    // 5. IME preedit text + underline
    if !self.preedit.is_empty() {
        let preedit_x = text_x + self.cursor_x;
        ctx.list.text(preedit_x, baseline, font_size, ctx.theme.search_bar_fg, &self.preedit);
        // Underline
        let ul_y = baseline + 2.0 * dpi;
        let ul_h = 1.5 * dpi;
        let ul_w = self.preedit_width;
        ctx.list.fill(
            Rect::new(preedit_x, ul_y, ul_w, ul_h),
            ctx.theme.search_bar_fg,
        );
    }

    // 6. Cursor
    if self.blink_on && self.focused && self.selection.is_none() {
        let cursor_h = font_size;
        let cursor_w = 2.0 * dpi;
        let cursor_y = baseline - cursor_h * 0.75;
        let cursor_rect = Rect::new(text_x + self.cursor_x, cursor_y, cursor_w, cursor_h);
        ctx.list.fill(cursor_rect, ctx.theme.search_bar_fg);
    }
}
```

- [ ] **Step 3: Write layout and paint tests**

```rust
#[test]
fn layout_computes_cursor_x() {
    use crate::core::measure::NoopMeasure;
    use crate::theme::Theme;

    let mut tb = TextBox::new();
    tb.set_text("abc");
    tb.cursor_byte = 2;

    let theme = crate::theme::test_theme();
    let mut measure = NoopMeasure;
    let mut ctx = LayoutCtx {
        measure: &mut measure,
        ui_measure: None,
        theme: &theme,
        dpi: 1.0,
    };
    let rect = Rect::new(10.0, 0.0, 200.0, 28.0);
    tb.layout(rect, &mut ctx);
    assert_eq!(tb.rect.x, 10.0);
    // cursor_x is 0.0 with NoopMeasure since it returns 0 for everything
    assert_eq!(tb.cursor_x, 0.0);
}

#[test]
fn paint_empty_shows_placeholder() {
    use crate::core::paint::DrawList;
    use crate::theme::Theme;

    let mut tb = TextBox::new();
    tb.set_placeholder("Search...");
    tb.rect = Rect::new(0.0, 0.0, 200.0, 28.0);
    tb.blink_on = true;
    tb.focused = true;

    let theme = crate::theme::test_theme();
    let mut dl = DrawList::new();
    let mut pc = PaintCtx { global_alpha: 1.0, list: &mut dl, theme: &theme, dpi: 1.0, offset: (0.0, 0.0) };
    tb.paint(&mut pc);

    // Should have: bg, border, placeholder text, cursor
    assert!(dl.cmds.len() >= 4);
    // First cmd is bg fill_rounded
    // Second is border fill_rounded
    // Third is placeholder text
    // Fourth is cursor fill
    let texts: Vec<_> = dl.cmds.iter().filter(|c| matches!(c, DrawCmd::Text { .. })).collect();
    assert_eq!(texts.len(), 1, "expected 1 text (placeholder)");
}

#[test]
fn paint_shows_text_not_placeholder() {
    use crate::core::paint::DrawList;
    use crate::theme::Theme;

    let mut tb = TextBox::new();
    tb.set_text("hello");
    tb.set_placeholder("Search...");
    tb.rect = Rect::new(0.0, 0.0, 200.0, 28.0);

    let theme = crate::theme::test_theme();
    let mut dl = DrawList::new();
    let mut pc = PaintCtx { global_alpha: 1.0, list: &mut dl, theme: &theme, dpi: 1.0, offset: (0.0, 0.0) };
    tb.paint(&mut pc);

    let texts: Vec<_> = dl.cmds.iter().filter(|c| matches!(c, DrawCmd::Text { .. })).collect();
    assert_eq!(texts.len(), 1);
    if let DrawCmd::Text { content, .. } = &texts[0] {
        assert_eq!(content, "hello");
    }
}

#[test]
fn paint_hides_cursor_when_blink_off() {
    use crate::core::paint::DrawList;
    use crate::theme::Theme;

    let mut tb = TextBox::new();
    tb.set_text("x");
    tb.rect = Rect::new(0.0, 0.0, 200.0, 28.0);
    tb.focused = true;
    tb.blink_on = false;

    let theme = crate::theme::test_theme();
    let mut dl = DrawList::new();
    let mut pc = PaintCtx { global_alpha: 1.0, list: &mut dl, theme: &theme, dpi: 1.0, offset: (0.0, 0.0) };
    tb.paint(&mut pc);

    let fills: Vec<_> = dl.cmds.iter().filter(|c| matches!(c, DrawCmd::FillRect { .. })).collect();
    assert_eq!(fills.len(), 2, "expected bg + border fills only, no cursor");
}

#[test]
fn paint_preedit_draws_text_and_underline() {
    use crate::core::paint::DrawList;
    use crate::theme::Theme;

    let mut tb = TextBox::new();
    tb.set_text("hello ");
    tb.cursor_byte = 6;
    tb.rect = Rect::new(0.0, 0.0, 200.0, 28.0);
    tb.on_ime(&TextBoxIme::Preedit { text: "世界".into(), cursor: Some((0, 6)) });
    tb.preedit_width = 28.0; // simulate layout

    let theme = crate::theme::test_theme();
    let mut dl = DrawList::new();
    let mut pc = PaintCtx { global_alpha: 1.0, list: &mut dl, theme: &theme, dpi: 1.0, offset: (0.0, 0.0) };
    tb.paint(&mut pc);

    let texts: Vec<_> = dl.cmds.iter().filter(|c| matches!(c, DrawCmd::Text { .. })).collect();
    assert_eq!(texts.len(), 2, "expected main text + preedit text");
    // preedit text should be "世界"
    if let DrawCmd::Text { content, .. } = &texts[1] {
        assert_eq!(content, "世界");
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p ui text_box 2>&1`
Expected: All tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/src/widgets/text_box.rs
git commit -m "feat: add layout (text measurement) and paint to TextBox"
```

---

### Task 6: Add sync_text for external state reconciliation

**Files:**
- Modify: `crates/ui/src/widgets/text_box.rs`

- [ ] **Step 1: Add sync_text method**

```rust
/// Sync text from an external source (e.g., app-layer snapshot).
/// Only overwrites when the external text differs from internal text.
/// This prevents snapshot-based state injection from overwriting
/// live user input that hasn't been flushed yet.
pub fn sync_text(&mut self, ext_text: &str) {
    if self.text != ext_text {
        self.text = String::from(ext_text);
        if self.cursor_byte > self.text.len() {
            self.cursor_byte = self.text.len();
        }
        self.selection = None;
    }
}
```

- [ ] **Step 2: Write sync_text tests**

```rust
#[test]
fn sync_text_overwrites_when_different() {
    let mut tb = TextBox::new();
    tb.set_text("hello");
    tb.cursor_byte = 3;
    tb.sync_text("world");
    assert_eq!(tb.text(), "world");
    assert!(tb.selection.is_none());
}

#[test]
fn sync_text_noop_when_same() {
    let mut tb = TextBox::new();
    tb.set_text("hello");
    tb.cursor_byte = 3;
    tb.sync_text("hello");
    assert_eq!(tb.text(), "hello");
    assert_eq!(tb.cursor_byte(), 3); // preserved!
}

#[test]
fn sync_text_clamps_cursor_when_shorter() {
    let mut tb = TextBox::new();
    tb.set_text("long text");
    tb.sync_text("short");
    assert_eq!(tb.cursor_byte(), 5); // clamped to "short".len()
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p ui text_box 2>&1`
Expected: All tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/ui/src/widgets/text_box.rs
git commit -m "feat: add sync_text for external state reconciliation"
```

---

### Task 7: Export text_box module

**Files:**
- Modify: `crates/ui/src/widgets/mod.rs`

- [ ] **Step 1: Add module declaration**

Add to `crates/ui/src/widgets/mod.rs`:

```rust
pub mod text_box;
```

- [ ] **Step 2: Build**

Run: `cargo build -p ui 2>&1`
Expected: Compiles.

- [ ] **Step 3: Commit**

```bash
git add crates/ui/src/widgets/mod.rs
git commit -m "feat: export text_box module from widgets"
```

---

### Task 8: Refactor SearchBarWidget to use TextBox

**Files:**
- Modify: `crates/ui/src/widgets/search_bar.rs`

- [ ] **Step 1: Rewrite SearchBarWidget struct to embed two TextBoxes**

Replace the struct and impl block. Key changes:

```rust
use crate::widgets::text_box::{TextBox, TextBoxIme};

pub struct SearchBarWidget {
    rect: Rect,
    pill_rect: Cell<Rect>,
    snap: SearchBarSnapshot,
    find_box: TextBox,
    replace_box: TextBox,
    close_btn_rect: Cell<Rect>,
    prev_btn_rect: Cell<Rect>,
    next_btn_rect: Cell<Rect>,
    replace_btn_rect: Cell<Rect>,
    replace_all_btn_rect: Cell<Rect>,
    toggle_replace_btn_rect: Cell<Rect>,
    regex_btn_rect: Cell<Rect>,
    hovered_btn: HoveredButton,
    pending_actions: std::rc::Rc<std::cell::RefCell<Vec<WidgetAction>>>,
}

impl SearchBarWidget {
    pub fn new() -> Self {
        let pending: std::rc::Rc<std::cell::RefCell<Vec<WidgetAction>>> = Default::default();
        let mut find_box = TextBox::new();
        find_box.set_placeholder("Find...");
        {
            let p = pending.clone();
            find_box.on_changed = Some(Box::new(move |text| {
                p.borrow_mut().push(WidgetAction::SearchBar(SearchBarAction::QueryChanged(text)));
            }));
        }
        {
            let p = pending.clone();
            find_box.on_enter = Some(Box::new(move || {
                p.borrow_mut().push(WidgetAction::SearchBar(SearchBarAction::Next));
            }));
        }
        {
            let p = pending.clone();
            find_box.on_escape = Some(Box::new(move || {
                p.borrow_mut().push(WidgetAction::SearchBar(SearchBarAction::DismissOrClear));
            }));
        }

        let mut replace_box = TextBox::new();
        replace_box.set_placeholder("Replace...");
        {
            let p = pending.clone();
            replace_box.on_changed = Some(Box::new(move |text| {
                p.borrow_mut().push(WidgetAction::SearchBar(SearchBarAction::ReplaceQueryChanged(text)));
            }));
        }
        {
            let p = pending.clone();
            replace_box.on_enter = Some(Box::new(move || {
                p.borrow_mut().push(WidgetAction::SearchBar(SearchBarAction::Replace));
            }));
        }

        Self {
            rect: Rect::ZERO,
            pill_rect: Cell::new(Rect::ZERO),
            snap: SearchBarSnapshot::default(),
            find_box,
            replace_box,
            close_btn_rect: Cell::new(Rect::ZERO),
            prev_btn_rect: Cell::new(Rect::ZERO),
            next_btn_rect: Cell::new(Rect::ZERO),
            replace_btn_rect: Cell::new(Rect::ZERO),
            replace_all_btn_rect: Cell::new(Rect::ZERO),
            toggle_replace_btn_rect: Cell::new(Rect::ZERO),
            regex_btn_rect: Cell::new(Rect::ZERO),
            hovered_btn: HoveredButton::None,
            pending_actions: pending,
        }
    }
}
```

- [ ] **Step 2: Update SearchBarAction — remove obsolete variants, add new ones**

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchBarAction {
    Next,
    Prev,
    Close,
    DismissOrClear,
    ToggleReplace,
    ToggleRegex,
    Replace,
    ReplaceAll,
    FocusFind,
    FocusReplace,
    QueryChanged(String),
    ReplaceQueryChanged(String),
}
```

Removed: `InsertChar`, `Backspace`, `InsertReplaceChar`, `ReplaceBackspace`, `MoveCursor`, `ClearQuery`.

- [ ] **Step 3: Refactor SearchBarWidget::set_input to call sync_text**

```rust
pub fn set_input(&mut self, snap: SearchBarSnapshot) {
    self.find_box.sync_text(&snap.query);
    self.replace_box.sync_text(&snap.replace_query);
    self.find_box.set_blink(snap.blink_on);
    self.replace_box.set_blink(snap.blink_on);
    self.snap = snap;
}
```

- [ ] **Step 4: Refactor set_rect to call TextBox::layout**

```rust
fn set_rect(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
    self.rect = Rect::new(0.0, 0.0, rect.w, rect.h);

    let dpi = ctx.dpi;
    let font_size = 14.0 * dpi;
    let pad_left = 36.0 * dpi;
    let pad_right = 8.0 * dpi;
    let btn_size = 20.0 * dpi;
    let btn_gap = 4.0 * dpi;
    let pill_w = self.rect.w;
    let pill_h = self.rect.h;

    // Compute right-side button widths (same as current paint_find_replace logic, lines 326-343)
    let replace_btn_w = 4.0 * 8.0 * dpi;
    let nav_width = if self.snap.match_count > 0 {
        let count_text = format!("{}/{}",
            self.snap.current_match.saturating_add(1).min(self.snap.match_count),
            self.snap.match_count);
        btn_size + btn_gap + btn_size + btn_gap + count_text.len() as f32 * 8.0 * dpi + btn_gap + btn_size + btn_gap
    } else if !self.snap.query.is_empty() {
        let no_w = "No results".len() as f32 * 8.0 * dpi;
        no_w + btn_gap + btn_size + btn_gap
    } else {
        btn_size + btn_gap
    };
    let right_total = pad_right + btn_size + btn_gap + btn_size + btn_gap
        + nav_width + replace_btn_w + btn_gap + replace_btn_w + btn_gap;

    let input_area_left = pad_left;
    let input_area_right = pill_w - right_total;
    let input_area_w = (input_area_right - input_area_left).max(80.0 * dpi);
    let sep_w = 20.0 * dpi;
    let input_h = font_size + 4.0 * dpi;
    let input_y = (pill_h - input_h) * 0.5;

    if self.snap.replace_mode {
        let find_w = (input_area_w - sep_w) * 0.5;
        let replace_w = (input_area_w - sep_w) * 0.5;
        let find_rect = Rect::new(input_area_left - 4.0 * dpi, input_y, find_w + 4.0 * dpi, input_h);
        let replace_left = input_area_left + find_w + sep_w;
        let replace_rect = Rect::new(replace_left - 4.0 * dpi, input_y, replace_w + 4.0 * dpi, input_h);
        self.find_box.layout(find_rect, ctx);
        self.replace_box.layout(replace_rect, ctx);
    } else {
        let find_rect = Rect::new(input_area_left - 4.0 * dpi, input_y, input_area_w, input_h);
        self.find_box.layout(find_rect, ctx);
    }
}
```

- [ ] **Step 5: Refactor on_event to route to TextBox**

```rust
fn on_event(&mut self, ev: &Event, _ctx: &mut EventCtx) -> Option<WidgetAction> {
    if !self.snap.visible {
        return None;
    }

    match ev {
        Event::KeyDown(kc, modifiers) => {
            let target = if self.snap.replace_mode && self.snap.focus_replace {
                &mut self.replace_box
            } else {
                &mut self.find_box
            };
            target.on_key(*kc, *modifiers);

            // Drain any pending actions from callbacks
            self.pending_actions.borrow_mut().pop()
        }
        Event::ImePreedit { text, cursor } => {
            let target = if self.snap.replace_mode && self.snap.focus_replace {
                &mut self.replace_box
            } else {
                &mut self.find_box
            };
            target.on_ime(&TextBoxIme::Preedit { text: text.clone(), cursor: *cursor });
            Some(WidgetAction::Consumed)
        }
        Event::ImeCommit(text) => {
            let target = if self.snap.replace_mode && self.snap.focus_replace {
                &mut self.replace_box
            } else {
                &mut self.find_box
            };
            target.on_ime(&TextBoxIme::Commit(text.clone()));
            self.pending_actions.borrow_mut().pop()
        }
        Event::ImeEnable => {
            self.find_box.on_ime(&TextBoxIme::Enabled);
            self.replace_box.on_ime(&TextBoxIme::Enabled);
            Some(WidgetAction::Consumed)
        }
        Event::ImeDisable => {
            self.find_box.on_ime(&TextBoxIme::Disabled);
            self.replace_box.on_ime(&TextBoxIme::Disabled);
            Some(WidgetAction::Consumed)
        }
        Event::MouseMove { px, py } => {
            // ... existing hover logic unchanged ...
        }
        Event::MouseDown { px, py, button: MouseButton::Left } => {
            // Route to TextBox first, then to buttons
            self.find_box.on_mouse_down(*px, *py);
            self.replace_box.on_mouse_down(*px, *py);
            // ... existing button hit-test unchanged ...
        }
        Event::MouseUp { .. } => {
            self.find_box.on_mouse_up();
            self.replace_box.on_mouse_up();
            None
        }
        _ => None,
    }
}
```

- [ ] **Step 6: Refactor paint to delegate to TextBox**

In `paint_find_only`, remove the inline text/cursor drawing and call `self.find_box.paint(ctx)`.

In `paint_find_replace`, remove the inline find/replace input drawing and call both `self.find_box.paint(ctx)` and `self.replace_box.paint(ctx)`.

The button drawing (`paint_right_buttons` / `paint_right_buttons_inline`) remains unchanged.

- [ ] **Step 7: Update existing search_bar tests**

Key changes to tests:
- SearchBarWidget construction changes (`new()` initializes TextBoxes)
- Tests involving `InsertChar`/`Backspace` actions need to route through `on_key` instead
- Create a helper function that builds a SearchBarWidget with the new constructor

The paint output tests should still pass since TextBox::paint produces the same visual structure (bg, border, text, cursor).

- [ ] **Step 7a: Add focused_textbox_ime_cursor_rect for app layer access**

```rust
/// Returns the IME cursor rect of the currently focused TextBox.
/// Used by app layer to position the OS IME candidate window.
pub fn focused_textbox_ime_cursor_rect(&self) -> Option<Rect> {
    if !self.snap.visible {
        return None;
    }
    if self.snap.replace_mode && self.snap.focus_replace {
        if self.replace_box.has_preedit() || self.replace_box.is_focused() {
            return Some(self.replace_box.ime_cursor_rect());
        }
    }
    if self.find_box.has_preedit() || self.find_box.is_focused() {
        return Some(self.find_box.ime_cursor_rect());
    }
    None
}
```

- [ ] **Step 8: Run all ui crate tests**

Run: `cargo test -p ui 2>&1`
Expected: All tests pass, including updated search_bar tests and text_box tests.

- [ ] **Step 9: Commit**

```bash
git add crates/ui/src/widgets/search_bar.rs
git commit -m "refactor: replace inline text input with TextBox in SearchBarWidget"
```

---

### Task 9: Wire app layer — IME events, clipboard, focus

**Files:**
- Modify: `crates/app/src/app_lifecycle.rs`
- Modify: `crates/app/src/ui_shell.rs`
- Modify: `crates/app/src/app_renderer.rs`
- Modify: `crates/app/src/app_window.rs`
- Modify: `crates/app/src/search_state.rs`

- [ ] **Step 1: Route winit IME events as Event::Ime* in app_lifecycle.rs**

In the `window_event` handler, replace the current IME handling:

```rust
// OLD (remove):
// WindowEvent::Ime(Ime::Preedit(text, cursor)) => {
//     self.preedit_text = text;
//     ...
// }

// NEW:
WindowEvent::Ime(Ime::Preedit(text, cursor)) => {
    self.ui_shell.forward_ime(ui::core::Event::ImePreedit { text, cursor });
    self.needs_redraw = true;
}
WindowEvent::Ime(Ime::Commit(text)) => {
    self.ui_shell.forward_ime(ui::core::Event::ImeCommit(text));
    self.needs_redraw = true;
}
WindowEvent::Ime(Ime::Enabled) => {
    self.ui_shell.forward_ime(ui::core::Event::ImeEnable);
    self.needs_redraw = true;
}
WindowEvent::Ime(Ime::Disabled) => {
    self.ui_shell.forward_ime(ui::core::Event::ImeDisable);
    self.needs_redraw = true;
}
```

- [ ] **Step 2: Add forward_ime to UiShell**

In `crates/app/src/ui_shell.rs`:

```rust
pub(crate) fn forward_ime(&mut self, ev: ui::core::Event) {
    let _ = self.dock.dispatch(&ev);
}
```

- [ ] **Step 3: Wire clipboard callbacks in UiShell**

When constructing SearchBarWidget, set clipboard callbacks on the TextBoxes. In `UiShell::new()` or wherever SearchBarWidget is created:

```rust
use arboard::Clipboard;
// ... when creating SearchBarWidget's TextBoxes:

let mut clipboard = Clipboard::new().ok();
// On the find_box (or replace_box):
find_box.on_copy = Some(Box::new(move |text: String| {
    if let Some(ref mut cb) = clipboard {
        let _ = cb.set_text(text);
    }
}));
// ... similar for on_cut and on_paste ...
```

Note: `arboard::Clipboard` is not `Send`/`Sync`, so the callback closure needs careful handling. Use `Rc<RefCell<Option<Clipboard>>>` or similar.

- [ ] **Step 4: Wire IME cursor area through TextBox**

In `app_window.rs`'s `update_ime_cursor_area`, instead of computing search bar cursor position from app state, read it from UiShell:

```rust
// Get IME cursor rect from the focused TextBox in SearchBarWidget
if let Some(ime_rect) = self.ui_shell.search_ime_cursor_rect() {
    // Convert to physical position
    let dpi = Settings::with(|s| s.dpi_scale);
    let pos = PhysicalPosition::new(
        (ime_rect.x * dpi) as i32,
        (ime_rect.y * dpi) as i32,
    );
    let size = PhysicalSize::new(
        (ime_rect.w * dpi) as u32,
        (ime_rect.h * dpi) as u32,
    );
    window.set_ime_cursor_area(pos, size);
}
```

- [ ] **Step 5: Add search_ime_cursor_rect to UiShell**

```rust
pub(crate) fn search_ime_cursor_rect(&self) -> Option<Rect> {
    for child in &self.dock.children {
        if child.widget.id() == Some(ui::core::widget::ids::SEARCH_BAR) {
            if let Some(sb) = child.widget.as_any().downcast_ref::<ui::widgets::search_bar::SearchBarWidget>() {
                return sb.focused_textbox_ime_cursor_rect();
            }
        }
    }
    None
}
```

- [ ] **Step 6: Update app_search.rs to handle new SearchBarAction variants**

Replace `InsertChar`/`InsertReplaceChar`/`Backspace`/`ReplaceBackspace` handling with `QueryChanged`/`ReplaceQueryChanged`:

```rust
SearchBarAction::QueryChanged(text) => {
    dv.search_state.query = text;
    self.perform_search_for_active_doc();
}
SearchBarAction::ReplaceQueryChanged(text) => {
    dv.search_state.replace_query = text;
}
```

- [ ] **Step 7: Build and fix compile errors**

Run: `cargo build 2>&1`
Expected: Various compile errors from the SearchBarAction changes. Fix each one systematically.

- [ ] **Step 8: Run full test suite**

Run: `cargo test 2>&1`
Expected: All tests pass.

- [ ] **Step 9: Commit**

```bash
git add crates/app/src/
git commit -m "feat: wire app layer — IME events, clipboard, TextBox integration"
```

---

### Task 10: Clean up old IME code

**Files:**
- Modify: `crates/app/src/app.rs`
- Modify: `crates/app/src/app_renderer.rs`
- Modify: `crates/app/src/app_window.rs`

- [ ] **Step 1: Remove preedit fields from App struct**

In `crates/app/src/app.rs`, remove:
```rust
pub(crate) preedit_text: String,
pub(crate) preedit_cursor: Option<(usize, usize)>,
pub(crate) preedit_advance_px: f32,
```

- [ ] **Step 2: Remove search bar preedit GPU rendering from app_renderer.rs**

Remove the block that renders preedit text at the search bar cursor position (the "IME preedit text rendering for search bar cursor" section). TextBox::paint() now handles this.

- [ ] **Step 3: Remove document preedit advance handling (if search bar was the only consumer)**

Keep the document editor preedit rendering since TextBox doesn't handle document editing. Only remove the search-bar-specific preedit code.

- [ ] **Step 4: Simplify update_ime_cursor_area**

Remove the search bar cursor position computation code — it now reads from TextBox via UiShell.

- [ ] **Step 5: Remove SearchBarSnapshot.preedit_text field**

In `crates/ui/src/widgets/search_bar.rs`:
```rust
// Remove from SearchBarSnapshot:
pub preedit_text: String,   // DELETE
```

- [ ] **Step 6: Build and test**

Run: `cargo build 2>&1 && cargo test 2>&1`
Expected: Compiles and all tests pass.

- [ ] **Step 7: Commit**

```bash
git add -A crates/
git commit -m "chore: remove old preedit/cursor code now handled by TextBox"
```

---

## Implementation Order

Tasks must be executed sequentially (each depends on the previous):

```
1 → 2 → 3 → 4 → 5 → 6 → 7 → 8 → 9 → 10
```

No tasks are independent; they all build on the TextBox incrementally.
