//! Integration test verifying that all root-level public API items are accessible.

use textora_app::{App, AppEvent, CliArgs, GpuError, headless_init, parse_args};

#[test]
fn root_exports_binary_contract() {
    // Verify CliArgs and parse_args are accessible from the crate root
    let args = vec!["NoteR".to_string(), "--headless".to_string()];
    let cli: CliArgs = parse_args(&args);
    assert!(cli.headless);

    // Verify App can be constructed
    let _app = App::new(None);

    // Verify AppEvent is accessible
    let _event: Option<AppEvent> = None;

    // Verify GpuError is accessible
    let _error: Option<GpuError> = None;

    // Verify headless_init is accessible (just reference the fn pointer)
    let _init = headless_init;
}
