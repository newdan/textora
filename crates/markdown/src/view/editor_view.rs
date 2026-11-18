use super::*;

// ===== MarkdownEditorView =====

/// WYSIWYG markdown editor view — wraps PreviewEngine with editing support.
/// Differs from MarkdownView in that it accepts cursor position updates
/// and provides layout-aware editing queries.
pub struct MarkdownEditorView {
    pub(in crate::view) engine: PreviewEngine,
    pub(in crate::view) source: String,
    pub(in crate::view) cached_source_hash: u64,
    pub(in crate::view) cached_generation: u32,
    pub(in crate::view) split_history: std::cell::RefCell<split_history::SplitHistory>,
}

impl Default for MarkdownEditorView {
    fn default() -> Self {
        Self::new()
    }
}

impl MarkdownEditorView {
    pub fn new() -> Self {
        let mut engine = PreviewEngine::new();
        engine.set_edit_source(Some(String::new()));
        Self {
            engine,
            source: String::new(),
            cached_source_hash: 0,
            cached_generation: 0,
            split_history: Default::default(),
        }
    }

    pub fn set_source(&mut self, text: String, generation: u32) {
        self.split_history.get_mut().synchronize(&self.source, &text);
        let hash = fxhash(&text);
        let source_changed =
            hash != self.cached_source_hash || generation != self.cached_generation;
        if source_changed {
            self.engine.set_preedit_text(String::new(), None);
        }
        if hash != self.cached_source_hash {
            self.source = text;
            self.cached_source_hash = hash;
            self.engine.mark_source_dirty();
        }
        self.engine.set_edit_source(Some(self.source.clone()));
        self.engine.set_source_generation(generation);
        self.cached_generation = generation;
        if source_changed {
            self.engine.prepare_current_source_layout(&self.source);
        }
    }

    pub fn needs_source_update(&self, generation: u32) -> bool {
        generation != self.cached_generation
    }

    pub fn engine(&self) -> &PreviewEngine {
        &self.engine
    }
}

impl ui::plugin::KeyIntentMapper for MarkdownEditorView {
    fn map_key(
        &self,
        key: &ui::KeyCode,
        modifiers: &ui::core::Modifiers,
    ) -> Option<ui::plugin::EditIntent> {
        let is_unmodified_shift_enter = matches!(key, ui::KeyCode::Enter)
            && modifiers.shift
            && !modifiers.cmd
            && !modifiers.ctrl
            && !modifiers.alt;
        is_unmodified_shift_enter.then_some(ui::plugin::EditIntent::InsertLineBreak)
    }
}

fn request_augment_kind(intent: &ui::plugin::EditIntent) -> Option<ui::plugin::AugmentKind> {
    match intent {
        ui::plugin::EditIntent::InsertText(text) => {
            Some(ui::plugin::AugmentKind::InsertText(text.clone()))
        }
        ui::plugin::EditIntent::InsertParagraphBreak => Some(ui::plugin::AugmentKind::Enter),
        ui::plugin::EditIntent::InsertLineBreak => Some(ui::plugin::AugmentKind::LineBreak),
        ui::plugin::EditIntent::DeleteBackward => Some(ui::plugin::AugmentKind::Backspace),
        ui::plugin::EditIntent::Indent => Some(ui::plugin::AugmentKind::Tab),
        ui::plugin::EditIntent::Outdent => Some(ui::plugin::AugmentKind::ShiftTab),
        ui::plugin::EditIntent::DeleteForward
        | ui::plugin::EditIntent::PromoteObject
        | ui::plugin::EditIntent::DemoteObject
        | ui::plugin::EditIntent::SelectObject => None,
    }
}

pub(in crate::view) fn augmentation_edit_plan(
    request: &ui::plugin::EditRequest,
    augmentation: ui::plugin::EditAugmentation,
) -> ui::plugin::EditPlan {
    let range = augmentation.replace_range.unwrap_or(request.cursor_byte..request.cursor_byte);
    let text = augmentation.insert_text.unwrap_or_default();
    if range.is_empty() && text.is_empty() {
        if augmentation.cursor_byte_after == request.cursor_byte {
            return ui::plugin::EditPlan::Consume;
        }
        return ui::plugin::EditPlan::MoveCursor(ui::plugin::CursorUpdate {
            cursor_after: augmentation.cursor_byte_after,
        });
    }
    ui::plugin::EditPlan::Apply(ui::plugin::EditTransaction::replace(
        request.source_generation,
        range,
        text,
        augmentation.cursor_byte_after,
    ))
}

/// 把基于"选区已删除"虚拟源码计算的增强映射回真实文档的单条替换。
///
/// 虚拟源码 = 真实源码删除 `selection` 后的文本，删除点（= `selection.start`）
/// 即增强计算的光标位置。偏移映射规则：虚拟偏移 `v < selection.start` 时真实
/// 偏移相同，否则 `v + 选区长度`；range 起点位于删除点左侧时保持不变，因此
/// 映射后的 range 恰好覆盖整个选区。
/// `cursor_byte_after` 是增强后文本的坐标——真实替换与虚拟替换产生的最终
/// 文本是同一字符串，无需映射。
///
/// 仅当增强的 `replace_range` 覆盖删除点时映射成立（Enter 的各增强均满足）；
/// 否则返回 `UseDefault` 回落默认计划。
fn selection_augmentation_edit_plan(
    request: &ui::plugin::EditRequest,
    selection: &std::ops::Range<usize>,
    augmentation: ui::plugin::EditAugmentation,
) -> ui::plugin::EditPlan {
    let virtual_range = augmentation.replace_range.unwrap_or(selection.start..selection.start);
    let deleted_len = selection.end - selection.start;
    let replacement_text = augmentation.insert_text.unwrap_or_default();

    if virtual_range.start <= selection.start && virtual_range.end >= selection.start {
        return ui::plugin::EditPlan::Apply(ui::plugin::EditTransaction::replace(
            request.source_generation,
            virtual_range.start..virtual_range.end + deleted_len,
            replacement_text,
            augmentation.cursor_byte_after,
        ));
    }

    let mapped_range = if virtual_range.end < selection.start {
        virtual_range
    } else if virtual_range.start > selection.start {
        virtual_range.start + deleted_len..virtual_range.end + deleted_len
    } else {
        return ui::plugin::EditPlan::UseDefault;
    };
    ui::plugin::EditPlan::Apply(ui::plugin::EditTransaction {
        source_generation: request.source_generation,
        replacements: vec![
            ui::plugin::TextReplacement { range: selection.clone(), text: String::new() },
            ui::plugin::TextReplacement { range: mapped_range, text: replacement_text },
        ],
        selection_after: ui::plugin::EditSelection::Caret(augmentation.cursor_byte_after),
    })
}

impl ui::plugin::EditPolicy for MarkdownEditorView {
    fn plan_edit(&self, request: &ui::plugin::EditRequest) -> ui::plugin::EditPlan {
        self.split_history.borrow_mut().begin_request();
        if request.source_generation != self.cached_generation {
            return ui::plugin::EditPlan::UseDefault;
        }
        // 零宽选区视为无选区。
        let selection = request.selection.as_ref().filter(|range| range.start < range.end);
        if let Some(selection) = selection {
            return self.plan_selection_edit(request, selection);
        }
        if matches!(request.intent, ui::plugin::EditIntent::DeleteBackward)
            && let Some(plan) = self.split_history.borrow_mut().restore(&self.source, request)
        {
            return plan;
        }
        if matches!(request.intent, ui::plugin::EditIntent::DeleteForward) {
            return crate::augmenter::augment_delete_forward(&self.source, request.cursor_byte)
                .map_or(ui::plugin::EditPlan::UseDefault, |augmentation| {
                    augmentation_edit_plan(request, augmentation)
                });
        }
        let Some(kind) = request_augment_kind(&request.intent) else {
            return ui::plugin::EditPlan::UseDefault;
        };
        let plan = self
            .engine
            .augment_edit(request.cursor_byte, kind)
            .map_or(ui::plugin::EditPlan::UseDefault, |augmentation| {
                augmentation_edit_plan(request, augmentation)
            });
        if matches!(request.intent, ui::plugin::EditIntent::InsertParagraphBreak) {
            self.split_history.borrow_mut().remember(&self.source, request, &plan);
        }
        plan
    }
}

impl MarkdownEditorView {
    /// 清空段落复用范围删除；回车基于删除选区后的上下文生成原子计划。
    pub(in crate::view) fn plan_selection_edit(
        &self,
        request: &ui::plugin::EditRequest,
        selection: &std::ops::Range<usize>,
    ) -> ui::plugin::EditPlan {
        if self.source.get(selection.clone()).is_none() {
            return ui::plugin::EditPlan::UseDefault;
        }
        if let ui::plugin::EditIntent::InsertText(text) = &request.intent {
            return crate::augmenter::augment_selected_text(&self.source, selection.clone(), text)
                .map_or(ui::plugin::EditPlan::UseDefault, |augmentation| {
                    selection_augmentation_edit_plan(request, selection, augmentation)
                });
        }
        if matches!(
            request.intent,
            ui::plugin::EditIntent::DeleteBackward | ui::plugin::EditIntent::DeleteForward
        ) {
            return crate::augmenter::augment_erase_range(&self.source, selection.clone())
                .map_or(ui::plugin::EditPlan::UseDefault, |augmentation| {
                    augmentation_edit_plan(request, augmentation)
                });
        }
        if !matches!(
            request.intent,
            ui::plugin::EditIntent::InsertParagraphBreak | ui::plugin::EditIntent::InsertLineBreak
        ) {
            return ui::plugin::EditPlan::UseDefault;
        }
        let mut source_after_delete = self.source.clone();
        source_after_delete.replace_range(selection.clone(), "");
        let kind = match request.intent {
            ui::plugin::EditIntent::InsertLineBreak => ui::plugin::AugmentKind::LineBreak,
            _ => ui::plugin::AugmentKind::Enter,
        };
        let Some(augmentation) =
            crate::augmenter::augment_edit(&source_after_delete, selection.start, kind)
        else {
            return ui::plugin::EditPlan::UseDefault;
        };
        selection_augmentation_edit_plan(request, selection, augmentation)
    }
}

impl ViewPlugin for MarkdownEditorView {
    fn paste_preference(&self) -> PastePreference {
        PastePreference::SemanticMarkdown
    }

    fn handles_own_rendering(&self) -> bool {
        true
    }

    fn edit_policy(&self) -> &dyn ui::plugin::EditPolicy {
        self
    }

    fn key_intent_mapper(&self) -> Option<&dyn ui::plugin::KeyIntentMapper> {
        Some(self)
    }

    fn name(&self) -> &str {
        "markdown_editor"
    }

    fn allows_editing(&self) -> bool {
        // WYSIWYG editor — app_renderer will enable
        // Copy/Paste/Undo context menu and scrollbar behaviour.
        true
    }

    fn shows_cursor(&self) -> bool {
        // WYSIWYG draws its own cursor via render() — app should not overlay one.
        false
    }

    fn shows_gutter(&self) -> bool {
        false
    }

    fn needs_cursor_blink_wakeup(&self) -> bool {
        // App must still compute blink phase and forward it via SetCursorVisible.
        true
    }

    fn render(
        &mut self,
        _doc: &dyn DocView,
        bounds: ui::core::geom::Rect,
        theme: &Theme,
        shaper: &mut shaping::Shaper,
        dpi_scale: f32,
    ) -> DrawList {
        preedit::render_editor(self, bounds, theme, shaper, dpi_scale)
    }

    fn handle_message(
        &mut self,
        msg: PluginMessage,
        _doc: &mut dyn core::document::DocViewMut,
    ) -> bool {
        if let Some(result) = self.engine.handle_message_common(&msg) {
            return result;
        }
        match msg {
            PluginMessage::UpdateSource { text, generation } => {
                self.set_source(text, generation);
                true
            }
            _ => false,
        }
    }

    fn query(&self, q: PluginQuery, _doc: &dyn DocView) -> PluginResponse {
        if matches!(q, PluginQuery::SelectedText) {
            return PluginResponse::String(
                self.engine.selected_reading_text(&self.source).unwrap_or_default(),
            );
        }
        if self.engine.replacement_preedit.is_some()
            && matches!(
                &q,
                PluginQuery::SearchHighlights { .. } | PluginQuery::SourceSearchHighlights { .. }
            )
        {
            return PluginResponse::DrawList(DrawList::new());
        }
        if let PluginQuery::HitTestEditTarget { x, y, offset_x, offset_y } = &q {
            return PluginResponse::EditHitTarget(
                self.engine.hit_test_byte(*x, *y, *offset_x, *offset_y).map(|byte_offset| {
                    ui::plugin::EditHitTarget::TextCaret { byte_offset, selection_scope: None }
                }),
            );
        }
        if let Some(resp) = self.engine.query_common(&q) {
            return resp;
        }
        match q {
            PluginQuery::NeedsSourceUpdate(gen_id) => {
                PluginResponse::Bool(self.needs_source_update(gen_id))
            }
            PluginQuery::SearchHighlights {
                query,
                match_case,
                use_regex,
                active_idx,
                match_color,
                inactive_color,
            } => PluginResponse::DrawList(self.query_search_highlights(
                &query,
                match_case,
                use_regex,
                active_idx,
                match_color,
                inactive_color,
            )),
            PluginQuery::SourceSearchHighlights {
                matches,
                source_generation,
                active_idx,
                match_color,
                inactive_color,
            } => PluginResponse::DrawList(if source_generation == self.cached_generation {
                self.source_search_highlights(&matches, active_idx, match_color, inactive_color)
            } else {
                DrawList::new()
            }),
            PluginQuery::PlanSemanticEdit {
                command,
                source_generation,
                cursor_byte,
                selection,
            } => PluginResponse::SemanticEdit(crate::commands::plan_semantic_edit(
                &self.source,
                source_generation,
                cursor_byte,
                selection,
                command,
            )),
            _ => PluginResponse::None,
        }
    }
}

// ===== MarkdownEditorViewFactory =====

pub struct MarkdownEditorViewFactory;

impl PluginFactory for MarkdownEditorViewFactory {
    fn name(&self) -> &str {
        "markdown_editor"
    }
    fn can_handle(&self, path: Option<&Path>) -> bool {
        // .md files default to the editor view.
        path.is_some_and(|p| p.extension().is_some_and(|e| e == "md" || e == "markdown"))
    }
    fn create(&self) -> Box<dyn ViewPlugin> {
        Box::new(MarkdownEditorView::new())
    }
}

/// Notora 使用的 Markdown 工厂；正文 H1 与产品元数据标题彼此独立。
pub struct NotoraMarkdownEditorViewFactory;

impl PluginFactory for NotoraMarkdownEditorViewFactory {
    fn name(&self) -> &str {
        "markdown_editor"
    }

    fn can_handle(&self, path: Option<&Path>) -> bool {
        MarkdownEditorViewFactory.can_handle(path)
    }

    fn create(&self) -> Box<dyn ViewPlugin> {
        Box::new(MarkdownEditorView::new())
    }
}
