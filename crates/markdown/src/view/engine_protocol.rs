use super::*;

impl<S: BlockSource> PreviewEngine<S> {
    /// 编辑干预：Enter/Backspace/Tab/InsertText 的 markdown 感知行为。委派到
    /// [`crate::augmenter`]（2026-07-06 方案阶段 2b/2c 抽出）。
    pub fn augment_edit(
        &self,
        current_byte: usize,
        kind: ui::plugin::AugmentKind,
    ) -> Option<ui::plugin::EditAugmentation> {
        let source = self.edit_source.as_deref()?;
        crate::augmenter::augment_edit(source, current_byte, kind)
    }

    /// 判断光标是否在某个 span 内。
    pub fn cursor_in_span(&self, span: &crate::builder::StyleSpan) -> bool {
        let Some(ctx) = self.edit_ctx.as_ref() else {
            return false;
        };
        crate::edit::cursor_in_span(span, ctx.cursor_byte)
    }

    // ── Common query/message helpers ──

    /// Handle queries common to all markdown views (preview + editor).
    /// Returns `Some(response)` for recognized queries, `None` for view-specific ones.
    pub fn query_common(&self, q: &PluginQuery) -> Option<PluginResponse> {
        match q {
            PluginQuery::ScrollY => Some(PluginResponse::Float(self.scroll_y)),
            PluginQuery::ContentHeight => Some(PluginResponse::Float(self.content_height)),
            PluginQuery::NeedsSourceUpdate(_) => None,
            PluginQuery::TOCHeadings => Some(PluginResponse::Headings(
                self.headings()
                    .iter()
                    .map(|h| ui::plugin::HeadingEntry {
                        title: h.text.clone(),
                        y: h.y_offset,
                        level: h.level,
                    })
                    .collect(),
            )),
            PluginQuery::CurrentHeadingIndex(scroll_y) => Some(PluginResponse::Position(
                self.current_heading_index(*scroll_y).map(|i| (i, 0)),
            )),
            PluginQuery::HasSelection => Some(PluginResponse::Bool(self.has_selection())),
            PluginQuery::SelectedText => {
                Some(PluginResponse::String(self.selected_text().unwrap_or_default()))
            }
            PluginQuery::SelCursor => Some(PluginResponse::Position(
                self.sel.cursor.map(|p| (p.flat_line_idx, p.grapheme_pos)),
            )),
            PluginQuery::SelectionRange => Some(PluginResponse::PositionPair(
                self.selection_source_range().map(|(start, end)| ((start, 0), (end, 0))),
            )),
            PluginQuery::HitTest { x, y, offset_x, offset_y } => Some(PluginResponse::Position(
                self.hit_test(*x, *y, *offset_x, *offset_y)
                    .map(|p| (p.flat_line_idx, p.grapheme_pos)),
            )),
            PluginQuery::SelectionHighlights(color) => {
                Some(PluginResponse::DrawList(self.selection_highlights(*color)))
            }
            PluginQuery::FlatLines => Some(PluginResponse::FlatLines(
                self.flat_lines()
                    .iter()
                    .map(|line| {
                        let grapheme_count = crate::grapheme_map::grapheme_count(&line.text);
                        ui::plugin::FlatLine {
                            text: line.text.clone(),
                            grapheme_count,
                            rect: line.rect,
                            grapheme_x: (0..=grapheme_count)
                                .map(|grapheme| crate::layout::grapheme_x(line, grapheme))
                                .collect(),
                        }
                    })
                    .collect(),
            )),
            PluginQuery::WordAtPos(li, gp) => {
                let (s, e) = self.word_at_pos(ViewPos { flat_line_idx: *li, grapheme_pos: *gp });
                Some(PluginResponse::PositionPair(Some((
                    (s.flat_line_idx, s.grapheme_pos),
                    (e.flat_line_idx, e.grapheme_pos),
                ))))
            }
            PluginQuery::LineRangeAtPos(li, gp) => {
                let (s, e) =
                    self.line_range_at_pos(ViewPos { flat_line_idx: *li, grapheme_pos: *gp });
                Some(PluginResponse::PositionPair(Some((
                    (s.flat_line_idx, s.grapheme_pos),
                    (e.flat_line_idx, e.grapheme_pos),
                ))))
            }
            PluginQuery::CursorScreenPos(byte) => Some(PluginResponse::CursorScreenRect(
                self.edit_ctx
                    .as_ref()
                    .filter(|ctx| ctx.cursor_byte == *byte)
                    .and_then(|_| self.visual_cursor_screen_pos())
                    .or_else(|| self.cursor_screen_pos_for_byte(*byte)),
            )),
            PluginQuery::HitTestByte { x, y, offset_x, offset_y } => {
                Some(PluginResponse::BytePosition(self.hit_test_byte(*x, *y, *offset_x, *offset_y)))
            }
            PluginQuery::HitTestVisualCaret { x, y, offset_x, offset_y } => {
                Some(PluginResponse::VisualCaretPosition(
                    self.hit_test_anchor(*x, *y, *offset_x, *offset_y).map(|anchor| {
                        VisualCaretPosition { byte_offset: anchor.byte, affinity: anchor.affinity }
                    }),
                ))
            }
            PluginQuery::VisualMove { current_byte, direction, target_x } => {
                Some(PluginResponse::BytePosition(self.visual_move(
                    *current_byte,
                    *direction,
                    *target_x,
                )))
            }
            PluginQuery::VisualMoveCaret { current_byte, direction, target_x } => {
                Some(PluginResponse::VisualCaretPosition(
                    self.visual_move_anchor(*current_byte, *direction, *target_x).map(|anchor| {
                        VisualCaretPosition { byte_offset: anchor.byte, affinity: anchor.affinity }
                    }),
                ))
            }

            _ => None,
        }
    }

    /// Handle messages common to all markdown views.
    /// Returns `Some(result)` for recognized messages, `None` for view-specific ones.
    pub fn handle_message_common(&mut self, msg: &PluginMessage) -> Option<bool> {
        match msg {
            PluginMessage::Scroll { delta, viewport_h } => Some(self.scroll(*delta, *viewport_h)),
            PluginMessage::ScrollToHeading(index) => {
                self.scroll_to_heading(*index);
                Some(true)
            }
            PluginMessage::UpdateSource { .. } => None,
            PluginMessage::SetSelCursor(pos) => {
                self.set_sel_cursor(*pos);
                Some(true)
            }
            PluginMessage::SetSelAnchor(pos) => {
                self.set_sel_anchor(*pos);
                Some(true)
            }
            PluginMessage::SetSelCursorByte(byte) => {
                self.set_sel_cursor_byte(*byte);
                Some(true)
            }
            PluginMessage::SetSelAnchorByte(byte) => {
                self.set_sel_anchor_byte(*byte);
                Some(true)
            }
            PluginMessage::ClearSelection => {
                self.clear_selection();
                Some(true)
            }
            PluginMessage::SelectAll => {
                self.select_all();
                Some(true)
            }
            PluginMessage::SetRenderSettings {
                font_family,
                font_size,
                line_height,
                toc_max_depth,
                markdown_first_line_indent,
            } => {
                if self.base_font_family != *font_family {
                    self.base_font_family.clone_from(font_family);
                    if !matches!(self.dirty, EngineDirty::SourceChanged) {
                        self.dirty = EngineDirty::StyleChanged;
                    }
                    self.cached_dl = None;
                    self.cached_vertices = None;
                }
                self.base_font_size = *font_size;
                self.base_line_height = *line_height;
                self.toc_max_depth = *toc_max_depth;
                self.markdown_first_line_indent = *markdown_first_line_indent;
                Some(true)
            }
            PluginMessage::SetTextSpacingMode(mode) => {
                if self.text_spacing_mode == *mode {
                    return Some(true);
                }
                self.text_spacing_mode = *mode;
                self.dirty = EngineDirty::StyleChanged;
                self.cached_dl = None;
                self.cached_vertices = None;
                Some(true)
            }
            PluginMessage::SetCursorByte(byte) => {
                self.handle_set_cursor_byte(*byte);
                Some(true)
            }
            PluginMessage::SetVisualCaret(position) => {
                self.handle_set_cursor_byte(position.byte_offset);
                if self.cursor_affinity != position.affinity {
                    self.cursor_affinity = position.affinity;
                    self.cached_dl = None;
                    self.cached_vertices = None;
                }
                Some(true)
            }
            PluginMessage::RevealCursor(byte) => {
                self.pending_cursor_reveal = Some(*byte);
                self.cached_dl = None;
                self.cached_vertices = None;
                Some(true)
            }
            PluginMessage::SetPreedit { text, cursor } => {
                self.set_preedit_text(text.clone(), *cursor);
                Some(true)
            }
            PluginMessage::SetCursorVisible(visible) => {
                self.cursor_visible = *visible;
                Some(true)
            }
            _ => None,
        }
    }
}
