# Pointer Multi-click Remediation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 保持双击按词、三击按物理行的选择粒度，避免指针轻微移动或拖拽后退化为逐字素选择。

**Architecture:** 多击粒度属于一次文本选择会话，由 EditorInputSession 保存并随捕获结束或失焦清理。EditorRuntime 继续保留现有公开 begin_text_selection 接口作为单击默认入口；产品内指针路径通过新增的 crate-private 入口显式传入粒度。

**Tech Stack:** Rust、winit 事件、ui::PointerClickKind、appkit-shell 单元测试。

## Global Constraints

- 不改变公开 EditorRuntime::begin_text_selection 的签名或行为。
- 不把点击跟踪器状态复制到文档模型；会话仅保存本次拖拽的选择粒度。
- Release、focus_lost、取消捕获以及新手势必须清理或覆盖旧粒度。
- 每个实现步骤先运行 RED 测试，再写最小实现，最后 cargo fmt 与定向测试。

---

## Task 1: 用失败测试锁定多击拖拽行为

**Files:**
- Modify: crates/appkit-shell/src/editor_runtime/mod.rs

- [ ] 在现有 custom_editor_double_click_selects_the_word_at_the_pointer 附近增加双击轻微移动回归测试：

    #[test]
    fn custom_editor_double_click_selection_survives_pointer_jitter() {
        let mut runtime = runtime_with_clean_tab();
        let tab_id = runtime.active_tab_id().expect("test runtime should have an active tab");
        runtime
            .tab_session_mut(tab_id)
            .expect("active tab should have a runtime")
            .replace_plugin(Box::new(PointerProbePlugin));
        let context = EditorInputContext { focus: EditorFocus::Active, modal_blocked: false };
        let pointer = (180.0, 260.0);
        paint_editor_surface(&mut runtime, ui::Rect::new(100.0, 200.0, 640.0, 480.0));

        runtime.handle_pointer_event(
            context,
            &ui::Event::MouseDown { px: pointer.0, py: pointer.1, button: ui::MouseButton::Left },
        );
        runtime.handle_pointer_event(
            context,
            &ui::Event::MouseUp { px: pointer.0, py: pointer.1, button: ui::MouseButton::Left },
        );
        runtime.handle_pointer_event(
            context,
            &ui::Event::MouseDown { px: pointer.0, py: pointer.1, button: ui::MouseButton::Left },
        );
        runtime.handle_pointer_event(
            context,
            &ui::Event::MouseMove { px: pointer.0 + 1.0, py: pointer.1 },
        );

        let snapshot = &runtime.workspace_snapshot().tabs[0];
        assert_eq!(snapshot.selection_anchor, Some(0));
        assert_eq!(snapshot.cursor_offset, 5);
    }

- [ ] 增加 plain_text_triple_click_drag_extends_by_source_line。先通过 replace_document_text 将正文替换为 alpha\nbeta，并为两个物理行写入 advance_cache；完成两次 MouseDown/MouseUp 后，第三次只发 MouseDown，再 MouseMove 到第二行。断言 selection_anchor == Some(0) 且 cursor_offset == 10。

- [ ] 运行两个测试并确认 RED；失败现象应是 MouseMove 后选择范围缩成命中字节，而不是测试装配错误：

    cargo test -p textora-appkit-shell --lib editor_runtime::tests::custom_editor_double_click_selection_survives_pointer_jitter -- --exact
    cargo test -p textora-appkit-shell --lib editor_runtime::tests::plain_text_triple_click_drag_extends_by_source_line -- --exact

- [ ] 提交测试：

    git add crates/appkit-shell/src/editor_runtime/mod.rs
    git commit -m "test(appkit-shell): cover multi-click pointer drag"

## Task 2: 将选择粒度纳入输入会话

**Files:**
- Modify: crates/appkit-shell/src/editor_runtime/input_session.rs
- Modify: crates/appkit-shell/src/editor_runtime/mod.rs

- [ ] 在 input_session.rs 定义 crate-private 类型，并给 EditorInputSession 增加 Option 字段：

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) enum TextSelectionGranularity {
        Grapheme,
        Word,
        SourceLine,
    }

- [ ] 将 start_text_selection 的内部签名改为接收 TextSelectionGranularity；成功捕获时写入粒度。新增 text_selection_granularity getter，未处于文本选择捕获时返回 None。

- [ ] 在 end_pointer_capture、focus_lost、start_canvas_drag 中清理 text_selection_granularity，确保不同指针模式不能复用旧状态。

- [ ] 保留公开 begin_text_selection(context)，内部以 Grapheme 启动；新增 pub(crate) begin_text_selection_with_granularity(context, granularity) 与 text_selection_granularity() 供 editor_pointer.rs 使用。

- [ ] 更新 input_session.rs 现有测试调用，并增加 text_selection_granularity_follows_pointer_capture_lifetime，覆盖开始、结束捕获、重新开始、失焦四个状态。

- [ ] 运行会话测试并确认 GREEN：

    cargo test -p textora-appkit-shell --lib editor_runtime::input_session::tests

- [ ] 提交会话状态：

    git add crates/appkit-shell/src/editor_runtime/input_session.rs crates/appkit-shell/src/editor_runtime/mod.rs
    git commit -m "refactor(appkit-shell): track text selection granularity"

## Task 3: 指针移动沿用会话粒度

**Files:**
- Modify: crates/appkit-shell/src/editor_runtime/editor_pointer.rs

- [ ] 增加两个无状态映射函数，保持 UI 点击分类与输入会话类型之间的边界清晰：

    fn selection_granularity(click_kind: PointerClickKind) -> TextSelectionGranularity {
        match click_kind {
            PointerClickKind::Single => TextSelectionGranularity::Grapheme,
            PointerClickKind::Double => TextSelectionGranularity::Word,
            PointerClickKind::Triple => TextSelectionGranularity::SourceLine,
        }
    }

    fn click_kind(granularity: TextSelectionGranularity) -> PointerClickKind {
        match granularity {
            TextSelectionGranularity::Grapheme => PointerClickKind::Single,
            TextSelectionGranularity::Word => PointerClickKind::Double,
            TextSelectionGranularity::SourceLine => PointerClickKind::Triple,
        }
    }

- [ ] Press 分支用 begin_text_selection_with_granularity 启动捕获；Move 分支从 text_selection_granularity 读取粒度。状态缺失时安全回退 PointerClickKind::Single，但不把回退写回会话。

- [ ] 运行 Task 1 的两个回归测试和整个 editor_runtime 测试模块：

    cargo test -p textora-appkit-shell --lib editor_runtime::tests::custom_editor_double_click_selection_survives_pointer_jitter -- --exact
    cargo test -p textora-appkit-shell --lib editor_runtime::tests::plain_text_triple_click_drag_extends_by_source_line -- --exact
    cargo test -p textora-appkit-shell --lib editor_runtime::tests

- [ ] 格式化并检查包：

    cargo fmt --all -- --check
    cargo check -p textora-appkit-shell

- [ ] 自审：确认公开 API 未变、所有退出路径清理粒度、没有新增 bool 状态组合。

- [ ] 提交实现：

    git add crates/appkit-shell/src/editor_runtime/editor_pointer.rs
    git commit -m "fix(appkit-shell): preserve multi-click drag granularity"
