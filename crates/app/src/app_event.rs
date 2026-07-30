pub use appkit_shell::ShellEvent as AppEvent;

#[cfg(test)]
mod tests {
    use super::AppEvent;

    #[test]
    fn background_services_have_a_semantic_start_event() {
        assert!(matches!(AppEvent::StartBackgroundServices, AppEvent::StartBackgroundServices));
    }

    #[test]
    fn production_event_boundary_reexports_only_shell_events() {
        let production_source = include_str!("app_event.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("event production source should precede tests");
        let local_event_enum = ["pub ", "enum App", "Event"].concat();
        let legacy_recent_files = ["Recent", "Files", "Loaded"].concat();
        let legacy_sync_results = ["Sync", "Results", "Ready"].concat();
        let legacy_open_files = ["Open", "Files"].concat();
        let shell_reexport = ["pub ", "use appkit_shell::ShellEvent as App", "Event;"].concat();

        assert!(!production_source.contains(&local_event_enum));
        assert!(!production_source.contains(&legacy_recent_files));
        assert!(!production_source.contains(&legacy_sync_results));
        assert!(!production_source.contains(&legacy_open_files));
        assert!(production_source.contains(&shell_reexport));
    }
}
