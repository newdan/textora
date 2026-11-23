use super::*;

// ===== MarkdownEditorView =====

/// WYSIWYG markdown editor view — wraps PreviewEngine with editing support.
/// Differs from MarkdownView in that it accepts cursor position updates
/// and provides layout-aware editing queries.
pub struct MarkdownEditorView {
    pub(in crate::view) engine: PreviewEngine,
    pub(in crate::view) source: SourceSnapshot,
    pub(in crate::view) cached_generation: Option<u32>,
    pub(in crate::view) split_history: std::cell::RefCell<split_history::SplitHistory>,
    pending_commit_source: std::cell::RefCell<Option<PendingCommitSource>>,
}

struct PendingCommitSource {
    base_generation: u32,
    snapshot: SourceSnapshot,
}

impl Default for MarkdownEditorView {
    fn default() -> Self {
        Self::new()
    }
}

impl MarkdownEditorView {
    pub fn new() -> Self {
        let mut engine = PreviewEngine::new();
        let source = MarkdownSource::snapshot(String::new());
        engine.edit_source = Some(source.clone());
        Self {
            engine,
            source,
            cached_generation: None,
            split_history: Default::default(),
            pending_commit_source: Default::default(),
        }
    }

    pub fn set_source(&mut self, text: String, generation: u32) {
        self.synchronize_source(text, generation, ui::plugin::SyncReason::Force);
    }

    fn synchronize_source(
        &mut self,
        text: String,
        generation: u32,
        reason: ui::plugin::SyncReason,
    ) {
        let pending = self.pending_commit_source.get_mut().take();
        self.split_history.get_mut().synchronize(&self.source, &text);
        let text_changed = self.source.as_str() != text;
        let source_changed = text_changed || self.needs_source_update(generation);
        if source_changed {
            self.engine.set_preedit_text(String::new(), None);
        }
        if text_changed {
            let reusable = pending.filter(|pending| {
                reason == ui::plugin::SyncReason::Normal
                    && self.cached_generation == Some(pending.base_generation)
                    && generation == pending.base_generation.wrapping_add(1)
                    && pending.snapshot.as_bytes() == text.as_bytes()
            });
            self.source =
                reusable.map_or_else(|| MarkdownSource::snapshot(text), |pending| pending.snapshot);
            self.engine.edit_source = Some(self.source.clone());
            self.engine.mark_source_dirty();
        }
        self.engine.set_source_generation(generation);
        self.cached_generation = Some(generation);
        if source_changed {
            self.engine.prepare_current_source_layout(&self.source);
        }
    }

    pub fn needs_source_update(&self, generation: u32) -> bool {
        self.cached_generation != Some(generation)
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
        *self.pending_commit_source.borrow_mut() = None;
        self.split_history.borrow_mut().begin_request();
        if self.needs_source_update(request.source_generation) {
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
        if matches!(kind, ui::plugin::AugmentKind::Enter) {
            return self.plan_paragraph_break(request);
        }
        self.engine
            .augment_edit(request.cursor_byte, kind)
            .map_or(ui::plugin::EditPlan::UseDefault, |augmentation| {
                augmentation_edit_plan(request, augmentation)
            })
    }
}

impl MarkdownEditorView {
    fn plan_paragraph_break(&self, request: &ui::plugin::EditRequest) -> ui::plugin::EditPlan {
        let Some(enter) = crate::augmenter::augment_enter(&self.source, request.cursor_byte) else {
            return ui::plugin::EditPlan::UseDefault;
        };
        let plan = augmentation_edit_plan(request, enter.augmentation);
        if let ui::plugin::EditPlan::Apply(transaction) = &plan
            && let Some(snapshot) = enter.validated_source
            && self.engine.edit_ctx.as_ref().is_none_or(|context| context.preedit_text.is_none())
        {
            *self.pending_commit_source.borrow_mut() = Some(PendingCommitSource {
                base_generation: transaction.source_generation,
                snapshot,
            });
        }
        self.split_history.borrow_mut().remember(&self.source, request, &plan);
        plan
    }

    /// 清空或改写选区时，先在原始源码上计算结构保护擦除，再合成为单一原子事务。
    pub(in crate::view) fn plan_selection_edit(
        &self,
        request: &ui::plugin::EditRequest,
        selection: &std::ops::Range<usize>,
    ) -> ui::plugin::EditPlan {
        if self.source.get(selection.clone()).is_none() {
            return ui::plugin::EditPlan::UseDefault;
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
        if let ui::plugin::EditIntent::InsertText(text) = &request.intent {
            if let Some(erasure) =
                crate::augmenter::augment_erase_range(&self.source, selection.clone())
            {
                return protected_selection_insert_text_plan(
                    &self.source,
                    request,
                    selection,
                    erasure,
                    text,
                );
            }
            return selection_insert_text_plan(&self.source, request, selection, text, None);
        }
        if !matches!(
            request.intent,
            ui::plugin::EditIntent::InsertParagraphBreak | ui::plugin::EditIntent::InsertLineBreak
        ) {
            return ui::plugin::EditPlan::UseDefault;
        }
        let is_line_break = matches!(request.intent, ui::plugin::EditIntent::InsertLineBreak);
        if let Some(erasure) =
            crate::augmenter::augment_erase_range(&self.source, selection.clone())
        {
            return protected_selection_break_plan(
                &self.source,
                request.source_generation,
                erasure,
                is_line_break,
            );
        }
        let mut source_after_delete = self.source.as_str().to_owned();
        source_after_delete.replace_range(selection.clone(), "");
        let kind = if is_line_break {
            ui::plugin::AugmentKind::LineBreak
        } else {
            ui::plugin::AugmentKind::Enter
        };
        let Some(augmentation) = crate::augmenter::augment_source(
            &MarkdownSource::owned(source_after_delete),
            selection.start,
            kind,
        ) else {
            return ui::plugin::EditPlan::UseDefault;
        };
        selection_augmentation_edit_plan(request, selection, augmentation)
    }
}

fn selection_insert_text_plan(
    source: &MarkdownSource<'_>,
    request: &ui::plugin::EditRequest,
    selection: &std::ops::Range<usize>,
    text: &str,
    erased_source: Option<&MarkdownSource<'_>>,
) -> ui::plugin::EditPlan {
    crate::augmenter::augment_selected_text(source, selection.clone(), text, erased_source)
        .map_or(ui::plugin::EditPlan::UseDefault, |augmentation| {
            selection_augmentation_edit_plan(request, selection, augmentation)
        })
}

fn protected_selection_insert_text_plan(
    source: &MarkdownSource<'_>,
    request: &ui::plugin::EditRequest,
    selection: &std::ops::Range<usize>,
    erasure: ui::plugin::EditAugmentation,
    text: &str,
) -> ui::plugin::EditPlan {
    let Some(erased_range) = erasure.replace_range else {
        return ui::plugin::EditPlan::Consume;
    };
    let erased_text = erasure.insert_text.unwrap_or_default();
    let mut source_after_erase = source.as_str().to_owned();
    source_after_erase.replace_range(erased_range.clone(), &erased_text);
    let source_after_erase = MarkdownSource::owned(source_after_erase);

    if erased_text.is_empty()
        && erased_range.start <= selection.start
        && selection.end <= erased_range.end
        && !matches!(
            crate::augmenter::classify_source(&source_after_erase, erasure.cursor_byte_after),
            crate::augmenter::EnterContext::TableCell { .. }
        )
    {
        let erased_source = (&erased_range == selection).then_some(&source_after_erase);
        return selection_insert_text_plan(source, request, selection, text, erased_source);
    }

    if let Some(follow) = crate::augmenter::augment_source(
        &source_after_erase,
        erasure.cursor_byte_after,
        ui::plugin::AugmentKind::InsertText(text.to_owned()),
    ) {
        return combine_erasure_and_followup_plan(
            source,
            request.source_generation,
            erased_range,
            erased_text,
            follow,
        );
    }
    combine_erasure_and_literal_insert(
        request.source_generation,
        erased_range,
        erased_text,
        erasure.cursor_byte_after,
        text,
    )
}

fn protected_selection_break_plan(
    source: &str,
    source_generation: u32,
    erasure: ui::plugin::EditAugmentation,
    is_line_break: bool,
) -> ui::plugin::EditPlan {
    let Some(erased_range) = erasure.replace_range else {
        return ui::plugin::EditPlan::Consume;
    };
    let erased_text = erasure.insert_text.unwrap_or_default();
    let mut source_after_erase = source.to_owned();
    source_after_erase.replace_range(erased_range.clone(), &erased_text);

    let source_after_erase = MarkdownSource::owned(source_after_erase);
    if matches!(
        crate::augmenter::classify_source(&source_after_erase, erasure.cursor_byte_after),
        crate::augmenter::EnterContext::TableCell { .. }
    ) {
        return ui::plugin::EditPlan::Apply(ui::plugin::EditTransaction::replace(
            source_generation,
            erased_range,
            erased_text,
            erasure.cursor_byte_after,
        ));
    }

    let kind = if is_line_break {
        ui::plugin::AugmentKind::LineBreak
    } else {
        ui::plugin::AugmentKind::Enter
    };
    if let Some(follow) =
        crate::augmenter::augment_source(&source_after_erase, erasure.cursor_byte_after, kind)
    {
        return combine_erasure_and_followup_plan(
            source,
            source_generation,
            erased_range,
            erased_text,
            follow,
        );
    }
    let newline = if source.contains("\r\n") { "\r\n" } else { "\n" };
    combine_erasure_and_literal_insert(
        source_generation,
        erased_range,
        erased_text,
        erasure.cursor_byte_after,
        newline,
    )
}

fn combine_erasure_and_followup_plan(
    source: &str,
    source_generation: u32,
    erased_range: std::ops::Range<usize>,
    mut erased_text: String,
    follow: ui::plugin::EditAugmentation,
) -> ui::plugin::EditPlan {
    let follow_range = follow.replace_range.unwrap_or(0..0);
    let follow_text = follow.insert_text.unwrap_or_default();
    let erased_end_in_new = erased_range.start + erased_text.len();

    if follow_range.start >= erased_range.start && follow_range.end <= erased_end_in_new {
        let local_start = follow_range.start - erased_range.start;
        let local_end = follow_range.end - erased_range.start;
        erased_text.replace_range(local_start..local_end, &follow_text);
        return ui::plugin::EditPlan::Apply(ui::plugin::EditTransaction::replace(
            source_generation,
            erased_range,
            erased_text,
            follow.cursor_byte_after,
        ));
    }

    let delta = erased_text.len() as isize - (erased_range.end - erased_range.start) as isize;
    if follow_range.end <= erased_range.start && follow_range.end <= source.len() {
        return ui::plugin::EditPlan::Apply(ui::plugin::EditTransaction {
            source_generation,
            replacements: vec![
                ui::plugin::TextReplacement { range: follow_range, text: follow_text },
                ui::plugin::TextReplacement { range: erased_range, text: erased_text },
            ],
            selection_after: ui::plugin::EditSelection::Caret(follow.cursor_byte_after),
        });
    }
    if follow_range.start >= erased_end_in_new {
        let orig_start = (follow_range.start as isize - delta).max(0) as usize;
        let orig_end = (follow_range.end as isize - delta).max(orig_start as isize) as usize;
        if orig_end <= source.len() {
            return ui::plugin::EditPlan::Apply(ui::plugin::EditTransaction {
                source_generation,
                replacements: vec![
                    ui::plugin::TextReplacement { range: erased_range, text: erased_text },
                    ui::plugin::TextReplacement { range: orig_start..orig_end, text: follow_text },
                ],
                selection_after: ui::plugin::EditSelection::Caret(follow.cursor_byte_after),
            });
        }
    }
    ui::plugin::EditPlan::UseDefault
}

fn combine_erasure_and_literal_insert(
    source_generation: u32,
    erased_range: std::ops::Range<usize>,
    mut erased_text: String,
    insert_cursor: usize,
    inserted: &str,
) -> ui::plugin::EditPlan {
    let erased_end_in_new = erased_range.start + erased_text.len();
    if insert_cursor >= erased_range.start && insert_cursor <= erased_end_in_new {
        let local = insert_cursor - erased_range.start;
        erased_text.insert_str(local, inserted);
        return ui::plugin::EditPlan::Apply(ui::plugin::EditTransaction::replace(
            source_generation,
            erased_range,
            erased_text,
            insert_cursor + inserted.len(),
        ));
    }
    ui::plugin::EditPlan::UseDefault
}

impl ViewPlugin for MarkdownEditorView {
    fn name(&self) -> &str {
        "markdown_editor"
    }

    fn mode(&self) -> ui::plugin::PluginMode {
        ui::plugin::PluginMode::RichText
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

    fn sync_document(
        &mut self,
        generation: u32,
        reason: ui::plugin::SyncReason,
        load_source: &mut dyn FnMut() -> String,
    ) -> ui::plugin::SyncOutcome {
        if reason == ui::plugin::SyncReason::Normal && !self.needs_source_update(generation) {
            return ui::plugin::SyncOutcome::Unchanged;
        }
        self.synchronize_source(load_source(), generation, reason);
        ui::plugin::SyncOutcome::Updated
    }
}

impl ui::plugin::ViewPluginCapabilities for MarkdownEditorView {
    fn scrollable(&self) -> Option<&dyn ui::plugin::ScrollableView> {
        Some(&self.engine)
    }

    fn scrollable_mut(&mut self) -> Option<&mut dyn ui::plugin::ScrollableView> {
        Some(&mut self.engine)
    }

    fn outline(&self) -> Option<&dyn ui::plugin::OutlineProvider> {
        Some(&self.engine)
    }

    fn outline_mut(&mut self) -> Option<&mut dyn ui::plugin::OutlineProvider> {
        Some(&mut self.engine)
    }

    fn search(&self) -> Option<&dyn ui::plugin::SearchView> {
        Some(self)
    }

    fn selection(&self) -> Option<&dyn ui::plugin::SelectionView> {
        Some(self)
    }

    fn selection_mut(&mut self) -> Option<&mut dyn ui::plugin::SelectionView> {
        Some(self)
    }
    fn editable(&self) -> Option<&dyn ui::plugin::EditableView> {
        Some(self)
    }

    fn editable_mut(&mut self) -> Option<&mut dyn ui::plugin::EditableView> {
        Some(self)
    }

    fn source_spacing_protected_ranges(
        &self,
        source_generation: u32,
    ) -> Option<std::sync::Arc<[std::ops::Range<usize>]>> {
        (!self.needs_source_update(source_generation))
            .then(|| self.source.source_spacing_protected_ranges())
    }

    fn allows_editing(&self) -> bool {
        // WYSIWYG editor — app_renderer will enable
        // Copy/Paste/Undo context menu and scrollbar behaviour.
        true
    }

    fn needs_cursor_blink_wakeup(&self) -> bool {
        // App must still compute blink phase and forward it via SetCursorVisible.
        true
    }

    fn set_view_render_settings(&mut self, settings: ui::plugin::ViewRenderSettings) {
        self.engine.set_view_render_settings(settings);
    }

    fn set_view_text_spacing_mode(&mut self, mode: ui::typography::TextSpacingMode) {
        self.engine.set_view_text_spacing_mode(mode);
    }
}

impl ui::plugin::EditableView for MarkdownEditorView {
    fn plan_semantic_edit(
        &self,
        command: ui::plugin::SemanticEditCommand,
        source_generation: u32,
        cursor_byte: usize,
        selection: Option<std::ops::Range<usize>>,
    ) -> ui::plugin::SemanticEditPlan {
        *self.pending_commit_source.borrow_mut() = None;
        if self.needs_source_update(source_generation) {
            return ui::plugin::SemanticEditPlan::Unsupported;
        }
        crate::commands::plan_semantic_source(
            &self.source,
            source_generation,
            cursor_byte,
            selection,
            command,
        )
    }

    fn table_structure_capabilities(
        &self,
        source_generation: u32,
        cursor_byte: usize,
    ) -> Option<ui::plugin::TableStructureCapabilities> {
        if self.needs_source_update(source_generation) {
            return None;
        }
        crate::commands::snapshot_table_capabilities(&self.source, cursor_byte)
    }

    fn paste_preference(&self) -> PastePreference {
        PastePreference::SemanticMarkdown
    }

    fn key_intent_mapper(&self) -> Option<&dyn ui::plugin::KeyIntentMapper> {
        Some(self)
    }

    fn hit_test_caret(
        &self,
        x: f32,
        y: f32,
        offset_x: f32,
        offset_y: f32,
    ) -> Option<ui::plugin::VisualCaretPosition> {
        self.engine.hit_test_anchor(x, y, offset_x, offset_y).map(|anchor| {
            ui::plugin::VisualCaretPosition {
                byte_offset: anchor.byte.get(),
                affinity: anchor.affinity,
            }
        })
    }

    fn visual_move(
        &self,
        current_byte: usize,
        direction: ui::plugin::MoveDirection,
        target_x: Option<f32>,
    ) -> Option<ui::plugin::VisualCaretPosition> {
        self.engine.visual_move_anchor(current_byte, direction, target_x).map(|anchor| {
            ui::plugin::VisualCaretPosition {
                byte_offset: anchor.byte.get(),
                affinity: anchor.affinity,
            }
        })
    }

    fn hit_test_edit_target(
        &self,
        x: f32,
        y: f32,
        offset_x: f32,
        offset_y: f32,
    ) -> Option<Option<ui::plugin::EditHitTarget>> {
        Some(self.engine.hit_test_byte(x, y, offset_x, offset_y).map(|byte_offset| {
            ui::plugin::EditHitTarget::TextCaret { byte_offset, selection_scope: None }
        }))
    }

    fn cursor_screen_rect(&self, cursor_byte: usize) -> Option<(f32, f32, f32, f32)> {
        self.engine
            .edit_ctx
            .as_ref()
            .filter(|ctx| ctx.cursor_byte == cursor_byte)
            .and_then(|_| self.engine.visual_cursor_screen_pos())
            .or_else(|| self.engine.cursor_screen_pos_for_byte(cursor_byte))
    }

    fn set_cursor_byte(&mut self, cursor_byte: usize) {
        self.engine.handle_set_cursor_byte(cursor_byte);
    }

    fn set_visual_caret(&mut self, position: ui::plugin::VisualCaretPosition) {
        self.engine.handle_set_cursor_byte(position.byte_offset);
        if self.engine.cursor_affinity != position.affinity {
            self.engine.cursor_affinity = position.affinity;
            self.engine.cached_dl = None;
            self.engine.cached_vertices = None;
        }
    }

    fn reveal_cursor(&mut self, cursor_byte: usize) -> bool {
        self.engine.pending_cursor_reveal = Some(cursor_byte);
        self.engine.cached_dl = None;
        self.engine.cached_vertices = None;
        true
    }

    fn set_cursor_visible(&mut self, visible: bool) {
        self.engine.cursor_visible = visible;
    }

    fn set_preedit(&mut self, text: String, cursor: Option<(usize, usize)>) {
        if !text.is_empty() {
            *self.pending_commit_source.get_mut() = None;
        }
        self.engine.set_preedit_text(text, cursor);
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
    fn create(&self) -> Box<dyn ViewPluginCapabilities> {
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

    fn create(&self) -> Box<dyn ViewPluginCapabilities> {
        Box::new(MarkdownEditorView::new())
    }
}
