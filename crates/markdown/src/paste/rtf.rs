use encoding_rs::{Encoding, WINDOWS_1252};
use url::Url;

use super::{InlineSemantic, ListKind, RichBlock, RichDocument, RichInline};

pub(crate) const MAX_RTF_GROUP_DEPTH: usize = 256;
pub(crate) const MAX_RTF_CONTROL_WORD_BYTES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RtfPasteError {
    GroupDepthExceeded,
    ControlWordTooLong,
    InvalidHexEscape,
    InvalidControlArgument,
    UnmatchedGroupEnd,
    UnclosedGroup,
    DanglingBackslash,
    UnsupportedCodePage(i32),
    InvalidUnicodeCodeUnit(i32),
    InvalidUnicodeFallbackCount(i32),
    InvalidUnicodeSurrogateOrder,
    TextDecodingFailed,
}

enum RtfToken<'a> {
    GroupStart,
    GroupEnd { raw_unmatched: bool },
    Control { name: &'a str, argument: Option<i32>, delimiter_space: bool },
    EscapedByte(u8),
    Text(&'a [u8]),
}

struct RtfTokenizer<'a> {
    input: &'a [u8],
    cursor: usize,
    group_depth: usize,
}

impl<'a> RtfTokenizer<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self { input, cursor: 0, group_depth: 0 }
    }

    fn next_token(&mut self) -> Result<Option<RtfToken<'a>>, RtfPasteError> {
        let Some(&byte) = self.input.get(self.cursor) else {
            return if self.group_depth == 0 {
                Ok(None)
            } else {
                Err(RtfPasteError::UnclosedGroup)
            };
        };
        match byte {
            b'{' => self.group_start().map(Some),
            b'}' => self.group_end().map(Some),
            b'\\' => self.control().map(Some),
            _ => Ok(Some(self.text())),
        }
    }

    fn group_start(&mut self) -> Result<RtfToken<'a>, RtfPasteError> {
        if self.group_depth >= MAX_RTF_GROUP_DEPTH {
            return Err(RtfPasteError::GroupDepthExceeded);
        }
        self.group_depth += 1;
        self.cursor += 1;
        Ok(RtfToken::GroupStart)
    }

    fn group_end(&mut self) -> Result<RtfToken<'a>, RtfPasteError> {
        let raw_unmatched = self.group_depth == 0;
        if self.group_depth > 0 {
            self.group_depth -= 1;
        }
        self.cursor += 1;
        Ok(RtfToken::GroupEnd { raw_unmatched })
    }

    fn text(&mut self) -> RtfToken<'a> {
        let start = self.cursor;
        while !matches!(self.input.get(self.cursor), None | Some(b'{' | b'}' | b'\\')) {
            self.cursor += 1;
        }
        RtfToken::Text(&self.input[start..self.cursor])
    }

    fn control(&mut self) -> Result<RtfToken<'a>, RtfPasteError> {
        self.cursor += 1;
        let Some(&next) = self.input.get(self.cursor) else {
            return Err(RtfPasteError::DanglingBackslash);
        };
        if matches!(next, b'\\' | b'{' | b'}') {
            self.cursor += 1;
            return Ok(RtfToken::EscapedByte(next));
        }
        if next == b'\'' {
            return self.hex_escape();
        }
        if next.is_ascii_alphabetic() {
            return self.control_word();
        }
        let start = self.cursor;
        self.cursor += 1;
        let name = std::str::from_utf8(&self.input[start..self.cursor])
            .map_err(|_| RtfPasteError::InvalidControlArgument)?;
        Ok(RtfToken::Control { name, argument: None, delimiter_space: false })
    }

    fn hex_escape(&mut self) -> Result<RtfToken<'a>, RtfPasteError> {
        let digits = self
            .input
            .get(self.cursor + 1..self.cursor + 3)
            .ok_or(RtfPasteError::InvalidHexEscape)?;
        let high = hex_digit(digits[0]).ok_or(RtfPasteError::InvalidHexEscape)?;
        let low = hex_digit(digits[1]).ok_or(RtfPasteError::InvalidHexEscape)?;
        self.cursor += 3;
        Ok(RtfToken::EscapedByte((high << 4) | low))
    }

    fn control_word(&mut self) -> Result<RtfToken<'a>, RtfPasteError> {
        let name_start = self.cursor;
        while self.input.get(self.cursor).is_some_and(u8::is_ascii_alphabetic) {
            self.cursor += 1;
        }
        if self.cursor - name_start > MAX_RTF_CONTROL_WORD_BYTES {
            return Err(RtfPasteError::ControlWordTooLong);
        }
        let name = std::str::from_utf8(&self.input[name_start..self.cursor])
            .map_err(|_| RtfPasteError::InvalidControlArgument)?;
        let argument = self.control_argument()?;
        let delimiter_space = self.input.get(self.cursor) == Some(&b' ');
        if delimiter_space {
            self.cursor += 1;
        }
        Ok(RtfToken::Control { name, argument, delimiter_space })
    }

    fn control_argument(&mut self) -> Result<Option<i32>, RtfPasteError> {
        let start = self.cursor;
        if self.input.get(self.cursor) == Some(&b'-') {
            self.cursor += 1;
        }
        let digit_start = self.cursor;
        while self.input.get(self.cursor).is_some_and(u8::is_ascii_digit) {
            self.cursor += 1;
        }
        if self.cursor == digit_start {
            self.cursor = start;
            return Ok(None);
        }
        let argument = std::str::from_utf8(&self.input[start..self.cursor])
            .map_err(|_| RtfPasteError::InvalidControlArgument)?;
        argument.parse::<i32>().map(Some).map_err(|_| RtfPasteError::InvalidControlArgument)
    }
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RtfDestination {
    Normal,
    FieldInstruction,
    FieldResult,
    ListText,
    Skip,
}

#[derive(Clone, Copy)]
enum CodePageDeclaration {
    Undeclared,
    GenericAnsi,
    Explicit,
}

#[derive(Clone, Copy)]
enum StarredControl {
    None,
    Pending,
}

#[derive(Clone)]
struct RtfState {
    destination: RtfDestination,
    inline_styles: Vec<InlineSemantic>,
    unicode_fallback_bytes: usize,
    ansi_code_page: &'static Encoding,
    code_page_declaration: CodePageDeclaration,
    starred_control: StarredControl,
}

impl Default for RtfState {
    fn default() -> Self {
        Self {
            destination: RtfDestination::Normal,
            inline_styles: Vec::new(),
            unicode_fallback_bytes: 1,
            ansi_code_page: WINDOWS_1252,
            code_page_declaration: CodePageDeclaration::Undeclared,
            starred_control: StarredControl::None,
        }
    }
}

enum PendingUnicode {
    None,
    HighSurrogate(u16),
}

#[derive(Clone, Copy)]
enum GroupPosition {
    Start,
    Content,
}

struct GroupFrame {
    state: RtfState,
    role: GroupRole,
}

enum GroupRole {
    Ordinary,
    FieldInstruction { field_index: usize },
    FieldResult { field_index: usize },
    ListMarker { route: ListMarkerRoute, marker: String },
}

enum ListMarkerRoute {
    Document,
    Suppressed,
}

struct FieldContext {
    owner_depth: usize,
    closure: FieldClosure,
    progress: FieldProgress,
    instruction: String,
    result: Vec<RichInline>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FieldClosure {
    Group,
    Implicit,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FieldProgress {
    Started,
    InstructionOpen { depth: usize },
    InstructionClosed,
    ResultOpen { depth: usize },
    Ready,
    AwaitingCompatibilityClose,
}

#[derive(Clone, Copy)]
enum StyleSeparator {
    None,
    Pending,
}

impl FieldContext {
    fn new(owner_depth: usize, closure: FieldClosure) -> Self {
        Self {
            owner_depth,
            closure,
            progress: FieldProgress::Started,
            instruction: String::new(),
            result: Vec::new(),
        }
    }
}

struct RtfParser {
    state: RtfState,
    group_frames: Vec<GroupFrame>,
    group_position: GroupPosition,
    output: RichOutput,
    fields: Vec<FieldContext>,
    pending_unicode: PendingUnicode,
    fallback_bytes_remaining: usize,
    style_separator: StyleSeparator,
}

impl RtfParser {
    fn new() -> Self {
        Self {
            state: RtfState::default(),
            group_frames: Vec::new(),
            group_position: GroupPosition::Start,
            output: RichOutput::default(),
            fields: Vec::new(),
            pending_unicode: PendingUnicode::None,
            fallback_bytes_remaining: 0,
            style_separator: StyleSeparator::None,
        }
    }

    fn accept(&mut self, token: RtfToken<'_>) -> Result<(), RtfPasteError> {
        if self.awaiting_implicit_field_close() {
            return match token {
                RtfToken::Text(bytes) if bytes.iter().all(u8::is_ascii_whitespace) => Ok(()),
                RtfToken::GroupEnd { raw_unmatched: true } => self.end_group(true),
                _ => Err(RtfPasteError::UnclosedGroup),
            };
        }
        self.resolve_style_separator(&token)?;
        match token {
            RtfToken::GroupStart => self.start_group()?,
            RtfToken::GroupEnd { raw_unmatched } => self.end_group(raw_unmatched)?,
            RtfToken::Text(bytes) => {
                self.text(bytes)?;
                if bytes.iter().any(|byte| !byte.is_ascii_whitespace()) {
                    self.group_position = GroupPosition::Content;
                }
            }
            RtfToken::EscapedByte(byte) => {
                self.escaped_byte(byte)?;
                self.group_position = GroupPosition::Content;
            }
            RtfToken::Control { name, argument, delimiter_space } => {
                self.control(name, argument)?;
                self.arm_style_separator(name, argument, delimiter_space);
            }
        }
        Ok(())
    }

    fn awaiting_implicit_field_close(&self) -> bool {
        self.fields
            .last()
            .is_some_and(|field| field.progress == FieldProgress::AwaitingCompatibilityClose)
    }

    fn resolve_style_separator(&mut self, token: &RtfToken<'_>) -> Result<(), RtfPasteError> {
        let StyleSeparator::Pending =
            std::mem::replace(&mut self.style_separator, StyleSeparator::None)
        else {
            return Ok(());
        };
        if matches!(
            token,
            RtfToken::Control { name: "b" | "i" | "strike", argument, .. }
                if *argument != Some(0)
        ) {
            self.append_text(" ")?;
        }
        Ok(())
    }

    fn arm_style_separator(&mut self, name: &str, argument: Option<i32>, delimiter: bool) {
        let visible_destination =
            matches!(self.state.destination, RtfDestination::Normal | RtfDestination::FieldResult);
        if delimiter
            && argument == Some(0)
            && matches!(name, "b" | "i" | "strike")
            && self.fallback_bytes_remaining == 0
            && visible_destination
        {
            self.style_separator = StyleSeparator::Pending;
        }
    }

    fn start_group(&mut self) -> Result<(), RtfPasteError> {
        self.ensure_complete_unicode()?;
        if self.group_frames.len() >= MAX_RTF_GROUP_DEPTH {
            return Err(RtfPasteError::GroupDepthExceeded);
        }
        self.group_frames.push(GroupFrame { state: self.state.clone(), role: GroupRole::Ordinary });
        self.group_position = GroupPosition::Start;
        Ok(())
    }

    fn end_group(&mut self, raw_unmatched: bool) -> Result<(), RtfPasteError> {
        if matches!(self.pending_unicode, PendingUnicode::HighSurrogate(_)) {
            return Err(RtfPasteError::InvalidUnicodeSurrogateOrder);
        }
        if raw_unmatched && self.awaiting_implicit_field_close() {
            self.finish_field();
            self.fallback_bytes_remaining = 0;
            return Ok(());
        }
        if raw_unmatched {
            return Err(RtfPasteError::UnmatchedGroupEnd);
        }
        let closing_depth = self.group_frames.len();
        let parent = self.group_frames.pop().ok_or(RtfPasteError::UnmatchedGroupEnd)?;
        self.close_group_role(parent.role, closing_depth)?;
        self.state = parent.state;
        self.group_position = GroupPosition::Content;
        self.fallback_bytes_remaining = 0;
        if self.fields.last().is_some_and(|field| {
            field.closure == FieldClosure::Group && field.owner_depth == closing_depth
        }) {
            if !self.field_is_ready() {
                return Err(RtfPasteError::UnclosedGroup);
            }
            self.finish_field();
        }
        if let Some(field) = self.fields.last_mut()
            && field.closure == FieldClosure::Implicit
            && field.owner_depth == closing_depth
            && field.progress == FieldProgress::Ready
        {
            field.progress = FieldProgress::AwaitingCompatibilityClose;
        }
        Ok(())
    }

    fn close_group_role(
        &mut self,
        role: GroupRole,
        closing_depth: usize,
    ) -> Result<(), RtfPasteError> {
        let (field_index, expected, next) = match role {
            GroupRole::Ordinary => return Ok(()),
            GroupRole::ListMarker { route, marker } => {
                if matches!(route, ListMarkerRoute::Document) {
                    self.output.pending_list = list_kind(&marker);
                }
                return Ok(());
            }
            GroupRole::FieldInstruction { field_index } => (
                field_index,
                FieldProgress::InstructionOpen { depth: closing_depth },
                FieldProgress::InstructionClosed,
            ),
            GroupRole::FieldResult { field_index } => (
                field_index,
                FieldProgress::ResultOpen { depth: closing_depth },
                FieldProgress::Ready,
            ),
        };
        let field = self.fields.get_mut(field_index).ok_or(RtfPasteError::UnclosedGroup)?;
        if field.progress != expected {
            return Err(RtfPasteError::UnclosedGroup);
        }
        field.progress = next;
        Ok(())
    }

    fn field_is_ready(&self) -> bool {
        self.fields.last().is_some_and(|field| field.progress == FieldProgress::Ready)
    }

    fn finish_field(&mut self) {
        let Some(field) = self.fields.pop() else {
            return;
        };
        if field.result.is_empty() {
            return;
        }
        let inlines = match safe_hyperlink_destination(&field.instruction) {
            Some(destination) => {
                vec![RichInline::Link { destination, title: None, children: field.result }]
            }
            None => field.result,
        };
        self.append_routed_inlines(inlines);
    }

    fn text(&mut self, bytes: &[u8]) -> Result<(), RtfPasteError> {
        let skipped = self.fallback_bytes_remaining.min(bytes.len());
        self.fallback_bytes_remaining -= skipped;
        let bytes = &bytes[skipped..];
        if bytes.is_empty() || self.state.destination == RtfDestination::Skip {
            return Ok(());
        }
        let normalized = normalize_source_text(bytes);
        if normalized.is_empty() {
            return Ok(());
        }
        let text = decode_bytes(&normalized, self.state.ansi_code_page)?;
        self.append_text(&text)
    }

    fn escaped_byte(&mut self, byte: u8) -> Result<(), RtfPasteError> {
        if self.consume_fallback_character() || self.state.destination == RtfDestination::Skip {
            return Ok(());
        }
        let text = decode_bytes(&[byte], self.state.ansi_code_page)?;
        self.append_text(&text)
    }

    fn control(&mut self, name: &str, argument: Option<i32>) -> Result<(), RtfPasteError> {
        self.ensure_unicode_control_boundary(name)?;
        if name == "*" {
            self.state.starred_control = StarredControl::Pending;
            return Ok(());
        }
        if self.state.destination == RtfDestination::Skip {
            return Ok(());
        }
        self.apply_starred_destination(name);
        if self.state.destination == RtfDestination::Skip {
            return Ok(());
        }
        if name == "field" {
            self.start_field()?;
        }
        let result = self.apply_control(name, argument);
        self.group_position = GroupPosition::Content;
        result
    }

    fn apply_control(&mut self, name: &str, argument: Option<i32>) -> Result<(), RtfPasteError> {
        match name {
            "rtf" | "deff" | "field" => Ok(()),
            "ansi" => self.generic_ansi(),
            "ansicpg" => self.ansi_code_page(argument),
            "b" => self.style(InlineSemantic::Strong, argument),
            "i" => self.style(InlineSemantic::Emphasis, argument),
            "strike" => self.style(InlineSemantic::Strikethrough, argument),
            "plain" => {
                self.state.inline_styles.clear();
                Ok(())
            }
            "par" => self.paragraph(),
            "line" => self.line_break(),
            "tab" => self.visible_control("\t"),
            "cell" => self.cell(),
            "row" => self.paragraph(),
            "uc" => self.unicode_fallback(argument),
            "u" => self.unicode(argument),
            "fldinst" => self.destination(RtfDestination::FieldInstruction),
            "fldrslt" => self.destination(RtfDestination::FieldResult),
            "listtext" | "pntext" => self.destination(RtfDestination::ListText),
            "pict" => self.destination(RtfDestination::Skip),
            "fonttbl" | "colortbl" | "stylesheet" | "info" | "object" => {
                self.destination(RtfDestination::Skip)
            }
            "~" => self.visible_control("\u{a0}"),
            "_" => self.visible_control("\u{2011}"),
            "-" => self.optional_hyphen(),
            _ => Ok(()),
        }
    }

    fn ensure_unicode_control_boundary(&self, name: &str) -> Result<(), RtfPasteError> {
        if matches!(self.pending_unicode, PendingUnicode::None) || name == "u" {
            return Ok(());
        }
        if self.fallback_bytes_remaining > 0 && matches!(name, "tab" | "~" | "_" | "-") {
            return Ok(());
        }
        Err(RtfPasteError::InvalidUnicodeSurrogateOrder)
    }

    fn start_field(&mut self) -> Result<(), RtfPasteError> {
        let closure = match self.group_position {
            GroupPosition::Start => FieldClosure::Group,
            GroupPosition::Content => FieldClosure::Implicit,
        };
        self.fields.push(FieldContext::new(self.group_frames.len(), closure));
        Ok(())
    }

    fn apply_starred_destination(&mut self, name: &str) {
        if !matches!(self.state.starred_control, StarredControl::Pending) {
            return;
        }
        self.state.starred_control = StarredControl::None;
        if !matches!(name, "fldinst" | "fldrslt" | "listtext" | "pntext") {
            self.state.destination = RtfDestination::Skip;
        }
    }

    fn destination(&mut self, destination: RtfDestination) -> Result<(), RtfPasteError> {
        match destination {
            RtfDestination::FieldInstruction => self.open_field_instruction()?,
            RtfDestination::FieldResult => self.open_field_result()?,
            RtfDestination::ListText => self.open_list_marker()?,
            RtfDestination::Normal | RtfDestination::Skip => {}
        }
        self.state.destination = destination;
        Ok(())
    }

    fn open_field_instruction(&mut self) -> Result<(), RtfPasteError> {
        let depth = self.group_frames.len();
        let field_index = self.fields.len().checked_sub(1).ok_or(RtfPasteError::UnclosedGroup)?;
        let field = &self.fields[field_index];
        if field.progress != FieldProgress::Started || depth <= field.owner_depth {
            return Err(RtfPasteError::UnclosedGroup);
        }
        self.assign_group_role(GroupRole::FieldInstruction { field_index })?;
        self.fields[field_index].instruction.clear();
        self.fields[field_index].progress = FieldProgress::InstructionOpen { depth };
        Ok(())
    }

    fn open_field_result(&mut self) -> Result<(), RtfPasteError> {
        let depth = self.group_frames.len();
        let field_index = self.fields.len().checked_sub(1).ok_or(RtfPasteError::UnclosedGroup)?;
        let field = &self.fields[field_index];
        if field.progress != FieldProgress::InstructionClosed || depth <= field.owner_depth {
            return Err(RtfPasteError::UnclosedGroup);
        }
        self.assign_group_role(GroupRole::FieldResult { field_index })?;
        self.fields[field_index].result.clear();
        self.fields[field_index].progress = FieldProgress::ResultOpen { depth };
        Ok(())
    }

    fn assign_group_role(&mut self, role: GroupRole) -> Result<(), RtfPasteError> {
        let frame = self.group_frames.last_mut().ok_or(RtfPasteError::UnclosedGroup)?;
        if !matches!(&frame.role, GroupRole::Ordinary) {
            return Err(RtfPasteError::UnclosedGroup);
        }
        frame.role = role;
        Ok(())
    }

    fn open_list_marker(&mut self) -> Result<(), RtfPasteError> {
        let route = if self.state.destination == RtfDestination::Normal && self.fields.is_empty() {
            ListMarkerRoute::Document
        } else {
            ListMarkerRoute::Suppressed
        };
        self.assign_group_role(GroupRole::ListMarker { route, marker: String::new() })
    }

    fn append_list_marker(&mut self, text: &str) {
        if let Some(GroupRole::ListMarker { marker, .. }) =
            self.group_frames.iter_mut().rev().find_map(|frame| match &mut frame.role {
                role @ GroupRole::ListMarker { .. } => Some(role),
                _ => None,
            })
        {
            marker.push_str(text);
        }
    }

    fn generic_ansi(&mut self) -> Result<(), RtfPasteError> {
        self.state.ansi_code_page = WINDOWS_1252;
        self.state.code_page_declaration = CodePageDeclaration::GenericAnsi;
        Ok(())
    }

    fn ansi_code_page(&mut self, argument: Option<i32>) -> Result<(), RtfPasteError> {
        let code_page = argument.ok_or(RtfPasteError::InvalidControlArgument)?;
        if code_page == 1252 {
            self.state.ansi_code_page = WINDOWS_1252;
            self.state.code_page_declaration = CodePageDeclaration::Explicit;
            return Ok(());
        }
        if matches!(self.state.code_page_declaration, CodePageDeclaration::GenericAnsi) {
            return Ok(());
        }
        Err(RtfPasteError::UnsupportedCodePage(code_page))
    }

    fn style(
        &mut self,
        semantic: InlineSemantic,
        argument: Option<i32>,
    ) -> Result<(), RtfPasteError> {
        self.state.inline_styles.retain(|current| *current != semantic);
        if argument != Some(0) {
            self.state.inline_styles.push(semantic);
            self.state.inline_styles.sort_by_key(semantic_order);
        }
        Ok(())
    }

    fn paragraph(&mut self) -> Result<(), RtfPasteError> {
        self.ensure_complete_unicode()?;
        match self.state.destination {
            RtfDestination::Normal => self.output.finish_paragraph(),
            RtfDestination::FieldInstruction => {
                if let Some(field) = self.fields.last_mut() {
                    field.instruction.push('\n');
                }
            }
            RtfDestination::FieldResult => {
                if let Some(field) = self.fields.last_mut() {
                    field.result.push(RichInline::LineBreak);
                }
            }
            RtfDestination::ListText | RtfDestination::Skip => {}
        }
        Ok(())
    }

    fn line_break(&mut self) -> Result<(), RtfPasteError> {
        self.ensure_complete_unicode()?;
        match self.state.destination {
            RtfDestination::Normal => self.output.append_inline(RichInline::LineBreak),
            RtfDestination::FieldInstruction => {
                if let Some(field) = self.fields.last_mut() {
                    field.instruction.push(' ');
                }
            }
            RtfDestination::FieldResult => {
                if let Some(field) = self.fields.last_mut() {
                    field.result.push(RichInline::LineBreak);
                }
            }
            RtfDestination::ListText => self.append_list_marker(" "),
            RtfDestination::Skip => {}
        }
        Ok(())
    }

    fn visible_control(&mut self, text: &str) -> Result<(), RtfPasteError> {
        if self.consume_fallback_character() {
            return Ok(());
        }
        self.append_text(text)
    }

    fn optional_hyphen(&mut self) -> Result<(), RtfPasteError> {
        self.consume_fallback_character();
        Ok(())
    }

    fn cell(&mut self) -> Result<(), RtfPasteError> {
        self.ensure_complete_unicode()?;
        match self.state.destination {
            RtfDestination::Normal => self.output.pending_cell_separator = true,
            RtfDestination::FieldInstruction => {
                if let Some(field) = self.fields.last_mut() {
                    field.instruction.push(' ');
                }
            }
            RtfDestination::FieldResult => self.append_text("\t")?,
            RtfDestination::ListText => self.append_list_marker("\t"),
            RtfDestination::Skip => {}
        }
        Ok(())
    }

    fn unicode_fallback(&mut self, argument: Option<i32>) -> Result<(), RtfPasteError> {
        let count = argument.ok_or(RtfPasteError::InvalidControlArgument)?;
        self.state.unicode_fallback_bytes = usize::try_from(count)
            .map_err(|_| RtfPasteError::InvalidUnicodeFallbackCount(count))?;
        Ok(())
    }

    fn unicode(&mut self, argument: Option<i32>) -> Result<(), RtfPasteError> {
        let value = argument.ok_or(RtfPasteError::InvalidControlArgument)?;
        let unit = signed_utf16_unit(value)?;
        self.fallback_bytes_remaining = self.state.unicode_fallback_bytes;
        match (&self.pending_unicode, unit) {
            (PendingUnicode::None, 0xD800..=0xDBFF) => {
                self.pending_unicode = PendingUnicode::HighSurrogate(unit);
                Ok(())
            }
            (PendingUnicode::HighSurrogate(high), 0xDC00..=0xDFFF) => {
                let character = char::decode_utf16([*high, unit])
                    .next()
                    .and_then(Result::ok)
                    .ok_or(RtfPasteError::InvalidUnicodeSurrogateOrder)?;
                self.pending_unicode = PendingUnicode::None;
                self.append_text(&character.to_string())
            }
            (PendingUnicode::None, 0xDC00..=0xDFFF) | (PendingUnicode::HighSurrogate(_), _) => {
                Err(RtfPasteError::InvalidUnicodeSurrogateOrder)
            }
            (PendingUnicode::None, _) => {
                let character = char::from_u32(u32::from(unit))
                    .ok_or(RtfPasteError::InvalidUnicodeSurrogateOrder)?;
                self.append_text(&character.to_string())
            }
        }
    }

    fn append_text(&mut self, text: &str) -> Result<(), RtfPasteError> {
        self.ensure_complete_unicode()?;
        match self.state.destination {
            RtfDestination::Normal => {
                self.output.append_styled_text(text, &self.state.inline_styles)
            }
            RtfDestination::FieldInstruction => {
                if let Some(field) = self.fields.last_mut() {
                    field.instruction.push_str(text);
                }
            }
            RtfDestination::FieldResult => {
                if let Some(field) = self.fields.last_mut() {
                    append_styled_text(&mut field.result, text, &self.state.inline_styles);
                }
            }
            RtfDestination::ListText => self.append_list_marker(text),
            RtfDestination::Skip => {}
        }
        Ok(())
    }

    fn append_routed_inlines(&mut self, inlines: Vec<RichInline>) {
        match self.state.destination {
            RtfDestination::Normal => self.output.append_inlines(inlines),
            RtfDestination::FieldResult => {
                if let Some(field) = self.fields.last_mut() {
                    for inline in inlines {
                        append_merging_inline(&mut field.result, inline);
                    }
                }
            }
            RtfDestination::FieldInstruction | RtfDestination::ListText | RtfDestination::Skip => {}
        }
    }

    fn consume_fallback_character(&mut self) -> bool {
        if self.fallback_bytes_remaining == 0 {
            return false;
        }
        self.fallback_bytes_remaining -= 1;
        true
    }

    fn ensure_complete_unicode(&self) -> Result<(), RtfPasteError> {
        match self.pending_unicode {
            PendingUnicode::None => Ok(()),
            PendingUnicode::HighSurrogate(_) => Err(RtfPasteError::InvalidUnicodeSurrogateOrder),
        }
    }

    fn finish(mut self) -> Result<RichDocument, RtfPasteError> {
        self.ensure_complete_unicode()?;
        if !self.group_frames.is_empty() {
            return Err(RtfPasteError::UnclosedGroup);
        }
        if !self.fields.is_empty() {
            return Err(RtfPasteError::UnclosedGroup);
        }
        self.output.finish_paragraph();
        Ok(RichDocument::new(self.output.blocks))
    }
}

pub(crate) fn parse_rtf(input: &[u8]) -> Result<RichDocument, RtfPasteError> {
    let mut tokenizer = RtfTokenizer::new(input);
    let mut parser = RtfParser::new();
    while let Some(token) = tokenizer.next_token()? {
        parser.accept(token)?;
    }
    parser.finish()
}

#[derive(Default)]
struct RichOutput {
    blocks: Vec<RichBlock>,
    paragraph: Vec<RichInline>,
    pending_list: Option<ListKind>,
    pending_cell_separator: bool,
}

impl RichOutput {
    fn append_styled_text(&mut self, text: &str, styles: &[InlineSemantic]) {
        self.append_cell_separator();
        append_styled_text(&mut self.paragraph, text, styles);
    }

    fn append_inline(&mut self, inline: RichInline) {
        self.append_cell_separator();
        append_merging_inline(&mut self.paragraph, inline);
    }

    fn append_inlines(&mut self, inlines: Vec<RichInline>) {
        for inline in inlines {
            self.append_inline(inline);
        }
    }

    fn append_cell_separator(&mut self) {
        if !self.pending_cell_separator {
            return;
        }
        self.pending_cell_separator = false;
        if !self.paragraph.is_empty() {
            append_merging_inline(&mut self.paragraph, RichInline::Text("\t".into()));
        }
    }

    fn finish_paragraph(&mut self) {
        self.pending_cell_separator = false;
        let content = std::mem::take(&mut self.paragraph);
        if content.is_empty() {
            self.pending_list = None;
            return;
        }
        let Some(kind) = self.pending_list.take() else {
            self.blocks.push(RichBlock::Paragraph(content));
            return;
        };
        let item = vec![RichBlock::Paragraph(content)];
        if let Some(RichBlock::List { kind: existing, items }) = self.blocks.last_mut()
            && list_kind_continues(*existing, kind, items.len())
        {
            items.push(item);
            return;
        }
        self.blocks.push(RichBlock::List { kind, items: vec![item] });
    }
}

fn list_kind_continues(existing: ListKind, incoming: ListKind, item_count: usize) -> bool {
    match (existing, incoming) {
        (ListKind::Unordered, ListKind::Unordered) => true,
        (ListKind::Ordered { start }, ListKind::Ordered { start: next }) => {
            u64::try_from(item_count).ok().and_then(|count| start.checked_add(count)) == Some(next)
        }
        _ => false,
    }
}

fn append_styled_text(output: &mut Vec<RichInline>, text: &str, styles: &[InlineSemantic]) {
    if text.is_empty() {
        return;
    }
    let mut nested = vec![RichInline::Text(text.to_owned())];
    for style in styles.iter().rev() {
        nested = vec![wrap_inline(*style, nested)];
    }
    for inline in nested {
        append_merging_inline(output, inline);
    }
}

fn append_merging_inline(output: &mut Vec<RichInline>, inline: RichInline) {
    match (output.last_mut(), inline) {
        (Some(RichInline::Text(existing)), RichInline::Text(text)) => existing.push_str(&text),
        (Some(RichInline::Strong(existing)), RichInline::Strong(mut incoming))
        | (Some(RichInline::Emphasis(existing)), RichInline::Emphasis(mut incoming))
        | (Some(RichInline::Strikethrough(existing)), RichInline::Strikethrough(mut incoming)) => {
            merge_inline_children(existing, &mut incoming);
        }
        (_, inline) => output.push(inline),
    }
}

fn merge_inline_children(existing: &mut Vec<RichInline>, incoming: &mut Vec<RichInline>) {
    if incoming.is_empty() {
        return;
    }
    let first = incoming.remove(0);
    append_merging_inline(existing, first);
    existing.append(incoming);
}

fn wrap_inline(style: InlineSemantic, children: Vec<RichInline>) -> RichInline {
    match style {
        InlineSemantic::Strong => RichInline::Strong(children),
        InlineSemantic::Emphasis => RichInline::Emphasis(children),
        InlineSemantic::Strikethrough => RichInline::Strikethrough(children),
    }
}

fn semantic_order(style: &InlineSemantic) -> u8 {
    match style {
        InlineSemantic::Strong => 0,
        InlineSemantic::Emphasis => 1,
        InlineSemantic::Strikethrough => 2,
    }
}

fn signed_utf16_unit(value: i32) -> Result<u16, RtfPasteError> {
    if !(-32768..=65535).contains(&value) {
        return Err(RtfPasteError::InvalidUnicodeCodeUnit(value));
    }
    Ok(value as i16 as u16)
}

fn normalize_source_text(bytes: &[u8]) -> Vec<u8> {
    let mut normalized = Vec::with_capacity(bytes.len());
    let mut after_line_break = false;
    for &byte in bytes {
        if matches!(byte, b'\r' | b'\n') {
            after_line_break = true;
            continue;
        }
        if after_line_break && matches!(byte, b' ' | b'\t') {
            continue;
        }
        after_line_break = false;
        normalized.push(byte);
    }
    normalized
}

fn decode_bytes(bytes: &[u8], encoding: &'static Encoding) -> Result<String, RtfPasteError> {
    let (decoded, had_errors) = encoding.decode_without_bom_handling(bytes);
    if had_errors {
        return Err(RtfPasteError::TextDecodingFailed);
    }
    Ok(decoded.into_owned())
}

fn safe_hyperlink_destination(instruction: &str) -> Option<String> {
    let instruction = instruction.trim();
    let keyword_end = instruction.find(char::is_whitespace).unwrap_or(instruction.len());
    if !instruction[..keyword_end].eq_ignore_ascii_case("HYPERLINK") {
        return None;
    }
    let argument = instruction[keyword_end..].trim_start();
    let destination = quoted_argument(argument)?;
    if destination.chars().any(|character| character.is_ascii_control()) {
        return None;
    }
    let parsed = Url::parse(destination).ok()?;
    matches!(parsed.scheme(), "http" | "https" | "mailto").then(|| parsed.to_string())
}

fn quoted_argument(argument: &str) -> Option<&str> {
    let argument = argument.strip_prefix('"')?;
    let end = argument.find('"')?;
    Some(&argument[..end])
}

fn list_kind(marker: &str) -> Option<ListKind> {
    let marker = marker.trim();
    let digits = marker.chars().take_while(char::is_ascii_digit).collect::<String>();
    if !digits.is_empty() {
        return digits.parse().ok().map(|start| ListKind::Ordered { start });
    }
    (!marker.is_empty()).then_some(ListKind::Unordered)
}

#[cfg(test)]
#[path = "rtf_tests.rs"]
mod tests;
