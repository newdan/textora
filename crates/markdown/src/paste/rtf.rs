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
        {
            if field.progress == FieldProgress::Ready {
                field.progress = FieldProgress::AwaitingCompatibilityClose;
            }
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
mod tests {
    use super::{
        MAX_RTF_CONTROL_WORD_BYTES, MAX_RTF_GROUP_DEPTH, RtfPasteError, parse_rtf,
        safe_hyperlink_destination,
    };
    use crate::paste::writer::write_markdown;

    #[test]
    fn parses_paragraphs_unicode_inline_styles_and_hyperlinks() {
        let input = br#"{\rtf1\ansi\ansicpg1252
            First \b bold\b0 \i italic\i0\par
            Unicode \u20320?\u22909? \field{\*\fldinst HYPERLINK "https://example.com"}{\fldrslt link}}
        }"#;
        let document = parse_rtf(input).expect("supported RTF fixture");
        assert_eq!(
            write_markdown(&document),
            "First **bold** *italic*\n\nUnicode 你好 [link](https://example.com/)"
        );
    }

    #[test]
    fn excessive_group_depth_returns_a_typed_error() {
        let input = "{".repeat(MAX_RTF_GROUP_DEPTH + 1);
        assert_eq!(parse_rtf(input.as_bytes()), Err(RtfPasteError::GroupDepthExceeded));
    }

    #[test]
    fn exact_group_and_control_word_limits_are_accepted() {
        let groups = "{".repeat(MAX_RTF_GROUP_DEPTH);
        let closes = "}".repeat(MAX_RTF_GROUP_DEPTH);
        let document = parse_rtf(format!("{groups}x{closes}").as_bytes())
            .expect("group depth at the limit is valid");
        assert_eq!(write_markdown(&document), "x");

        let control_word = "a".repeat(MAX_RTF_CONTROL_WORD_BYTES);
        parse_rtf(format!("{{\\{control_word}}}").as_bytes())
            .expect("control word length at the limit is valid");
    }

    #[test]
    fn cells_and_rows_degrade_to_ordered_visible_text() {
        let document =
            parse_rtf(br"{\rtf1 a\cell b\cell\row}").expect("table controls degrade safely");
        assert_eq!(write_markdown(&document), "a\tb");
    }

    #[test]
    fn malformed_tokens_return_typed_errors() {
        assert_eq!(parse_rtf(br"{\rtf1 text"), Err(RtfPasteError::UnclosedGroup));
        assert_eq!(parse_rtf(br"{\rtf1 \'zz}"), Err(RtfPasteError::InvalidHexEscape));
        assert_eq!(parse_rtf(br"}"), Err(RtfPasteError::UnmatchedGroupEnd));
        assert_eq!(parse_rtf(br"{\rtf1 \"), Err(RtfPasteError::DanglingBackslash));
    }

    #[test]
    fn oversized_control_words_and_arguments_return_typed_errors() {
        let oversized_word = "a".repeat(MAX_RTF_CONTROL_WORD_BYTES + 1);
        let input = format!("{{\\{oversized_word}}}");
        assert_eq!(parse_rtf(input.as_bytes()), Err(RtfPasteError::ControlWordTooLong));
        assert_eq!(parse_rtf(br"{\rtf2147483648}"), Err(RtfPasteError::InvalidControlArgument));
    }

    #[test]
    fn signed_and_large_supported_arguments_are_not_truncated() {
        let document = parse_rtf(br"{\rtf1\b-1 bold\b0\uc2147483647\u65?}")
            .expect("in-range signed arguments remain supported");
        assert_eq!(write_markdown(&document), "**bold**A");
    }

    #[test]
    fn unicode_fallback_and_strike_are_decoded_once() {
        let document = parse_rtf(br"{\rtf1\uc1 \u-10180?\u-8435? \strike gone\strike0}")
            .expect("signed UTF-16 units and strike are supported");
        assert_eq!(write_markdown(&document), "🌍 ~~gone~~");
    }

    #[test]
    fn unicode_fallback_counts_text_and_control_tokens_exactly() {
        let document = parse_rtf(br"{\rtf1\uc0\u65? \uc2\u66\'3f?X \uc1\u67\tab Y}")
            .expect("fallback tokens are bounded by uc");
        assert_eq!(write_markdown(&document), "A? BX CY");
    }

    #[test]
    fn invalid_surrogate_order_returns_a_typed_error() {
        assert_eq!(
            parse_rtf(br"{\rtf1\u-8435?}"),
            Err(RtfPasteError::InvalidUnicodeSurrogateOrder)
        );
        assert_eq!(
            parse_rtf(br"{\rtf1\u-10180?x}"),
            Err(RtfPasteError::InvalidUnicodeSurrogateOrder)
        );
        assert_eq!(
            parse_rtf(br"{\rtf1\u70000?}"),
            Err(RtfPasteError::InvalidUnicodeCodeUnit(70000))
        );
        assert_eq!(
            parse_rtf(br"{\rtf1\uc-1 text}"),
            Err(RtfPasteError::InvalidUnicodeFallbackCount(-1))
        );
    }

    #[test]
    fn surrogate_pairs_cannot_cross_semantic_or_group_boundaries() {
        assert_eq!(
            parse_rtf(br"{\rtf1\u-10180?\b\u-8435?}"),
            Err(RtfPasteError::InvalidUnicodeSurrogateOrder)
        );
        assert_eq!(
            parse_rtf(br"{\rtf1\u-10180?{\u-8435?}}"),
            Err(RtfPasteError::InvalidUnicodeSurrogateOrder)
        );
        assert_eq!(
            parse_rtf(br"{\rtf1\u-10180?\fldinst\u-8435?}"),
            Err(RtfPasteError::InvalidUnicodeSurrogateOrder)
        );
    }

    #[test]
    fn unicode_fallback_does_not_escape_its_group() {
        let document = parse_rtf(br"{\rtf1\uc1 {\u65}B}")
            .expect("missing fallback bytes do not consume parent text");
        assert_eq!(write_markdown(&document), "AB");
    }

    #[test]
    fn nested_groups_restore_styles_and_escaped_literals() {
        let document = parse_rtf(br"{\rtf1\b outer {\i both} outer\b0 \{x\}\\y}")
            .expect("groups inherit and restore their parent state");
        assert_eq!(write_markdown(&document), "**outer *both* outer**{x}\\\\y");
    }

    #[test]
    fn cp1252_bytes_decode_and_unsupported_code_pages_are_typed() {
        let document =
            parse_rtf(br"{\rtf1\ansicpg1252 euro \'80}").expect("Windows-1252 is supported");
        assert_eq!(write_markdown(&document), "euro €");
        assert_eq!(
            parse_rtf(br"{\rtf1\ansicpg932 text}"),
            Err(RtfPasteError::UnsupportedCodePage(932))
        );
        let ansi_fallback = parse_rtf(br"{\rtf1\ansi\ansicpg932 \'80}")
            .expect("generic ANSI permits the documented Windows-1252 fallback");
        assert_eq!(write_markdown(&ansi_fallback), "€");
    }

    #[test]
    fn field_instruction_quotes_and_unsafe_schemes_degrade_safely() {
        let safe = parse_rtf(
            br#"{\rtf1{\field{\*\fldinst HYPERLINK "https://example.com/a?q=1"}{\fldrslt safe}}}"#,
        )
        .expect("quoted safe hyperlink instruction");
        assert_eq!(write_markdown(&safe), "[safe](https://example.com/a?q=1)");
        let unsafe_field = parse_rtf(
            br#"{\rtf1{\field{\*\fldinst HYPERLINK "javascript:alert(1)"}{\fldrslt shown}}}"#,
        )
        .expect("unsafe schemes degrade to visible result text");
        assert_eq!(write_markdown(&unsafe_field), "shown");
    }

    #[test]
    fn hyperlink_destinations_are_canonical_and_control_free() {
        assert_eq!(
            safe_hyperlink_destination(r#"HYPERLINK "https://example.com/a b>c""#),
            Some("https://example.com/a%20b%3Ec".into())
        );
        assert_eq!(
            safe_hyperlink_destination(r#"HYPERLINK "mailto:user@example.com""#),
            Some("mailto:user@example.com".into())
        );
        assert_eq!(safe_hyperlink_destination(r#"HYPERLINK "javascript:alert(1)""#), None);
        for control in ['\0', '\t', '\n', '\r'] {
            let instruction = format!("HYPERLINK \"https://example.com/a{control}b\"");
            assert_eq!(safe_hyperlink_destination(&instruction), None);
        }

        let unicode = parse_rtf(
            br#"{\rtf1{\field{\*\fldinst HYPERLINK "https://example.com/\u36335?\u24452?"}{\fldrslt unicode}}}"#,
        )
        .expect("Unicode destinations are represented canonically");
        assert_eq!(write_markdown(&unicode), "[unicode](https://example.com/%E8%B7%AF%E5%BE%84)");

        let controls = parse_rtf(
            br#"{\rtf1{\field{\*\fldinst HYPERLINK "https://example.com/\tab>injected"}{\fldrslt tab}} {\field{\*\fldinst HYPERLINK "https://example.com/\par injected"}{\fldrslt paragraph}}}"#,
        )
        .expect("unsafe field instructions degrade to their visible labels");
        assert_eq!(write_markdown(&controls), "tab paragraph");
    }

    #[test]
    fn field_group_whitespace_does_not_change_brace_validation() {
        let document = parse_rtf(
            br#"{\rtf1 { \field{\*\fldinst HYPERLINK "https://example.com"}{\fldrslt link}}}"#,
        )
        .expect("whitespace before a grouped field is harmless");
        assert_eq!(write_markdown(&document), " [link](https://example.com/)");
        assert_eq!(parse_rtf(br"\field}"), Err(RtfPasteError::UnmatchedGroupEnd));
        assert_eq!(parse_rtf(br"x\field}"), Err(RtfPasteError::UnmatchedGroupEnd));
        assert_eq!(parse_rtf(br"{\rtf1 x\field}}"), Err(RtfPasteError::UnmatchedGroupEnd));
    }

    #[test]
    fn completed_child_groups_advance_the_parent_group_position() {
        for child in ["{}", r"{\pict ignored}", "{visible}"] {
            let input = format!(
                r#"{{{child}\field{{\*\fldinst HYPERLINK "https://example.com"}}{{\fldrslt link}}}}"#,
            );
            assert_eq!(
                parse_rtf(input.as_bytes()),
                Err(RtfPasteError::UnclosedGroup),
                "child group must prevent grouped-field classification: {child}"
            );
        }
    }

    #[test]
    fn adjacent_styles_do_not_invent_visible_spaces() {
        let document =
            parse_rtf(br"{\rtf1\b A\b0\i B\i0}").expect("adjacent style controls are supported");
        assert_eq!(document.visible_segments()[0].text, "AB");
        let plain = parse_rtf(br"{\rtf1\b bold\b0 plain}")
            .expect("a control-word delimiter is not visible text");
        assert_eq!(plain.visible_segments()[0].text, "boldplain");
        let explicit_space = parse_rtf(br"{\rtf1\b bold\b0  plain}")
            .expect("the second of two spaces remains visible");
        assert_eq!(explicit_space.visible_segments()[0].text, "bold plain");
        let fallback = parse_rtf(br"{\rtf1\uc1\u65\b0 ?}")
            .expect("style delimiters do not consume Unicode fallback");
        assert_eq!(fallback.visible_segments()[0].text, "A");
    }

    #[test]
    fn incomplete_implicit_fields_return_typed_errors() {
        assert_eq!(
            parse_rtf(br#"{\rtf1 x\field{\*\fldinst HYPERLINK "https://example.com"}}"#),
            Err(RtfPasteError::UnclosedGroup)
        );
        assert_eq!(
            parse_rtf(br"{\rtf1 x\field{\fldrslt shown}}"),
            Err(RtfPasteError::UnclosedGroup)
        );
        assert_eq!(parse_rtf(br"x\field{\fldrslt shown}"), Err(RtfPasteError::UnclosedGroup));
        assert_eq!(
            parse_rtf(
                br#"x\field{\*\fldinst HYPERLINK "https://one.example"}{\fldrslt one} y\field{\*\fldinst HYPERLINK "https://two.example"}{\fldrslt two}"#,
            ),
            Err(RtfPasteError::UnclosedGroup)
        );
    }

    #[test]
    fn nested_fields_preserve_outer_result_order() {
        let document = parse_rtf(
            br#"{\rtf1{\field{\*\fldinst HYPERLINK "javascript:outer"}{\fldrslt outer {\field{\*\fldinst HYPERLINK "https://inner.example"}{\fldrslt inner}} tail}}}"#,
        )
        .expect("nested fields use independent contexts");
        assert_eq!(write_markdown(&document), "outer [inner](https://inner.example/) tail");
    }

    #[test]
    fn field_destinations_require_independent_closed_groups() {
        let same_group =
            br#"{\rtf1{\field{\*\fldinst HYPERLINK "https://example.com"\fldrslt bad}}}"#;
        assert!(parse_rtf(same_group).is_err());

        let wrong_order =
            br#"{\rtf1{\field{\fldrslt bad}{\*\fldinst HYPERLINK "https://example.com"}}}"#;
        assert!(parse_rtf(wrong_order).is_err());

        let ungrouped = br#"{\rtf1{\field\fldinst HYPERLINK "https://example.com"\fldrslt bad}}"#;
        assert!(parse_rtf(ungrouped).is_err());

        let nested_instruction = parse_rtf(
            br#"{\rtf1{\field{\*\fldinst HYPERLINK {"https://example.com/a"}}{\fldrslt link}}}"#,
        )
        .expect("nested ordinary groups stay inside the instruction group");
        assert_eq!(write_markdown(&nested_instruction), "[link](https://example.com/a)");
    }

    #[test]
    fn field_instruction_table_controls_do_not_pollute_visible_output() {
        let document = parse_rtf(
            br#"{\rtf1 before {\field{\*\fldinst HYPERLINK "https://example.com"\cell\row}{\fldrslt link}} after}"#,
        )
        .expect("field instructions are isolated from visible structure");
        assert_eq!(write_markdown(&document), "before [link](https://example.com/) after");
    }

    #[test]
    fn unknown_starred_destinations_and_empty_groups_are_ignored() {
        let document =
            parse_rtf(br"{\rtf1{}\par first{\*\unknown secret}\par\par second{\pict ignored}}")
                .expect("unknown destinations and empty paragraphs are harmless");
        assert_eq!(write_markdown(&document), "first\n\nsecond");
    }

    #[test]
    fn list_destinations_and_pictures_degrade_safely() {
        let document = parse_rtf(br"{\rtf1{\pntext\'b7\tab}item\par{\pict ignored}after}")
            .expect("simple bullet and skipped picture are supported");
        assert_eq!(write_markdown(&document), "- item\n\nafter");
    }

    #[test]
    fn nested_list_destinations_cannot_pollute_the_parent_context() {
        let instruction = parse_rtf(
            br#"{\rtf1{\field{\*\fldinst HYPERLINK "https://example.com"{\listtext 1.\tab}}{\fldrslt link}}}"#,
        )
        .expect("list markers in field instructions are isolated");
        assert_eq!(write_markdown(&instruction), "[link](https://example.com/)");

        let result = parse_rtf(
            br#"{\rtf1{\field{\*\fldinst HYPERLINK "javascript:unsafe"}{\fldrslt before {\pntext 1.\tab}label}}}"#,
        )
        .expect("list markers in field results do not change the outer paragraph");
        assert_eq!(write_markdown(&result), "before label");

        let skipped = parse_rtf(
            br#"{\rtf1{\pict{\listtext 1.\tab}ignored}{\*\unknown{\pntext 2.\tab}}plain}"#,
        )
        .expect("skipped destinations cannot emit list markers");
        assert_eq!(write_markdown(&skipped), "plain");
    }

    #[test]
    fn field_container_sibling_list_markers_are_suppressed() {
        for destination in ["listtext", "pntext"] {
            let input = format!(
                r#"{{\rtf1 before {{\field{{\{destination} 1.\tab}}{{\*\fldinst HYPERLINK "https://example.com"}}{{\fldrslt link}}}}\par after}}"#,
            );
            let document = parse_rtf(input.as_bytes())
                .expect("field-container sibling markers are accepted but isolated");
            assert_eq!(write_markdown(&document), "before [link](https://example.com/)\n\nafter");
        }

        let nested = parse_rtf(
            br#"{\rtf1{\field{\*\fldinst HYPERLINK "javascript:outer"}{\fldrslt outer {\field{\pntext 1.\tab}{\*\fldinst HYPERLINK "javascript:inner"}{\fldrslt inner}} tail}}}"#,
        )
        .expect("nested field-container sibling markers remain field-local");
        assert_eq!(write_markdown(&nested), "outer inner tail");
    }

    #[test]
    fn document_list_marker_before_a_field_remains_effective() {
        let document = parse_rtf(
            br#"{\rtf1{\listtext 1.\tab}before {\field{\*\fldinst HYPERLINK "https://example.com"}{\fldrslt link}}\par after}"#,
        )
        .expect("a document list marker remains effective when followed by a field");
        assert_eq!(write_markdown(&document), "1. before [link](https://example.com/)\n\nafter");
    }

    #[test]
    fn consecutive_ordered_list_markers_form_one_list() {
        let document = parse_rtf(br"{\rtf1{\listtext 3.\tab}third\par{\listtext 4.\tab}fourth}")
            .expect("consecutive ordered markers are supported");
        assert_eq!(write_markdown(&document), "3. third\n4. fourth");
    }

    #[test]
    fn line_tabs_and_multiple_rows_keep_visible_order() {
        let document = parse_rtf(br"{\rtf1 a\line b\par c\tab d\cell e\row}")
            .expect("visible structural controls are supported");
        assert_eq!(write_markdown(&document), "a  \nb\n\nc\td\te");
    }
}
