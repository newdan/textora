//! Static boundary tests: enforce the single-apply router invariant.
//!
//! Every dispatch domain must return `AppEffect` to the top-level
//! `dispatch()` method, which is the *sole* place `apply_effect()` is
//! called. The tests below scan production source to guarantee this.

fn dispatch_sources() -> [(&'static str, &'static str); 9] {
    [
        ("app_dispatch.rs", include_str!("app_dispatch.rs")),
        ("app_scroll.rs", include_str!("app_scroll.rs")),
        ("commands.rs", include_str!("dispatch/commands.rs")),
        ("editor.rs", include_str!("dispatch/editor.rs")),
        ("mouse.rs", include_str!("dispatch/mouse.rs")),
        ("search.rs", include_str!("dispatch/search.rs")),
        ("tabs.rs", include_str!("dispatch/tabs.rs")),
        ("chrome.rs", include_str!("dispatch/chrome.rs")),
        ("viewport.rs", include_str!("dispatch/viewport.rs")),
    ]
}

/// Strip everything after `#[cfg(test)]` so test helper code doesn't
/// trigger false positives.
fn production_source(source: &str) -> &str {
    source.split("#[cfg(test)]").next().unwrap_or(source)
}

#[test]
fn only_router_applies_effect() {
    for (name, source) in dispatch_sources() {
        let source = production_source(source);
        let count = source.match_indices("apply_effect(").count();
        let expected = usize::from(name == "app_dispatch.rs");
        assert_eq!(count, expected, "{name}");
    }
}

#[test]
fn dispatch_domains_do_not_apply_global_followups_directly() {
    let forbidden = [
        "needs_redraw =",
        "request_redraw(",
        "invalidate_reshape(",
        "update_window_title(",
        "persist_workspace_state(",
        "settings_io::save(",
    ];
    for (name, source) in dispatch_sources() {
        let source = production_source(source);
        for needle in forbidden {
            assert!(!source.contains(needle), "{name} contains forbidden call {needle}");
        }
    }
}

#[test]
fn workspace_stores_document_models_without_doc_item_runtime_fallback() {
    let workspace_source = include_str!("../../appkit-shell/src/workspace.rs");

    assert!(
        workspace_source.contains("WorkspaceModel<DocumentModel>"),
        "workspace must store headless document models directly"
    );
    assert!(
        !workspace_source.contains("WorkspaceModel<DocItem>"),
        "workspace must not retain the transitional DocItem model"
    );
    assert!(
        !workspace_source.contains("struct DocItem"),
        "workspace must not define a local DocItem adapter"
    );
    assert!(
        !workspace_source.contains("pending_runtimes"),
        "workspace must not retain a runtime fallback store"
    );
}
