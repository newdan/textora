//! CLI argument parsing for edit+.

use std::path::PathBuf;

/// Parsed CLI arguments.
#[derive(Debug, PartialEq)]
pub struct CliArgs {
    /// File path to open (if provided).
    pub file: Option<PathBuf>,
    /// Run in headless mode (no window).
    pub headless: bool,
}

/// Parse CLI arguments.
///
/// Usage: `edit-plus [OPTIONS] [FILE]`
///
/// Options:
///   --headless  Run without creating a window (for testing GPU init)
pub fn parse_args(args: &[String]) -> CliArgs {
    let mut file = None;
    let mut headless = false;

    for arg in &args[1..] {
        if arg == "--headless" {
            headless = true;
        } else if file.is_none() {
            file = Some(PathBuf::from(arg));
        }
    }

    CliArgs { file, headless }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(strs: &[&str]) -> Vec<String> {
        strs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn no_args() {
        let cli = parse_args(&args(&["edit-plus"]));
        assert_eq!(cli, CliArgs { file: None, headless: false });
    }

    #[test]
    fn file_arg() {
        let cli = parse_args(&args(&["edit-plus", "foo.txt"]));
        assert_eq!(cli, CliArgs { file: Some(PathBuf::from("foo.txt")), headless: false });
    }

    #[test]
    fn headless_flag() {
        let cli = parse_args(&args(&["edit-plus", "--headless"]));
        assert_eq!(cli, CliArgs { file: None, headless: true });
    }

    #[test]
    fn file_and_headless() {
        let cli = parse_args(&args(&["edit-plus", "--headless", "test.rs"]));
        assert_eq!(cli, CliArgs { file: Some(PathBuf::from("test.rs")), headless: true });
    }

    #[test]
    fn headless_after_file() {
        let cli = parse_args(&args(&["edit-plus", "test.rs", "--headless"]));
        assert_eq!(cli, CliArgs { file: Some(PathBuf::from("test.rs")), headless: true });
    }

    #[test]
    fn only_program_name() {
        let cli = parse_args(&args(&["edit-plus"]));
        assert!(!cli.headless);
        assert!(cli.file.is_none());
    }
}
