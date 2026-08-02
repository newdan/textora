use std::fs;
use std::time::{Duration, UNIX_EPOCH};

use notora_core::catalog::SearchIndexEntry;
use notora_core::{
    Catalog, CatalogNote, DocumentKind, NavigationScope, NoteId, Workspace, move_to_trash,
    permanently_delete_trashed_note, restore_from_trash, restore_from_trash_with_renamed_path,
};

fn insert_note(catalog: &Catalog, note_id: NoteId, relative_path: &str) {
    catalog
        .upsert_active_note(&CatalogNote {
            note_id,
            relative_path: relative_path.into(),
            kind: DocumentKind::Markdown,
            title: relative_path.to_owned(),
            excerpt: "fixture".to_owned(),
            modified_at: UNIX_EPOCH + Duration::from_secs(1),
            file_size: 8,
            content_hash: relative_path.as_bytes().to_vec(),
            starred: false,
        })
        .expect("fixture note should persist");
}

#[test]
fn trash_excludes_notes_from_search_star_and_tag_scopes_then_restore_keeps_metadata() {
    let directory = tempfile::tempdir().expect("workspace test directory should exist");
    let workspace = Workspace::open_or_initialize(directory.path()).expect("workspace should open");
    let catalog = Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
        .expect("catalog should initialize");
    let note_id = NoteId::generate();
    let note_path = workspace.root().join("plans/roadmap.md");
    fs::create_dir_all(note_path.parent().expect("fixture note should have a parent"))
        .expect("fixture directory should create");
    fs::write(&note_path, "# Roadmap\nlaunch plan").expect("fixture note should write");
    insert_note(&catalog, note_id, "plans/roadmap.md");
    let tag = catalog.create_tag("Plan").expect("tag should create");
    assert!(catalog.toggle_note_starred(note_id).expect("star should toggle"));
    assert!(catalog.attach_tag(note_id, tag.tag_id).expect("tag should attach"));
    catalog
        .index_note_batch(&[SearchIndexEntry {
            note_id,
            title: "Roadmap".to_owned(),
            relative_path: "plans/roadmap.md".into(),
            body: "launch plan".to_owned(),
            tags: vec!["Plan".to_owned()],
        }])
        .expect("fixture should index");

    move_to_trash(&workspace, &catalog, note_id).expect("note should trash");
    assert!(catalog.search_active_notes("plan", 10).expect("search should run").is_empty());
    assert!(
        catalog
            .query_catalog_cards(&NavigationScope::Starred, None, 10)
            .expect("star scope should query")
            .cards
            .is_empty()
    );
    assert!(
        catalog
            .query_catalog_cards(&NavigationScope::Tag { tag_id: tag.tag_id }, None, 10)
            .expect("tag scope should query")
            .cards
            .is_empty()
    );

    restore_from_trash(&workspace, &catalog, note_id).expect("note should restore");
    let restored =
        catalog.active_note(note_id).expect("restored note should query").expect("note exists");
    assert!(restored.starred);
    assert_eq!(catalog.tags_for_note(note_id).expect("tag should remain"), vec![tag]);
    move_to_trash(&workspace, &catalog, note_id).expect("note should trash again");
    permanently_delete_trashed_note(&workspace, &catalog, note_id)
        .expect("exact trashed note should permanently delete");
    assert!(catalog.active_note(note_id).expect("deleted note should query").is_none());
}

#[test]
fn restore_conflict_keeps_the_existing_file_and_restores_metadata_under_a_new_path() {
    let directory = tempfile::tempdir().expect("workspace test directory should exist");
    let workspace = Workspace::open_or_initialize(directory.path()).expect("workspace should open");
    let catalog = Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
        .expect("catalog should initialize");
    let note_id = NoteId::generate();
    let note_path = workspace.root().join("roadmap.md");
    fs::write(&note_path, "# Original").expect("fixture note should write");
    insert_note(&catalog, note_id, "roadmap.md");
    let tag = catalog.create_tag("Release").expect("tag should create");
    assert!(catalog.attach_tag(note_id, tag.tag_id).expect("tag should attach"));
    move_to_trash(&workspace, &catalog, note_id).expect("note should move to trash");
    fs::write(&note_path, "# Existing").expect("conflicting note should write");

    assert!(matches!(
        restore_from_trash(&workspace, &catalog, note_id),
        Err(notora_core::TrashError::RestoreConflict { .. })
    ));
    restore_from_trash_with_renamed_path(&workspace, &catalog, note_id)
        .expect("renamed restore should preserve both files");

    assert_eq!(fs::read_to_string(&note_path).expect("existing note should remain"), "# Existing");
    let restored = catalog
        .active_note(note_id)
        .expect("restored note should query")
        .expect("restored note should remain active");
    assert_ne!(restored.relative_path, std::path::PathBuf::from("roadmap.md"));
    assert!(workspace.root().join(restored.relative_path).is_file());
    assert_eq!(catalog.tags_for_note(note_id).expect("metadata should remain"), vec![tag]);
}

#[test]
fn metadata_tag_lifecycle_keeps_note_content_and_removes_only_deleted_tag_links() {
    let directory = tempfile::tempdir().expect("workspace test directory should exist");
    let workspace = Workspace::open_or_initialize(directory.path()).expect("workspace should open");
    let catalog = Catalog::open(&workspace.metadata_directory().join("catalog.sqlite3"))
        .expect("catalog should initialize");
    let note_id = NoteId::generate();
    let note_path = workspace.root().join("release.md");
    fs::write(&note_path, "# Release").expect("fixture note should write");
    insert_note(&catalog, note_id, "release.md");
    let release_tag = catalog.create_tag("Release").expect("tag should create");
    let planning_tag = catalog.create_tag("Planning").expect("tag should create");
    assert!(catalog.attach_tag(note_id, release_tag.tag_id).expect("tag should attach"));
    assert!(catalog.attach_tag(note_id, planning_tag.tag_id).expect("tag should attach"));
    catalog
        .rename_tag(release_tag.tag_id, "Shipped")
        .expect("tag should rename without changing identity");
    assert!(catalog.detach_tag(note_id, planning_tag.tag_id).expect("tag should detach"));
    assert!(
        !catalog.detach_tag(note_id, planning_tag.tag_id).expect("detach should be idempotent")
    );
    assert!(catalog.delete_tag(release_tag.tag_id).expect("tag should delete"));

    assert!(note_path.is_file());
    assert!(catalog.tags_for_note(note_id).expect("note metadata should query").is_empty());
    assert!(catalog.active_note(note_id).expect("note should query").is_some());
}
