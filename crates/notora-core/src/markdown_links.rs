use std::ops::Range;
use std::path::{Component, Path, PathBuf};

use percent_encoding::percent_decode_str;
use pulldown_cmark::{Event, Options, Parser, Tag};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkdownPathReferenceKind {
    Link,
    Image,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownPathReference {
    pub source_relative_path: PathBuf,
    pub target_relative_path: PathBuf,
    pub source_byte_range: Range<usize>,
    pub kind: MarkdownPathReferenceKind,
}

/// 提取能在工作区内按标准 Markdown 语义解析的相对文件引用。
pub fn extract_markdown_path_references(
    source_relative_path: &Path,
    markdown: &str,
) -> Vec<MarkdownPathReference> {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_HEADING_ATTRIBUTES;
    Parser::new_ext(markdown, options)
        .into_offset_iter()
        .filter_map(|(event, source_byte_range)| {
            let (destination, kind) = match event {
                Event::Start(Tag::Link { dest_url, .. }) => {
                    (dest_url, MarkdownPathReferenceKind::Link)
                }
                Event::Start(Tag::Image { dest_url, .. }) => {
                    (dest_url, MarkdownPathReferenceKind::Image)
                }
                _ => return None,
            };
            let target_relative_path = resolve_relative_destination(
                source_relative_path.parent().unwrap_or_else(|| Path::new("")),
                &destination,
            )?;
            Some(MarkdownPathReference {
                source_relative_path: source_relative_path.to_path_buf(),
                target_relative_path,
                source_byte_range,
                kind,
            })
        })
        .collect()
}

fn resolve_relative_destination(source_directory: &Path, destination: &str) -> Option<PathBuf> {
    let path_part = destination.split(['#', '?']).next()?.trim();
    if path_part.is_empty() || has_uri_scheme(path_part) {
        return None;
    }
    let decoded = percent_decode_str(path_part).decode_utf8().ok()?;
    let decoded_path = Path::new(decoded.as_ref());
    if decoded_path.is_absolute() {
        return None;
    }
    normalize_workspace_relative_path(&source_directory.join(decoded_path))
}

fn has_uri_scheme(destination: &str) -> bool {
    let Some(colon_index) = destination.find(':') else {
        return false;
    };
    let scheme = &destination[..colon_index];
    !scheme.is_empty()
        && scheme.bytes().enumerate().all(|(index, byte)| match (index, byte) {
            (0, byte) => byte.is_ascii_alphabetic(),
            (_, byte) => byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'),
        })
}

fn normalize_workspace_relative_path(path: &Path) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(segment) => normalized.push(segment),
            Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!normalized.as_os_str().is_empty()).then_some(normalized)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{MarkdownPathReferenceKind, extract_markdown_path_references};

    #[test]
    fn parser_extracts_links_and_images_but_not_code_or_external_destinations() {
        let markdown = r#"
[inline](../目标.md#章节)
[reference][target]
![image](assets/示意%20图.png)
`[code](../目标.md)`
```md
[fenced](../目标.md)
```
[web](https://example.com/目标.md)
[mail](mailto:test@example.com)
[anchor](#local)

[target]: ../引用目标.md "title"
"#;

        let references = extract_markdown_path_references(Path::new("docs/source.md"), markdown);

        assert_eq!(references.len(), 3);
        assert_eq!(references[0].target_relative_path, Path::new("目标.md"));
        assert_eq!(references[0].kind, MarkdownPathReferenceKind::Link);
        assert_eq!(references[1].target_relative_path, Path::new("引用目标.md"));
        assert_eq!(references[2].target_relative_path, Path::new("docs/assets/示意 图.png"));
        assert_eq!(references[2].kind, MarkdownPathReferenceKind::Image);
        assert!(references.iter().all(|reference| !reference.source_byte_range.is_empty()));
    }

    #[test]
    fn parser_rejects_links_that_escape_the_workspace_root() {
        let references = extract_markdown_path_references(
            Path::new("source.md"),
            "[outside](../outside.md) [inside](inside.md)",
        );

        assert_eq!(references.len(), 1);
        assert_eq!(references[0].target_relative_path, Path::new("inside.md"));
    }
}
