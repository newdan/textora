use std::time::{Duration, Instant, SystemTime};

use super::{
    ActiveEditorMetadata, CardPageState, CompactContent, CompactNavigation,
    EncryptedConflictCopyState, EncryptedNoteCreationState, EncryptedNoteUnlockState, FocusTarget,
    LibraryState, NavigationPaneVisibility, NotoraState, OverlayState, WorkspaceRootState,
};
use crate::action::{CardQuery, DocumentLoadRequest, NotoraAction, NotoraEffect};
use crate::search_controller::{SEARCH_DEBOUNCE_DELAY, SearchController};
use notora_core::{
    CatalogCard, CatalogCardCursor, CatalogCardPage, DocumentIdentity, DocumentKind,
    NavigationScope, NoteEditorMetadata, NoteId, TagId,
};

fn card(note_id: NoteId, title: &str, modified_nanoseconds: i64) -> CatalogCard {
    CatalogCard {
        note_id,
        relative_path: format!("notes/{title}.md").into(),
        kind: DocumentKind::Markdown,
        encryption: notora_core::NoteEncryption::Unencrypted,
        title: title.to_owned(),
        excerpt: format!("{title} excerpt"),
        modified_nanoseconds,
        starred: false,
        tags: vec!["计划".to_owned()],
    }
}

fn state_with_active_workspace() -> NotoraState {
    NotoraState { workspace_root: WorkspaceRootState::Active, ..NotoraState::default() }
}

#[test]
fn starts_in_workspace_root_scope() {
    assert_eq!(LibraryState::default().navigation_scope, NavigationScope::WorkspaceRoot);
}

#[test]
fn settings_view_actions_request_only_their_owned_effects() {
    let mut state = NotoraState::default();

    assert_eq!(state.reduce(NotoraAction::SettingsViewChanged), vec![NotoraEffect::Redraw]);
    assert_eq!(
        state.reduce(NotoraAction::RetryProductSettingsPersistence),
        vec![NotoraEffect::PersistProductSettings, NotoraEffect::Redraw]
    );
}

#[test]
fn title_commit_is_only_effectful_for_a_selected_workspace_note() {
    let note_id = NoteId::generate();
    let mut state = NotoraState::default();
    state.library.selected_card = Some(DocumentIdentity::Note(note_id));

    assert_eq!(
        state.reduce(NotoraAction::TitleCommitRequested("项目路线图".to_owned())),
        vec![NotoraEffect::CommitTitle("项目路线图".to_owned()), NotoraEffect::Redraw]
    );
    assert_eq!(state.layout.focus_target, FocusTarget::Editor);
    assert_eq!(
        state
            .library
            .pending_title_commit
            .as_ref()
            .map(|pending_title| (pending_title.identity, pending_title.title.as_str())),
        Some((DocumentIdentity::Note(note_id), "项目路线图"))
    );

    state.library.navigation_scope = NavigationScope::Trash;
    assert_eq!(
        state.reduce(NotoraAction::TitleCommitRequested("不能提交".to_owned())),
        vec![NotoraEffect::Redraw]
    );

    state.library.navigation_scope = NavigationScope::ExternalFiles;
    state.library.selected_card =
        Some(DocumentIdentity::ExternalFile(notora_core::ExternalFileId::generate()));
    assert_eq!(
        state.reduce(NotoraAction::TitleCommitRequested("外部文件".to_owned())),
        vec![NotoraEffect::Redraw]
    );
}

#[test]
fn external_file_record_is_removed_only_after_runtime_close_completes() {
    let mut state = NotoraState::default();
    state.library.navigation_scope = NavigationScope::ExternalFiles;
    let identity = state.external_files.create_untitled(DocumentKind::Markdown);
    let DocumentIdentity::ExternalFile(external_file_id) = identity else {
        panic!("an external session must have an external identity");
    };
    state.library.selected_card = Some(identity);

    assert_eq!(
        state.reduce(NotoraAction::ExternalFileCloseRequested(external_file_id)),
        vec![NotoraEffect::CloseExternalFile(external_file_id), NotoraEffect::Redraw]
    );
    assert!(state.external_files.session(external_file_id).is_some());

    assert_eq!(
        state.reduce(NotoraAction::ExternalFileCloseCompleted(external_file_id)),
        vec![NotoraEffect::Redraw]
    );
    assert!(state.external_files.session(external_file_id).is_none());
    assert_eq!(state.library.selected_card, None);
}

#[test]
fn failed_external_file_close_keeps_the_record_and_reports_the_reason() {
    let mut state = NotoraState::default();
    let identity = state.external_files.create_untitled(DocumentKind::Text);
    let DocumentIdentity::ExternalFile(external_file_id) = identity else {
        panic!("an external session must have an external identity");
    };

    let effects =
        state.reduce(NotoraAction::ExternalFileCloseFailed("文件仍有未保存修改".to_owned()));

    assert_eq!(effects, vec![NotoraEffect::Redraw]);
    assert!(state.external_files.session(external_file_id).is_some());
    assert_eq!(state.library.last_command_error.as_deref(), Some("文件仍有未保存修改"));
}

#[test]
fn clearing_external_files_removes_completed_records_and_keeps_blocked_records() {
    let mut state = NotoraState::default();
    state.library.navigation_scope = NavigationScope::ExternalFiles;
    let closed_identity = state.external_files.create_untitled(DocumentKind::Markdown);
    let blocked_identity = state.external_files.create_untitled(DocumentKind::Text);
    let DocumentIdentity::ExternalFile(closed_external_file_id) = closed_identity else {
        panic!("external fixture must have an external identity");
    };
    let DocumentIdentity::ExternalFile(blocked_external_file_id) = blocked_identity else {
        panic!("external fixture must have an external identity");
    };
    state.library.selected_card = Some(closed_identity);

    assert_eq!(
        state.reduce(NotoraAction::ExternalFilesClearRequested),
        vec![NotoraEffect::CloseAllExternalFiles, NotoraEffect::Redraw]
    );
    assert_eq!(
        state.reduce(NotoraAction::ExternalFilesClearCompleted {
            closed_external_file_ids: vec![closed_external_file_id],
            blocked_count: 1,
        }),
        vec![NotoraEffect::Redraw]
    );

    assert!(state.external_files.session(closed_external_file_id).is_none());
    assert!(state.external_files.session(blocked_external_file_id).is_some());
    assert_eq!(state.library.selected_card, None);
    assert_eq!(
        state.library.last_command_error.as_deref(),
        Some("有 1 个文件未清空：请先保存未保存内容或取消固定")
    );
}

#[test]
fn refreshed_catalog_title_confirms_the_pending_title_commit() {
    let note_id = NoteId::generate();
    let query = CardQuery::from(NavigationScope::WorkspaceRoot);
    let mut state = NotoraState::default();
    state.library.selected_card = Some(DocumentIdentity::Note(note_id));

    let _ = state.reduce(NotoraAction::TitleCommitRequested("  项目路线图  ".to_owned()));
    state.library.card_page = CardPageState::LoadingInitial { query: query.clone() };
    let _ = state.reduce(NotoraAction::CardQueryCompleted {
        query,
        page: CatalogCardPage {
            cards: vec![card(note_id, "项目路线图", 1)], next_cursor: None
        },
    });

    assert_eq!(state.library.pending_title_commit, None);
}

#[test]
fn title_text_changes_remain_a_local_draft_until_commit() {
    let note_id = NoteId::generate();
    let mut state = NotoraState::default();
    state.library.selected_card = Some(DocumentIdentity::Note(note_id));
    state.layout.focus_target = FocusTarget::EditorTitle;

    assert_eq!(
        state.reduce(NotoraAction::TitleTextChanged("项目路线图".to_owned())),
        vec![NotoraEffect::Redraw]
    );
    assert_eq!(state.layout.focus_target, FocusTarget::EditorTitle);
}

#[test]
fn source_view_toggle_is_a_typed_editor_effect() {
    let mut state = NotoraState::default();

    assert_eq!(
        state.reduce(NotoraAction::ToggleSourceViewRequested),
        vec![NotoraEffect::ToggleEditorView]
    );
    assert_eq!(state.layout.focus_target, FocusTarget::Editor);
}

#[test]
fn empty_search_restores_the_scope_before_search() {
    let mut state = NotoraState::default();
    let _ = state.reduce(NotoraAction::NavigationSelected(NavigationScope::Starred));
    let _ = state.reduce(NotoraAction::SearchCommitted {
        query: "roadmap".to_owned(),
        search_generation: None,
    });

    assert_eq!(
        state.reduce(NotoraAction::SearchCommitted {
            query: String::new(),
            search_generation: None,
        }),
        vec![
            NotoraEffect::QueryCards(CardQuery::from(NavigationScope::Starred)),
            NotoraEffect::Redraw,
        ]
    );
    assert_eq!(state.library.navigation_scope, NavigationScope::Starred);
}

#[test]
fn search_text_updates_immediately_without_querying_until_the_debounce_commits() {
    let mut state = NotoraState::default();

    assert_eq!(
        state.reduce(NotoraAction::SearchTextChanged("roadmap".to_owned())),
        vec![NotoraEffect::Redraw]
    );
    assert_eq!(state.library.search_text, "roadmap");
    assert_eq!(state.library.navigation_scope, NavigationScope::WorkspaceRoot);
}

#[test]
fn non_empty_search_keeps_keyboard_focus_in_the_search_box() {
    let mut state = NotoraState::default();
    let _ = state.reduce(NotoraAction::FocusRequested(FocusTarget::NavigationSearch));

    let _ = state.reduce(NotoraAction::SearchCommitted {
        query: "roadmap".to_owned(),
        search_generation: None,
    });

    assert_eq!(state.layout.focus_target, FocusTarget::NavigationSearch);
}

#[test]
fn trash_scope_cannot_request_note_creation() {
    let mut state = state_with_active_workspace();
    let _ = state.reduce(NotoraAction::NavigationSelected(NavigationScope::Trash));

    assert_eq!(
        state.reduce(NotoraAction::CreateRequested(DocumentKind::Markdown)),
        vec![NotoraEffect::Redraw]
    );
}

#[test]
fn external_files_scope_does_not_query_the_workspace_catalog() {
    let mut state = NotoraState::default();

    assert_eq!(
        state.reduce(NotoraAction::NavigationSelected(NavigationScope::ExternalFiles)),
        vec![NotoraEffect::Redraw]
    );
}

#[test]
fn opening_an_external_file_scrolls_the_file_list_to_the_top() {
    let mut state = NotoraState::default();
    state.library.navigation_scope = NavigationScope::ExternalFiles;
    state.library.card_scroll_offset_px = 240.0;
    let identity = state.external_files.create_untitled(DocumentKind::Markdown);

    let effects = state.reduce(NotoraAction::ExternalFileOpened(identity));

    assert_eq!(state.library.card_scroll_offset_px, 0.0);
    assert_eq!(state.library.selected_card, Some(identity));
    assert!(effects.contains(&NotoraEffect::Redraw));
}

#[test]
fn changing_navigation_scope_invalidates_the_previous_card_selection() {
    let mut state = NotoraState::default();
    let note_id = NoteId::generate();
    let request = state.select_document(DocumentIdentity::Note(note_id));
    state.library.active_editor_metadata = Some(ActiveEditorMetadata {
        identity: request.identity,
        selection_generation: request.selection_generation,
        metadata: NoteEditorMetadata {
            note_id,
            created_at: SystemTime::UNIX_EPOCH,
            modified_at: SystemTime::UNIX_EPOCH,
            encryption: notora_core::NoteEncryption::Unencrypted,
            title_initialization: notora_core::TitleInitialization::Independent,
            file_name_binding: notora_core::NoteFileNameBinding::TitleBound { disambiguator: 1 },
            title_revision: 0,
        },
        tags: Vec::new(),
    });

    let _ = state.reduce(NotoraAction::NavigationSelected(NavigationScope::Trash));

    assert_eq!(state.library.selected_card, None);
    assert_eq!(state.library.active_editor_metadata, None);
    assert_eq!(state.library.selected_document_generation, 2);
}

#[test]
fn reselecting_the_current_navigation_scope_preserves_the_active_selection() {
    let mut state = NotoraState::default();
    let identity = DocumentIdentity::Note(NoteId::generate());
    let _ = state.select_document(identity);

    let _ = state.reduce(NotoraAction::NavigationSelected(NavigationScope::WorkspaceRoot));

    assert_eq!(state.library.selected_card, Some(identity));
    assert_eq!(state.library.selected_document_generation, 1);
}

#[test]
fn tag_scope_requires_content_hashtags_instead_of_implicit_attachment() {
    let mut state = state_with_active_workspace();
    let tag_id = TagId::generate();
    let _ = state.reduce(NotoraAction::NavigationSelected(NavigationScope::Tag { tag_id }));

    assert_eq!(
        state.reduce(NotoraAction::CreateRequested(DocumentKind::Markdown)),
        vec![
            NotoraEffect::RequestNoteCreation {
                kind: DocumentKind::Markdown,
                target: crate::action::NoteCreationTarget { directory: None },
            },
            NotoraEffect::Redraw,
        ]
    );
}

#[test]
fn note_requests_reduce_to_a_typed_domain_command_effect() {
    let mut state = state_with_active_workspace();

    assert!(matches!(
        state.reduce(NotoraAction::CreateRequested(DocumentKind::Markdown)).as_slice(),
        [
            NotoraEffect::RequestNoteCreation {
                kind: DocumentKind::Markdown,
                target: crate::action::NoteCreationTarget { directory: None },
            },
            NotoraEffect::Redraw
        ]
    ));
}

#[test]
fn created_note_is_selected_and_enters_editor_state() {
    let mut state = NotoraState::default();
    let note_id = NoteId::generate();
    let result = notora_core::note_command::NoteCommandResult {
        note: notora_core::CatalogNote {
            note_id,
            relative_path: "无标题.md".into(),
            kind: DocumentKind::Markdown,
            title: "无标题".to_owned(),
            excerpt: String::new(),
            modified_at: SystemTime::UNIX_EPOCH,
            file_size: 0,
            content_hash: Vec::new(),
            starred: false,
        },
        previous_relative_path: None,
        outcome: notora_core::note_command::NoteCommandOutcome::Created,
        created_access: Some(notora_core::CreatedNoteAccess::Unencrypted),
        created_editor_metadata: None,
    };

    let effects = state.reduce(NotoraAction::NoteCommandCompleted(result));

    assert_eq!(state.library.selected_card, Some(DocumentIdentity::Note(note_id)));
    assert_eq!(state.layout.focus_target, FocusTarget::EditorTitle);
    assert_eq!(state.layout.compact_content, CompactContent::Editor);
    assert!(matches!(effects.get(1), Some(NotoraEffect::PrepareDocument(_))));
}

#[test]
fn move_dialog_entry_stays_behind_a_typed_effect() {
    let mut state = NotoraState::default();
    let note_id = NoteId::generate();

    assert_eq!(
        state.reduce(NotoraAction::MoveDialogRequested(note_id)),
        vec![NotoraEffect::ChooseNoteMoveDirectory(note_id), NotoraEffect::Redraw]
    );
}

#[test]
fn command_failure_preserves_the_existing_selection_and_exposes_a_recoverable_message() {
    let mut state = NotoraState::default();
    let selected_note = notora_core::DocumentIdentity::Note(notora_core::NoteId::generate());
    let _ = state.reduce(NotoraAction::CardSelected(selected_note));

    assert_eq!(
        state.reduce(NotoraAction::NoteCommandFailed("destination exists".to_owned())),
        vec![NotoraEffect::Redraw]
    );
    assert_eq!(state.library.selected_card, Some(selected_note));
    assert_eq!(state.library.last_command_error.as_deref(), Some("destination exists"));
}

#[test]
fn preview_promotion_is_an_explicit_runtime_effect() {
    let mut state = NotoraState::default();

    assert_eq!(
        state.reduce(NotoraAction::PromotePreviewRequested),
        vec![NotoraEffect::PromoteActivePreview, NotoraEffect::Redraw]
    );
}

#[test]
fn opening_new_document_menu_only_opens_the_menu() {
    let mut state = state_with_active_workspace();

    assert_eq!(state.reduce(NotoraAction::OpenNewDocumentMenu), vec![NotoraEffect::Redraw]);
    assert_eq!(state.layout.overlay, OverlayState::NewDocumentMenu);
    assert_eq!(state.layout.focus_target, FocusTarget::Overlay);
}

#[test]
fn encrypted_note_creation_validates_and_clears_sensitive_draft() {
    use ui::core::widget::SensitiveText;

    let mut state = state_with_active_workspace();
    let _ = state.reduce(NotoraAction::OpenNewDocumentMenu);
    let _ = state.reduce(NotoraAction::BeginEncryptedNoteCreation);

    assert_eq!(state.layout.overlay, OverlayState::EncryptedNoteDialog);
    let _ = state.reduce(NotoraAction::EncryptedNotePasswordChanged(SensitiveText::new(
        "valid-password".to_owned(),
    )));
    let _ = state.reduce(NotoraAction::EncryptedNoteConfirmationChanged(SensitiveText::new(
        "different-password".to_owned(),
    )));
    assert_eq!(
        state.reduce(NotoraAction::EncryptedNoteDialogSubmitRequested),
        vec![NotoraEffect::Redraw]
    );
    assert!(matches!(
        &state.encrypted_note_creation,
        EncryptedNoteCreationState::Editing { error_message: Some(message), .. }
            if message == "两次输入的密码不一致"
    ));

    let _ = state.reduce(NotoraAction::OverlayDismissed);

    assert_eq!(state.encrypted_note_creation, EncryptedNoteCreationState::Inactive);
    assert_eq!(state.layout.overlay, OverlayState::None);
}

#[test]
fn encrypted_note_creation_rejects_five_character_password() {
    use ui::core::widget::SensitiveText;

    let mut state = state_with_active_workspace();
    let _ = state.reduce(NotoraAction::BeginEncryptedNoteCreation);
    let _ = state
        .reduce(NotoraAction::EncryptedNotePasswordChanged(SensitiveText::new("abc12".to_owned())));
    let _ = state.reduce(NotoraAction::EncryptedNoteConfirmationChanged(SensitiveText::new(
        "abc12".to_owned(),
    )));

    assert_eq!(
        state.reduce(NotoraAction::EncryptedNoteDialogSubmitRequested),
        vec![NotoraEffect::Redraw]
    );
    assert!(matches!(
        state.encrypted_note_creation,
        EncryptedNoteCreationState::Editing { error_message: Some(message), .. }
            if message == "密码至少需要 6 个字符"
    ));
}

#[test]
fn encrypted_note_creation_prevents_duplicate_submission() {
    use ui::core::widget::SensitiveText;

    let mut state = state_with_active_workspace();
    let _ = state.reduce(NotoraAction::BeginEncryptedNoteCreation);
    let _ = state.reduce(NotoraAction::EncryptedNotePasswordChanged(SensitiveText::new(
        "abc123".to_owned(),
    )));
    let _ = state.reduce(NotoraAction::EncryptedNoteConfirmationChanged(SensitiveText::new(
        "abc123".to_owned(),
    )));

    let effects = state.reduce(NotoraAction::EncryptedNoteDialogSubmitRequested);
    assert!(matches!(
        effects.as_slice(),
        [NotoraEffect::ExecuteNoteCommand(
            notora_core::note_command::NoteCommand::CreateConfigured(request)
        ), NotoraEffect::Redraw]
            if request.kind == DocumentKind::Markdown
                && matches!(request.storage, notora_core::CreateNoteStorage::Encrypted { .. })
    ));
    assert!(matches!(
        state.encrypted_note_creation,
        EncryptedNoteCreationState::Submitting { generation: 1, .. }
    ));
    assert_eq!(
        state.reduce(NotoraAction::EncryptedNoteDialogSubmitRequested),
        vec![NotoraEffect::Redraw]
    );
}

#[test]
fn encrypted_note_creation_effect_debug_redacts_password() {
    use ui::core::widget::SensitiveText;

    let mut state = state_with_active_workspace();
    let _ = state.reduce(NotoraAction::BeginEncryptedNoteCreation);
    let _ = state.reduce(NotoraAction::EncryptedNotePasswordChanged(SensitiveText::new(
        "debug-secret-password".to_owned(),
    )));
    let _ = state.reduce(NotoraAction::EncryptedNoteConfirmationChanged(SensitiveText::new(
        "debug-secret-password".to_owned(),
    )));

    let effects = state.reduce(NotoraAction::EncryptedNoteDialogSubmitRequested);
    let rendered = format!("{effects:?}");

    assert!(!rendered.contains("debug-secret-password"));
    assert!(rendered.contains("<redacted>"));
}

#[test]
fn encrypted_conflict_copy_requires_confirmation_and_redacts_its_effect() {
    use ui::core::widget::SensitiveText;

    let identity = DocumentIdentity::Note(NoteId::generate());
    let target_path = std::path::PathBuf::from("冲突副本.md");
    let mut state = state_with_active_workspace();
    let _ = state.reduce(NotoraAction::SaveConflictDetected { identity, content_revision: 9 });
    let _ = state.reduce(NotoraAction::EncryptedConflictCopyRequired {
        identity,
        target_path: target_path.clone(),
    });
    let _ = state.reduce(NotoraAction::EncryptedNotePasswordChanged(SensitiveText::new(
        "abc123".to_owned(),
    )));
    let _ = state.reduce(NotoraAction::EncryptedNoteConfirmationChanged(SensitiveText::new(
        "abc123".to_owned(),
    )));

    let effects = state.reduce(NotoraAction::EncryptedNoteDialogSubmitRequested);

    assert!(matches!(
        effects.as_slice(),
        [NotoraEffect::SaveEncryptedConflictCopy(request), NotoraEffect::Redraw]
            if request.identity == identity
                && request.target_path == target_path
                && request.generation == 1
    ));
    assert!(!format!("{effects:?}").contains("abc123"));
    assert!(matches!(
        state.encrypted_conflict_copy,
        EncryptedConflictCopyState::Submitting { generation: 1, .. }
    ));
}

#[test]
fn encrypted_conflict_copy_rejects_five_character_password() {
    use ui::core::widget::SensitiveText;

    let identity = DocumentIdentity::Note(NoteId::generate());
    let mut state = state_with_active_workspace();
    let _ = state.reduce(NotoraAction::SaveConflictDetected { identity, content_revision: 9 });
    let _ = state.reduce(NotoraAction::EncryptedConflictCopyRequired {
        identity,
        target_path: "冲突副本.md".into(),
    });
    let _ = state
        .reduce(NotoraAction::EncryptedNotePasswordChanged(SensitiveText::new("abc12".to_owned())));
    let _ = state.reduce(NotoraAction::EncryptedNoteConfirmationChanged(SensitiveText::new(
        "abc12".to_owned(),
    )));

    assert_eq!(
        state.reduce(NotoraAction::EncryptedNoteDialogSubmitRequested),
        vec![NotoraEffect::Redraw]
    );
    assert!(matches!(
        state.encrypted_conflict_copy,
        EncryptedConflictCopyState::Editing { error_message: Some(message), .. }
            if message == "密码至少需要 6 个字符"
    ));
}

#[test]
fn encrypted_note_unlock_is_inline_and_closes_on_completion_or_document_selection() {
    use ui::core::widget::SensitiveText;

    let note_id = NoteId::generate();
    let request =
        DocumentLoadRequest { identity: DocumentIdentity::Note(note_id), selection_generation: 4 };
    let mut state = state_with_active_workspace();
    state.library.selected_card = Some(request.identity);
    state.library.selected_document_generation = request.selection_generation;

    let _ = state.reduce(NotoraAction::EncryptedNoteUnlockRequired {
        request,
        title: "秘密笔记".to_owned(),
        metadata: NoteEditorMetadata {
            note_id,
            created_at: SystemTime::UNIX_EPOCH,
            modified_at: SystemTime::UNIX_EPOCH,
            encryption: notora_core::NoteEncryption::Encrypted,
            title_initialization: notora_core::TitleInitialization::Independent,
            file_name_binding: notora_core::NoteFileNameBinding::TitleBound { disambiguator: 1 },
            title_revision: 0,
        },
        tags: Vec::new(),
    });
    assert_eq!(state.layout.overlay, OverlayState::None);
    assert_eq!(state.layout.focus_target, FocusTarget::Editor);
    let _ = state.reduce(NotoraAction::EncryptedNotePasswordChanged(SensitiveText::new(
        "abc123".to_owned(),
    )));
    let mut switched_state = state.clone();
    let other_identity = DocumentIdentity::Note(NoteId::generate());
    let _ = switched_state.reduce(NotoraAction::CardSelected(other_identity));
    assert_eq!(switched_state.encrypted_note_unlock, EncryptedNoteUnlockState::Inactive);
    assert_eq!(switched_state.layout.overlay, OverlayState::None);

    let effects = state.reduce(NotoraAction::EncryptedNoteDialogSubmitRequested);
    assert!(matches!(
        effects.as_slice(),
        [NotoraEffect::UnlockEncryptedNote(unlock_request), NotoraEffect::Redraw]
            if unlock_request.request == request && unlock_request.generation == 1
    ));
    assert!(matches!(
        state.encrypted_note_unlock,
        EncryptedNoteUnlockState::Submitting {
            request: pending_request,
            generation: 1,
            ..
        } if pending_request == request
    ));

    assert!(matches!(state.encrypted_note_unlock, EncryptedNoteUnlockState::Submitting { .. }));
    let _ = state.reduce(NotoraAction::EncryptedNoteUnlockCompleted);
    assert_eq!(state.encrypted_note_unlock, EncryptedNoteUnlockState::Inactive);
    assert_eq!(state.layout.focus_target, FocusTarget::Editor);
}

#[test]
fn encrypted_note_unlock_rejects_five_character_password() {
    use ui::core::widget::SensitiveText;

    let note_id = NoteId::generate();
    let request =
        DocumentLoadRequest { identity: DocumentIdentity::Note(note_id), selection_generation: 4 };
    let mut state = state_with_active_workspace();
    state.library.selected_card = Some(request.identity);
    state.library.selected_document_generation = request.selection_generation;
    let _ = state.reduce(NotoraAction::EncryptedNoteUnlockRequired {
        request,
        title: "秘密笔记".to_owned(),
        metadata: NoteEditorMetadata {
            note_id,
            created_at: SystemTime::UNIX_EPOCH,
            modified_at: SystemTime::UNIX_EPOCH,
            encryption: notora_core::NoteEncryption::Encrypted,
            title_initialization: notora_core::TitleInitialization::Independent,
            file_name_binding: notora_core::NoteFileNameBinding::TitleBound { disambiguator: 1 },
            title_revision: 0,
        },
        tags: Vec::new(),
    });
    let _ = state
        .reduce(NotoraAction::EncryptedNotePasswordChanged(SensitiveText::new("abc12".to_owned())));

    assert_eq!(
        state.reduce(NotoraAction::EncryptedNoteDialogSubmitRequested),
        vec![NotoraEffect::Redraw]
    );
    assert!(matches!(
        state.encrypted_note_unlock,
        EncryptedNoteUnlockState::Editing { error_message: Some(message), .. }
            if message == "密码至少需要 6 个字符"
    ));
}

#[test]
fn focus_requests_cannot_escape_an_open_product_overlay() {
    let mut state = state_with_active_workspace();
    let _ = state.reduce(NotoraAction::OpenNewDocumentMenu);

    let _ = state.reduce(NotoraAction::FocusRequested(FocusTarget::Editor));

    assert_eq!(state.layout.overlay, OverlayState::NewDocumentMenu);
    assert_eq!(state.layout.focus_target, FocusTarget::Overlay);
}

#[test]
fn missing_workspace_root_cannot_open_the_new_document_menu() {
    let mut state = NotoraState::default();

    assert_eq!(state.reduce(NotoraAction::OpenNewDocumentMenu), vec![NotoraEffect::Redraw]);
    assert_eq!(state.layout.overlay, OverlayState::None);
}

#[test]
fn workspace_root_selection_is_separate_from_note_creation() {
    let mut state = NotoraState::default();

    assert_eq!(
        state.reduce(NotoraAction::WorkspaceRootSelectionRequested),
        vec![NotoraEffect::ChooseWorkspaceRoot, NotoraEffect::Redraw]
    );
    assert_eq!(state.layout.overlay, OverlayState::None);
}

#[test]
fn escape_closes_the_new_document_menu() {
    let mut state = state_with_active_workspace();
    let _ = state.reduce(NotoraAction::OpenNewDocumentMenu);

    assert_eq!(state.reduce(NotoraAction::EscapePressed), vec![NotoraEffect::Redraw]);
    assert_eq!(state.layout.overlay, OverlayState::None);
    assert_eq!(state.layout.focus_target, FocusTarget::NavigationTree);
}

#[test]
fn trash_scope_cannot_open_the_new_document_menu() {
    let mut state = state_with_active_workspace();
    let _ = state.reduce(NotoraAction::NavigationSelected(NavigationScope::Trash));

    assert_eq!(state.reduce(NotoraAction::OpenNewDocumentMenu), vec![NotoraEffect::Redraw]);
    assert_eq!(state.layout.overlay, OverlayState::None);
    assert_ne!(state.layout.focus_target, FocusTarget::Overlay);
}

#[test]
fn activating_the_selected_card_promotes_preview_and_focuses_the_editor() {
    let mut state = NotoraState::default();
    let identity = notora_core::DocumentIdentity::Note(notora_core::NoteId::generate());
    let _ = state.reduce(NotoraAction::CardSelected(identity));

    assert_eq!(
        state.reduce(NotoraAction::CardActivated(identity)),
        vec![NotoraEffect::PromoteActivePreview, NotoraEffect::Redraw]
    );
    assert_eq!(state.layout.focus_target, FocusTarget::Editor);
}

#[test]
fn repeated_selection_of_the_same_document_uses_distinct_load_generations() {
    let mut state = NotoraState::default();
    let identity = notora_core::DocumentIdentity::Note(notora_core::NoteId::generate());

    let first_effects = state.reduce(NotoraAction::CardSelected(identity));
    let second_effects = state.reduce(NotoraAction::CardSelected(identity));

    assert!(matches!(
        first_effects.as_slice(),
        [
            NotoraEffect::PrepareDocument(crate::action::DocumentLoadRequest {
                identity: first_identity,
                selection_generation: 1,
            }),
            NotoraEffect::Redraw,
        ] if *first_identity == identity
    ));
    assert!(matches!(
        second_effects.as_slice(),
        [
            NotoraEffect::PrepareDocument(crate::action::DocumentLoadRequest {
                identity: second_identity,
                selection_generation: 2,
            }),
            NotoraEffect::Redraw,
        ] if *second_identity == identity
    ));
    assert_eq!(state.library.selected_document_generation, 2);
}

#[test]
fn stale_a_b_a_search_completion_cannot_replace_the_latest_generation() {
    let mut controller = SearchController::default();
    controller.set_active_workspace(notora_core::WorkspaceId::generate(), 1);
    let start = Instant::now();
    controller.schedule_committed_query("same".to_owned(), start);
    let first_request = controller
        .take_due_request(start + SEARCH_DEBOUNCE_DELAY)
        .expect("first search should become due");
    controller.schedule_committed_query("other".to_owned(), start);
    let _ = controller
        .take_due_request(start + SEARCH_DEBOUNCE_DELAY)
        .expect("intermediate search should become due");
    controller.schedule_committed_query("same".to_owned(), start + Duration::from_millis(1));
    let latest_request = controller
        .take_due_request(start + SEARCH_DEBOUNCE_DELAY + Duration::from_millis(1))
        .expect("latest repeated search should become due");
    let mut state = NotoraState::default();
    let _ = state.reduce(NotoraAction::SearchCommitted {
        query: latest_request.query,
        search_generation: Some(latest_request.search_generation),
    });
    let stale_query = CardQuery {
        scope: NavigationScope::Search { query: first_request.query },
        cursor: None,
        page_size: crate::action::DEFAULT_CARD_PAGE_SIZE,
        search_generation: Some(first_request.search_generation),
    };

    assert!(
        state
            .reduce(NotoraAction::CardQueryCompleted {
                query: stale_query,
                page: CatalogCardPage { cards: Vec::new(), next_cursor: None },
            })
            .is_empty()
    );
    assert!(matches!(
        state.library.card_page,
        CardPageState::LoadingInitial { query }
            if query.search_generation == Some(latest_request.search_generation)
    ));
}

#[test]
fn escape_closes_overlay_then_clears_search_then_focuses_navigation() {
    let mut state = NotoraState::default();
    let _ = state.reduce(NotoraAction::OpenSettings);
    assert_eq!(state.layout.overlay, OverlayState::Settings);
    let _ = state.reduce(NotoraAction::EscapePressed);
    assert_eq!(state.layout.overlay, OverlayState::None);

    let _ = state.reduce(NotoraAction::SearchCommitted {
        query: "idea".to_owned(),
        search_generation: None,
    });
    let _ = state.reduce(NotoraAction::EscapePressed);
    assert_eq!(state.library.navigation_scope, NavigationScope::WorkspaceRoot);

    state.layout.focus_target = FocusTarget::Editor;
    let _ = state.reduce(NotoraAction::EscapePressed);
    assert_eq!(state.layout.focus_target, FocusTarget::NavigationTree);
}

#[test]
fn compact_navigation_and_back_use_explicit_mutually_exclusive_layout_state() {
    let mut state = NotoraState::default();
    let identity = notora_core::DocumentIdentity::Note(notora_core::NoteId::generate());
    state.library.selected_card = Some(identity);

    let _ = state.reduce(NotoraAction::CompactNavigationRequested);
    assert_eq!(state.layout.compact_navigation, CompactNavigation::Visible);

    let _ = state.reduce(NotoraAction::CardActivated(identity));
    assert_eq!(state.layout.compact_content, CompactContent::Editor);

    let _ = state.reduce(NotoraAction::CompactBackRequested);
    assert_eq!(state.layout.compact_content, CompactContent::CardList);
    assert_eq!(state.layout.compact_navigation, CompactNavigation::Hidden);
}

#[test]
fn compact_navigation_toggle_preserves_the_editor_and_persistent_pane_visibility() {
    let mut state = NotoraState::default();
    state.layout.responsive_mode = crate::ResponsiveLayoutMode::EditorOverlay;
    state.layout.compact_content = CompactContent::Editor;
    let visibility = state.layout.navigation_pane_visibility;
    for expected in [CompactNavigation::Visible, CompactNavigation::Hidden] {
        assert_eq!(
            state.reduce(NotoraAction::NavigationPaneVisibilityToggled),
            vec![NotoraEffect::Redraw]
        );
        assert_eq!(state.layout.compact_navigation, expected);
        assert_eq!(state.layout.compact_content, CompactContent::Editor);
        assert_eq!(state.layout.navigation_pane_visibility, visibility);
    }
    assert_eq!(state.layout.focus_target, FocusTarget::Editor);
}

#[test]
fn navigation_pane_toggle_preserves_width_and_moves_focus_to_visible_content() {
    let mut state = NotoraState::default();
    state.layout.navigation_width_logical = 268.0;
    state.layout.focus_target = FocusTarget::NavigationSearch;

    let collapse_effects = state.reduce(NotoraAction::NavigationPaneVisibilityToggled);

    assert_eq!(state.layout.navigation_pane_visibility, NavigationPaneVisibility::Collapsed);
    assert_eq!(state.layout.navigation_width_logical, 268.0);
    assert_eq!(state.layout.focus_target, FocusTarget::CardList);
    assert_eq!(collapse_effects, vec![NotoraEffect::PersistLayout, NotoraEffect::Redraw]);

    let _ = state.reduce(NotoraAction::NavigationPaneVisibilityToggled);

    assert_eq!(state.layout.navigation_pane_visibility, NavigationPaneVisibility::Expanded);
    assert_eq!(state.layout.navigation_width_logical, 268.0);
    assert_eq!(state.layout.focus_target, FocusTarget::NavigationTree);
}

#[test]
fn concurrent_save_requires_an_explicit_typed_resolution() {
    let mut state = NotoraState::default();
    let identity = notora_core::DocumentIdentity::Note(notora_core::NoteId::generate());
    let _ = state.reduce(NotoraAction::SaveConflictDetected { identity, content_revision: 7 });

    assert_eq!(state.layout.overlay, OverlayState::SaveConflict);
    assert_eq!(state.layout.focus_target, FocusTarget::Overlay);

    assert_eq!(
        state.reduce(NotoraAction::SaveConflictResolutionRequested(
            crate::action::ConflictResolution::RetrySave,
        )),
        vec![
            NotoraEffect::ResolveSaveConflict(crate::action::SaveConflictRequest {
                identity,
                resolution: crate::action::ConflictResolution::RetrySave,
            }),
            NotoraEffect::Redraw,
        ]
    );
}

#[test]
fn files_scope_creates_each_document_kind_without_a_workspace() {
    for kind in [DocumentKind::Markdown, DocumentKind::Text, DocumentKind::Mindmap] {
        let mut state = NotoraState::default();
        state.library.navigation_scope = NavigationScope::ExternalFiles;
        assert_eq!(state.reduce(NotoraAction::OpenNewDocumentMenu), vec![NotoraEffect::Redraw]);
        assert_eq!(state.layout.overlay, OverlayState::NewDocumentMenu);
        assert_eq!(
            state.reduce(NotoraAction::CreateRequested(kind)),
            vec![NotoraEffect::CreateUntitledExternal(kind), NotoraEffect::Redraw]
        );
        assert_eq!(state.layout.overlay, OverlayState::None);
    }
}

#[test]
fn files_scope_creates_an_untitled_external_document() {
    let mut state = state_with_active_workspace();
    let _ = state.reduce(NotoraAction::NavigationSelected(NavigationScope::ExternalFiles));

    assert_eq!(
        state.reduce(NotoraAction::CreateRequested(DocumentKind::Markdown)),
        vec![NotoraEffect::CreateUntitledExternal(DocumentKind::Markdown), NotoraEffect::Redraw,]
    );
}

#[test]
fn card_query_next_page_merges_cards_and_preserves_selection() {
    let mut state = NotoraState::default();
    let first_note_id = NoteId::generate();
    let second_note_id = NoteId::generate();
    let initial_query = CardQuery::from(NavigationScope::WorkspaceRoot);
    let _ = state.reduce(NotoraAction::NavigationSelected(NavigationScope::WorkspaceRoot));
    let _ = state.reduce(NotoraAction::CardQueryCompleted {
        query: initial_query.clone(),
        page: CatalogCardPage {
            cards: vec![card(first_note_id, "first", 20)],
            next_cursor: Some(CatalogCardCursor {
                modified_nanoseconds: 20,
                relative_path: "notes/first.md".into(),
                note_id: first_note_id,
            }),
        },
    });
    let _ = state.reduce(NotoraAction::CardSelected(DocumentIdentity::Note(first_note_id)));

    let effects = state.reduce(NotoraAction::CardListScrolled { offset_px: 240.0, near_end: true });
    let next_query = initial_query.next_page(CatalogCardCursor {
        modified_nanoseconds: 20,
        relative_path: "notes/first.md".into(),
        note_id: first_note_id,
    });
    assert_eq!(effects, vec![NotoraEffect::QueryCards(next_query.clone()), NotoraEffect::Redraw]);
    assert!(matches!(
        &state.library.card_page,
        CardPageState::LoadingNextPage { query, cards }
            if query == &next_query && cards.len() == 1
    ));

    let _ = state.reduce(NotoraAction::CardQueryCompleted {
        query: next_query,
        page: CatalogCardPage {
            cards: vec![card(second_note_id, "second", 10)],
            next_cursor: None,
        },
    });
    assert_eq!(state.library.selected_card, Some(DocumentIdentity::Note(first_note_id)));
    assert!(matches!(
        &state.library.card_page,
        CardPageState::Ready { cards, next_cursor: None, .. }
            if cards.iter().map(|card| card.note_id).collect::<Vec<_>>()
                == vec![first_note_id, second_note_id]
    ));
}

#[test]
fn card_query_empty_result_uses_an_explicit_empty_state() {
    let mut state = NotoraState::default();
    let query = CardQuery::from(NavigationScope::Starred);
    let _ = state.reduce(NotoraAction::NavigationSelected(NavigationScope::Starred));

    let _ = state.reduce(NotoraAction::CardQueryCompleted {
        query: query.clone(),
        page: CatalogCardPage { cards: Vec::new(), next_cursor: None },
    });

    assert_eq!(state.library.card_page, CardPageState::Empty { query });
}

#[test]
fn new_workspace_form_builds_a_create_transition_from_name_and_location() {
    let mut state = NotoraState::default();

    assert_eq!(
        state.reduce(NotoraAction::OpenWorkspaceCreationRequested),
        vec![NotoraEffect::Redraw]
    );
    assert_eq!(state.layout.overlay, OverlayState::NewWorkspace);
    let _ = state.reduce(NotoraAction::WorkspaceCreationNameChanged("我的工作区".to_owned()));
    let _ = state.reduce(NotoraAction::WorkspaceCreationLocationSelected("/tmp".into()));

    let effects = state.reduce(NotoraAction::WorkspaceCreationCommitRequested);

    assert_eq!(
        effects,
        vec![
            NotoraEffect::PrepareWorkspaceTransition(
                crate::action::WorkspaceTransitionRequest::Create {
                    root: "/tmp/我的工作区".into(),
                },
            ),
            NotoraEffect::Redraw,
        ]
    );
    assert_eq!(state.layout.overlay, OverlayState::None);
}

#[test]
fn invalid_workspace_name_keeps_the_form_open_without_starting_a_transition() {
    let mut state = NotoraState::default();
    let _ = state.reduce(NotoraAction::OpenWorkspaceCreationRequested);
    let _ = state.reduce(NotoraAction::WorkspaceCreationNameChanged("a/b".to_owned()));
    let _ = state.reduce(NotoraAction::WorkspaceCreationLocationSelected("/tmp".into()));

    assert_eq!(
        state.reduce(NotoraAction::WorkspaceCreationCommitRequested),
        vec![NotoraEffect::Redraw]
    );
    assert_eq!(state.layout.overlay, OverlayState::NewWorkspace);
    assert!(state.library.last_command_error.is_some());
}
