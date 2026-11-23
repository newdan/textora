//! 中栏卡片及工具栏的展示输入。

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use notora_core::{DocumentIdentity, DocumentKind, NavigationScope, NoteId};
use ui::status_state::{StatusStateInput, StatusStateKind};

use crate::action::{NotoraAction, TrashOperation};
use crate::external_files::ExternalFileSession;
use crate::state::CardPageState;
use crate::{NotoraState, WorkspaceRootState};

use super::SET_WORKSPACE_ROOT_BUTTON_ID;

/// UI 之前的产品展示卡片；保持领域身份，避免将 app 状态泄漏给 ui crate。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderCard {
    pub identity: DocumentIdentity,
    pub title: String,
    pub excerpt: String,
    pub timestamp: String,
    pub icon: Option<String>,
    pub tag_summary: String,
    pub closable: bool,
}

/// 产品层计算好的工具栏动作；widget 不保存 NoteId、TagId 或导航范围。
#[derive(Clone, Debug, PartialEq)]
pub struct NoteToolbarButtonInput {
    pub icon: Option<&'static str>,
    pub label: String,
    pub action: NotoraAction,
}

/// 中栏标题栏的新建入口状态；隐藏态不保留可点击矩形。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NewNoteControlState {
    #[default]
    Hidden,
    Disabled,
    Enabled,
    EnabledForFiles,
}

impl NewNoteControlState {
    pub(super) fn is_visible(self) -> bool {
        self != Self::Hidden
    }

    pub(super) fn is_enabled(self) -> bool {
        matches!(self, Self::Enabled | Self::EnabledForFiles)
    }
}

pub(super) fn note_toolbar_buttons(
    scope: &NavigationScope,
    selected_note_id: Option<NoteId>,
    external_files_present: bool,
) -> Vec<NoteToolbarButtonInput> {
    if *scope == NavigationScope::ExternalFiles {
        let mut buttons = vec![NoteToolbarButtonInput {
            icon: Some("folder-open"),
            label: "打开".to_owned(),
            action: NotoraAction::OpenExternalFileDialogRequested,
        }];
        if external_files_present {
            buttons.push(NoteToolbarButtonInput {
                icon: Some("x"),
                label: "清空".to_owned(),
                action: NotoraAction::ExternalFilesClearRequested,
            });
        }
        return buttons;
    }
    if matches!(scope, NavigationScope::Search { .. }) {
        return vec![NoteToolbarButtonInput {
            icon: None,
            label: "清除".to_owned(),
            action: NotoraAction::SearchCommitted { query: String::new(), search_generation: None },
        }];
    }
    if *scope == NavigationScope::Trash {
        let Some(note_id) = selected_note_id else {
            return vec![NoteToolbarButtonInput {
                icon: Some("trash-2"),
                label: "清空".to_owned(),
                action: NotoraAction::TrashOperationRequested(TrashOperation::Empty),
            }];
        };
        return vec![
            NoteToolbarButtonInput {
                icon: Some("undo-2"),
                label: "恢复".to_owned(),
                action: NotoraAction::TrashOperationRequested(TrashOperation::Restore { note_id }),
            },
            NoteToolbarButtonInput {
                icon: Some("trash-2"),
                label: "删除".to_owned(),
                action: NotoraAction::TrashOperationRequested(TrashOperation::PermanentlyDelete {
                    note_id,
                }),
            },
        ];
    }
    let _ = selected_note_id;
    Vec::new()
}

pub(super) fn new_note_control_state(
    scope: &NavigationScope,
    workspace_root: WorkspaceRootState,
) -> NewNoteControlState {
    match scope {
        NavigationScope::Search { .. }
        | NavigationScope::Starred
        | NavigationScope::Trash
        | NavigationScope::Tag { .. } => NewNoteControlState::Hidden,
        NavigationScope::ExternalFiles => NewNoteControlState::EnabledForFiles,
        NavigationScope::WorkspaceRoot | NavigationScope::Directory { .. } => {
            match workspace_root {
                WorkspaceRootState::Missing => NewNoteControlState::Disabled,
                WorkspaceRootState::Active => NewNoteControlState::Enabled,
            }
        }
    }
}

pub(super) fn card_empty_state_input(state: &NotoraState) -> StatusStateInput {
    if state.library.navigation_scope == NavigationScope::ExternalFiles {
        return StatusStateInput {
            kind: StatusStateKind::Empty,
            title: "暂无文件".to_owned(),
            description: "新建一个文件，或打开已有文件。".to_owned(),
            icon: Some("file".to_owned()),
            action_label: None,
            action_id: None,
        };
    }
    if state.workspace_root == WorkspaceRootState::Missing {
        return StatusStateInput {
            kind: StatusStateKind::Empty,
            title: "尚未设置工作区根目录".to_owned(),
            description: "请先选择一个文件夹作为工作区根目录。".to_owned(),
            icon: Some("folder-open".to_owned()),
            action_label: Some("设置根目录".to_owned()),
            action_id: Some(SET_WORKSPACE_ROOT_BUTTON_ID),
        };
    }
    StatusStateInput {
        kind: StatusStateKind::Empty,
        title: "暂无笔记".to_owned(),
        description: "新建一篇笔记，或者从左侧选择其他位置。".to_owned(),
        icon: Some("notebook-pen".to_owned()),
        action_label: None,
        action_id: None,
    }
}

pub(super) fn render_cards(state: &NotoraState) -> Vec<RenderCard> {
    if state.library.navigation_scope == NavigationScope::ExternalFiles {
        return state
            .external_files
            .sessions()
            .iter()
            .rev()
            .map(render_external_file_card)
            .collect();
    }
    let mut cards = match &state.library.card_page {
        CardPageState::Ready { cards, .. }
        | CardPageState::LoadingNextPage { cards, .. }
        | CardPageState::Refreshing { cards, .. }
        | CardPageState::Failed { cards, .. } => cards.iter().map(render_catalog_card).collect(),
        CardPageState::Idle
        | CardPageState::LoadingInitial { .. }
        | CardPageState::Empty { .. } => Vec::new(),
    };
    let Some(selected_identity) = state.library.selected_card else {
        return cards;
    };
    let selected_title = state.library.title_draft.as_ref().or_else(|| {
        state
            .library
            .pending_title_commit
            .as_ref()
            .filter(|pending_title| pending_title.identity == selected_identity)
            .map(|pending_title| &pending_title.title)
    });
    let Some(selected_title) = selected_title else {
        return cards;
    };
    if let Some(selected_card) = cards.iter_mut().find(|card| card.identity == selected_identity) {
        selected_card.title.clone_from(selected_title);
    }
    cards
}

fn render_catalog_card(card: &notora_core::CatalogCard) -> RenderCard {
    let mut summary_parts = Vec::with_capacity(card.tags.len() + usize::from(card.starred));
    if card.starred {
        summary_parts.push("★".to_owned());
    }
    summary_parts.extend(card.tags.iter().map(|tag| format!("#{tag}")));
    RenderCard {
        identity: DocumentIdentity::Note(card.note_id),
        title: card.title.clone(),
        excerpt: card.excerpt.clone(),
        timestamp: format_modified_timestamp(card.modified_nanoseconds),
        icon: Some(if card.encryption == notora_core::NoteEncryption::Encrypted {
            "lock".to_owned()
        } else {
            document_icon(card.kind).to_owned()
        }),
        tag_summary: summary_parts.join(" "),
        closable: false,
    }
}

fn render_external_file_card(session: &ExternalFileSession) -> RenderCard {
    match session {
        ExternalFileSession::Existing { canonical_path, .. } => RenderCard {
            identity: session.identity(),
            title: canonical_path
                .as_path()
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("未命名")
                .to_owned(),
            excerpt: canonical_path.as_path().display().to_string(),
            timestamp: "外部文件".to_owned(),
            icon: Some(
                document_icon(
                    DocumentKind::from_external_path(canonical_path.as_path())
                        .unwrap_or(DocumentKind::Text),
                )
                .to_owned(),
            ),
            tag_summary: String::new(),
            closable: true,
        },
        ExternalFileSession::Untitled { kind, .. } => RenderCard {
            identity: session.identity(),
            title: "未命名".to_owned(),
            excerpt: "尚未保存的外部文件".to_owned(),
            timestamp: "外部文件".to_owned(),
            icon: Some(document_icon(*kind).to_owned()),
            tag_summary: String::new(),
            closable: true,
        },
        ExternalFileSession::Missing { last_known_path, .. } => RenderCard {
            identity: session.identity(),
            title: last_known_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("文件已丢失")
                .to_owned(),
            excerpt: last_known_path.display().to_string(),
            timestamp: "已丢失".to_owned(),
            icon: Some("file".to_owned()),
            tag_summary: String::new(),
            closable: true,
        },
    }
}

pub(super) fn document_icon(kind: DocumentKind) -> &'static str {
    match kind {
        DocumentKind::Text => "notebook-text",
        DocumentKind::Markdown => "file-code-corner",
        DocumentKind::Mindmap => "workflow",
    }
}

fn format_modified_timestamp(modified_nanoseconds: i64) -> String {
    format_modified_timestamp_at(modified_nanoseconds, SystemTime::now())
}

pub(super) fn format_modified_timestamp_at(modified_nanoseconds: i64, now: SystemTime) -> String {
    let Ok(modified_nanoseconds) = u64::try_from(modified_nanoseconds) else {
        return "修改时间未知".to_owned();
    };
    let Some(modified_at) = UNIX_EPOCH.checked_add(Duration::from_nanos(modified_nanoseconds))
    else {
        return "修改时间未知".to_owned();
    };
    format_modified_time(modified_at, now)
}

pub(super) fn card_list_title(scope: &NavigationScope) -> &'static str {
    match scope {
        NavigationScope::Search { .. } => "搜索结果",
        NavigationScope::WorkspaceRoot => "工作区",
        NavigationScope::Directory { .. } => "文件夹",
        NavigationScope::Starred => "星标",
        NavigationScope::Trash => "回收站",
        NavigationScope::Tag { .. } => "标签",
        NavigationScope::ExternalFiles => "文件",
    }
}

pub(super) fn format_created_time(timestamp: SystemTime) -> String {
    let Some((year, month, day)) = utc_calendar_date(timestamp) else {
        return "创建时间未知".to_owned();
    };
    format!("创建 {year:04}/{month:02}/{day:02} UTC")
}

pub(super) fn format_modified_time(timestamp: SystemTime, now: SystemTime) -> String {
    let elapsed_seconds =
        now.duration_since(timestamp).map(|elapsed| elapsed.as_secs()).unwrap_or(0);
    if elapsed_seconds < 60 {
        return "修改 刚刚".to_owned();
    }
    if elapsed_seconds < 3_600 {
        return format!("修改 {} 分钟前", elapsed_seconds / 60);
    }
    if elapsed_seconds < 86_400 {
        return format!("修改 {} 小时前", elapsed_seconds / 3_600);
    }
    format!("修改 {} 天前", elapsed_seconds / 86_400)
}

fn utc_calendar_date(timestamp: SystemTime) -> Option<(i64, i64, i64)> {
    let elapsed_seconds = timestamp.duration_since(UNIX_EPOCH).ok()?.as_secs();
    let days_since_epoch = i64::try_from(elapsed_seconds / 86_400).ok()?;
    let shifted_days = days_since_epoch.checked_add(719_468)?;
    let era = shifted_days.div_euclid(146_097);
    let day_of_era = shifted_days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_parameter = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_parameter + 2) / 5 + 1;
    let month = month_parameter + if month_parameter < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    Some((year, month, day))
}
