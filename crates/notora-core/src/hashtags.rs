use std::collections::HashSet;

/// 从正文中按首次出现顺序提取 `#标签`。
pub fn extract_hashtags(source: &str) -> Vec<String> {
    let characters = source.chars().collect::<Vec<_>>();
    let mut tags = Vec::new();
    let mut seen = HashSet::new();
    let mut index = 0;

    while index < characters.len() {
        if characters[index] != '#' || !has_tag_boundary(&characters, index) {
            index += 1;
            continue;
        }
        let tag_start = index + 1;
        if tag_start >= characters.len() || !is_tag_start(characters[tag_start]) {
            index += 1;
            continue;
        }
        let mut tag_end = tag_start + 1;
        while tag_end < characters.len() && is_tag_character(characters[tag_end]) {
            tag_end += 1;
        }
        let tag = characters[tag_start..tag_end].iter().collect::<String>();
        if seen.insert(tag.clone()) {
            tags.push(tag);
        }
        index = tag_end;
    }

    tags
}

fn has_tag_boundary(characters: &[char], marker_index: usize) -> bool {
    marker_index == 0
        || (characters[marker_index - 1] != '#' && !is_tag_character(characters[marker_index - 1]))
}

fn is_tag_start(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

fn is_tag_character(character: char) -> bool {
    is_tag_start(character) || character == '-'
}

#[cfg(test)]
mod tests {
    use super::extract_hashtags;

    #[test]
    fn extracts_unicode_hashtags_and_deduplicates_exact_names() {
        assert_eq!(
            extract_hashtags("发布 #计划-2026，关联 #研发_一组。再次提到 #计划-2026"),
            vec!["计划-2026".to_owned(), "研发_一组".to_owned()]
        );
    }

    #[test]
    fn ignores_markdown_headings_embedded_markers_and_empty_tags() {
        assert_eq!(
            extract_hashtags(
                "# 一级标题\n正文中的 abc#错误 和 ##二级标题不算。\n有效：(#标签) #_草稿 #"
            ),
            vec!["标签".to_owned(), "_草稿".to_owned()]
        );
    }
}
