use serde::{Deserialize, Serialize};
use std::ops::Range;

const LATIN_MIX_GAP_EM: f32 = 0.125;
const SENTENCE_LEFT_GAP_EM: f32 = 0.0625;
const SENTENCE_RIGHT_GAP_EM: f32 = 0.125;
const OPEN_OUTER_GAP_EM: f32 = 0.125;
const OPEN_INNER_GAP_EM: f32 = 0.0625;
const CLOSE_INNER_GAP_EM: f32 = 0.0625;
const CLOSE_OUTER_GAP_EM: f32 = 0.125;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TextSpacingMode {
    #[default]
    Natural,
    Verbatim,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypographyCluster {
    pub byte_range: Range<usize>,
    pub font_size: f32,
}

impl TypographyCluster {
    pub fn new(byte_range: Range<usize>, font_size: f32) -> Self {
        Self { byte_range, font_size }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GapCandidate {
    pub boundary: usize,
    pub advance: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct TypographyInput<'a> {
    pub text: &'a str,
    pub mode: TextSpacingMode,
    pub clusters: &'a [TypographyCluster],
    pub protected_ranges: &'a [Range<usize>],
}

impl<'a> TypographyInput<'a> {
    pub fn new(
        text: &'a str,
        mode: TextSpacingMode,
        clusters: &'a [TypographyCluster],
        protected_ranges: &'a [Range<usize>],
    ) -> Self {
        Self { text, mode, clusters, protected_ranges }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClusterClass {
    Han,
    Latin,
    Digit,
    SentencePunctuation,
    OpenBracket,
    CloseBracket,
    Whitespace,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuoteRole {
    Open,
    Close,
}

#[derive(Debug, Clone)]
struct ClusterView {
    cluster: TypographyCluster,
    class: ClusterClass,
    semantic_protected: bool,
    technical_protected: bool,
}

/// Compute natural mixed-script spacing candidates at UTF-8 cluster boundaries.
///
/// The returned candidates are generated before visual wrapping. A later layout
/// pass must discard candidates that cross a visual line boundary.
pub fn calculate_gaps(input: &TypographyInput<'_>) -> Vec<GapCandidate> {
    if input.mode == TextSpacingMode::Verbatim || input.clusters.len() < 2 {
        return Vec::new();
    }

    let technical_ranges = technical_token_ranges(input.text);
    let semantic_ranges = normalize_ranges(input.text.len(), input.protected_ranges);
    let technical_ranges = normalize_ranges(input.text.len(), &technical_ranges);
    let mut ordered_clusters = input.clusters.to_vec();
    if !clusters_are_ordered(&ordered_clusters) {
        ordered_clusters.sort_unstable_by_key(|cluster| cluster.byte_range.start);
    }
    let mut semantic_cursor = RangeCursor::new(&semantic_ranges);
    let mut technical_cursor = RangeCursor::new(&technical_ranges);
    let mut views = Vec::with_capacity(ordered_clusters.len());
    for cluster in ordered_clusters {
        let Some(value) = input.text.get(cluster.byte_range.clone()) else { continue };
        views.push(ClusterView {
            semantic_protected: semantic_cursor.overlaps(&cluster.byte_range),
            technical_protected: technical_cursor.overlaps(&cluster.byte_range),
            class: classify_cluster(value),
            cluster,
        });
    }
    let mut unique_views: Vec<ClusterView> = Vec::with_capacity(views.len());
    for view in views {
        if let Some(previous) = unique_views.last_mut()
            && previous.cluster.byte_range == view.cluster.byte_range
        {
            previous.cluster.font_size = mixed_font_size(previous, &view);
            previous.semantic_protected |= view.semantic_protected;
            previous.technical_protected |= view.technical_protected;
            continue;
        }
        unique_views.push(view);
    }
    let views = unique_views;

    let quote_roles = paired_quote_roles(input.text, &views, &semantic_ranges, &technical_ranges);
    let mut boundary_candidates = Vec::new();

    for index in 0..views.len().saturating_sub(1) {
        let left = &views[index];
        let right = &views[index + 1];
        if left.cluster.byte_range.end != right.cluster.byte_range.start
            || left.semantic_protected
            || right.semantic_protected
            || (left.technical_protected && right.technical_protected)
            || left.class == ClusterClass::Whitespace
            || right.class == ClusterClass::Whitespace
        {
            continue;
        }

        let boundary = right.cluster.byte_range.start;
        if is_mixed_script_boundary(left.class, right.class) {
            push_max_gap(
                &mut boundary_candidates,
                boundary,
                mixed_font_size(left, right) * LATIN_MIX_GAP_EM,
            );
        }

        add_bracket_gap(
            &mut boundary_candidates,
            boundary,
            left,
            right,
            quote_roles[index],
            quote_roles[index + 1],
        );
    }

    let mut sentence_candidates = Vec::new();
    add_sentence_group_gaps(&mut sentence_candidates, &views);
    merge_gap_candidates(boundary_candidates, sentence_candidates)
}

fn classify_cluster(value: &str) -> ClusterClass {
    let characters = value.chars().collect::<Vec<_>>();
    if characters.is_empty() {
        return ClusterClass::Other;
    }
    if characters.iter().all(|character| character.is_whitespace()) {
        return ClusterClass::Whitespace;
    }
    if characters.iter().any(|character| is_han_ideograph(*character)) {
        return ClusterClass::Han;
    }
    if characters
        .iter()
        .all(|character| character.is_ascii_digit() || is_combining_mark(*character))
        && characters.iter().any(|character| character.is_ascii_digit())
    {
        return ClusterClass::Digit;
    }
    if characters
        .iter()
        .all(|character| is_latin_letter(*character) || is_combining_mark(*character))
        && characters.iter().any(|character| is_latin_letter(*character))
    {
        return ClusterClass::Latin;
    }
    if characters.len() == 1 {
        return match characters[0] {
            ',' | '.' | ':' | ';' | '!' | '?' => ClusterClass::SentencePunctuation,
            '(' | '[' => ClusterClass::OpenBracket,
            ')' | ']' => ClusterClass::CloseBracket,
            _ => ClusterClass::Other,
        };
    }
    ClusterClass::Other
}

fn is_combining_mark(character: char) -> bool {
    let code_point = character as u32;
    matches!(
        code_point,
        0x0300..=0x036F
            | 0x1AB0..=0x1AFF
            | 0x1DC0..=0x1DFF
            | 0x20D0..=0x20FF
            | 0xFE20..=0xFE2F
    )
}

fn is_latin_letter(character: char) -> bool {
    let code_point = character as u32;
    character.is_alphabetic()
        && matches!(
            code_point,
            0x0041..=0x005A
                | 0x0061..=0x007A
                | 0x00C0..=0x02AF
                | 0x1E00..=0x1EFF
                | 0x2C60..=0x2C7F
                | 0xA720..=0xA7FF
                | 0xAB30..=0xAB6F
        )
}

fn is_han_ideograph(character: char) -> bool {
    let code_point = character as u32;
    matches!(
        code_point,
        0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0x20000..=0x2A6DF
            | 0x2A700..=0x2B73F
            | 0x2B740..=0x2B81F
            | 0x2B820..=0x2CEAF
            | 0x2CEB0..=0x2EBEF
            | 0x2F800..=0x2FA1F
            | 0x30000..=0x3134F
    )
}

fn is_mixed_script_boundary(left: ClusterClass, right: ClusterClass) -> bool {
    matches!(
        (left, right),
        (ClusterClass::Han, ClusterClass::Latin | ClusterClass::Digit)
            | (ClusterClass::Latin | ClusterClass::Digit, ClusterClass::Han)
    )
}

fn mixed_font_size(left: &ClusterView, right: &ClusterView) -> f32 {
    left.cluster.font_size.max(0.0).min(right.cluster.font_size.max(0.0))
}

fn add_bracket_gap(
    candidates: &mut Vec<GapCandidate>,
    boundary: usize,
    left: &ClusterView,
    right: &ClusterView,
    left_quote_role: Option<QuoteRole>,
    right_quote_role: Option<QuoteRole>,
) {
    let left_is_open =
        left.class == ClusterClass::OpenBracket || left_quote_role == Some(QuoteRole::Open);
    let right_is_open =
        right.class == ClusterClass::OpenBracket || right_quote_role == Some(QuoteRole::Open);
    let left_is_close =
        left.class == ClusterClass::CloseBracket || left_quote_role == Some(QuoteRole::Close);
    let right_is_close =
        right.class == ClusterClass::CloseBracket || right_quote_role == Some(QuoteRole::Close);

    if left.class == ClusterClass::Han && right_is_open {
        push_max_gap(candidates, boundary, mixed_font_size(left, right) * OPEN_OUTER_GAP_EM);
    }
    if left_is_open && right.class == ClusterClass::Han {
        push_max_gap(candidates, boundary, mixed_font_size(left, right) * OPEN_INNER_GAP_EM);
    }
    if left.class == ClusterClass::Han && right_is_close {
        push_max_gap(candidates, boundary, mixed_font_size(left, right) * CLOSE_INNER_GAP_EM);
    }
    if left_is_close && right.class == ClusterClass::Han {
        push_max_gap(candidates, boundary, mixed_font_size(left, right) * CLOSE_OUTER_GAP_EM);
    }
}

fn add_sentence_group_gaps(candidates: &mut Vec<GapCandidate>, views: &[ClusterView]) {
    let mut group_start = 0;
    while group_start < views.len() {
        if views[group_start].class != ClusterClass::SentencePunctuation
            || views[group_start].semantic_protected
            || views[group_start].technical_protected
        {
            group_start += 1;
            continue;
        }
        let mut group_end = group_start + 1;
        while group_end < views.len()
            && views[group_end].class == ClusterClass::SentencePunctuation
            && !views[group_end].semantic_protected
            && !views[group_end].technical_protected
            && views[group_end - 1].cluster.byte_range.end
                == views[group_end].cluster.byte_range.start
        {
            group_end += 1;
        }

        let left = group_start.checked_sub(1).and_then(|index| views.get(index));
        let right = views.get(group_end);
        let left_is_han = left.is_some_and(|view| {
            view.class == ClusterClass::Han && !view.semantic_protected && !view.technical_protected
        });
        let right_is_han = right.is_some_and(|view| {
            view.class == ClusterClass::Han && !view.semantic_protected && !view.technical_protected
        });
        if left_is_han
            && let Some(left) = left.filter(|view| {
                !view.semantic_protected
                    && !view.technical_protected
                    && view.class != ClusterClass::Whitespace
                    && view.cluster.byte_range.end == views[group_start].cluster.byte_range.start
            })
        {
            let boundary = views[group_start].cluster.byte_range.start;
            push_max_gap(
                candidates,
                boundary,
                mixed_font_size(left, &views[group_start]) * SENTENCE_LEFT_GAP_EM,
            );
        }
        if (left_is_han || right_is_han)
            && let Some(right) = right.filter(|view| {
                !view.semantic_protected
                    && !view.technical_protected
                    && view.class != ClusterClass::Whitespace
                    && views[group_end - 1].cluster.byte_range.end == view.cluster.byte_range.start
            })
        {
            let boundary = right.cluster.byte_range.start;
            push_max_gap(
                candidates,
                boundary,
                mixed_font_size(&views[group_end - 1], right) * SENTENCE_RIGHT_GAP_EM,
            );
        }
        group_start = group_end;
    }
}

fn paired_quote_roles(
    text: &str,
    views: &[ClusterView],
    semantic_ranges: &[Range<usize>],
    technical_ranges: &[Range<usize>],
) -> Vec<Option<QuoteRole>> {
    let paired_quotes = source_quote_roles(text, semantic_ranges, technical_ranges);
    let mut quote_cursor = 0;
    views
        .iter()
        .map(|view| {
            while quote_cursor < paired_quotes.len()
                && paired_quotes[quote_cursor].0 < view.cluster.byte_range.start
            {
                quote_cursor += 1;
            }
            paired_quotes.get(quote_cursor).and_then(|&(offset, role)| {
                (view.cluster.byte_range == (offset..offset + 1)).then_some(role)
            })
        })
        .collect()
}

fn source_quote_roles(
    text: &str,
    semantic_ranges: &[Range<usize>],
    technical_ranges: &[Range<usize>],
) -> Vec<(usize, QuoteRole)> {
    let mut roles = Vec::new();
    let mut pending_open = None;
    let mut semantic_cursor = RangeCursor::new(semantic_ranges);
    let mut technical_cursor = RangeCursor::new(technical_ranges);
    for (offset, character) in text.char_indices() {
        if character == '\n' {
            pending_open = None;
            continue;
        }
        if character != '\"' {
            continue;
        }
        let quote_range = offset..offset + character.len_utf8();
        if semantic_cursor.overlaps(&quote_range)
            || technical_cursor.overlaps(&quote_range)
            || is_escaped_quote(text, &quote_range)
        {
            continue;
        }
        if let Some(open_offset) = pending_open.take() {
            roles.push((open_offset, QuoteRole::Open));
            roles.push((offset, QuoteRole::Close));
        } else {
            pending_open = Some(offset);
        }
    }
    roles
}

fn is_escaped_quote(text: &str, quote_range: &Range<usize>) -> bool {
    let mut slash_count = 0;
    let prefix = &text[..quote_range.start];
    for character in prefix.chars().rev() {
        if character != '\\' {
            break;
        }
        slash_count += 1;
    }
    slash_count % 2 == 1
}

fn push_max_gap(candidates: &mut Vec<GapCandidate>, boundary: usize, advance: f32) {
    if let Some(last) = candidates.last_mut()
        && last.boundary == boundary
    {
        last.advance = last.advance.max(advance);
        return;
    }
    candidates.push(GapCandidate { boundary, advance });
}

fn technical_token_ranges(text: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut run_start = None;
    for (index, character) in text.char_indices() {
        let is_ascii_token_character = character.is_ascii() && !character.is_ascii_whitespace();
        if is_ascii_token_character && run_start.is_none() {
            run_start = Some(index);
        } else if !is_ascii_token_character && let Some(start) = run_start.take() {
            append_technical_range(text, start, index, &mut ranges);
        }
    }
    if let Some(start) = run_start {
        append_technical_range(text, start, text.len(), &mut ranges);
    }
    ranges
}

fn append_technical_range(text: &str, start: usize, end: usize, ranges: &mut Vec<Range<usize>>) {
    let mut token_end = end;
    while token_end > start {
        let character = text[..token_end]
            .chars()
            .next_back()
            .expect("token end must remain on a UTF-8 boundary");
        if !matches!(character, ',' | '.' | ':' | ';' | '!' | '?') {
            break;
        }
        token_end -= character.len_utf8();
    }
    if token_end > start && is_technical_token(&text[start..token_end]) {
        ranges.push(start..token_end);
    }
}

fn is_technical_token(token: &str) -> bool {
    if token.starts_with("https://") || token.starts_with("http://") || token.starts_with("ftp://")
    {
        return true;
    }
    if token.starts_with('/') && token[1..].contains('/') {
        return true;
    }
    if token.starts_with("./") || token.starts_with("../") {
        return true;
    }
    let bytes = token.as_bytes();
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\')
    {
        return true;
    }
    if token.matches('@').count() == 1 {
        let mut parts = token.split('@');
        let local = parts.next().unwrap_or_default();
        let domain = parts.next().unwrap_or_default();
        if !local.is_empty() && domain.contains('.') {
            return true;
        }
    }
    token.chars().any(|character| character.is_ascii_digit())
        && token.chars().all(|character| character.is_ascii_digit() || ".,:-%+".contains(character))
        && token.chars().any(|character| ".,:-%".contains(character))
}

fn normalize_ranges(text_length: usize, ranges: &[Range<usize>]) -> Vec<Range<usize>> {
    let mut ranges = ranges
        .iter()
        .filter_map(|range| {
            let start = range.start.min(text_length);
            let end = range.end.min(text_length);
            (start < end).then_some(start..end)
        })
        .collect::<Vec<_>>();
    if !ranges_are_ordered(&ranges) {
        ranges.sort_unstable_by(|left, right| {
            left.start.cmp(&right.start).then(left.end.cmp(&right.end))
        });
    }
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for range in ranges {
        if let Some(last) = merged.last_mut()
            && range.start <= last.end
        {
            last.end = last.end.max(range.end);
            continue;
        }
        merged.push(range);
    }
    merged
}

fn clusters_are_ordered(clusters: &[TypographyCluster]) -> bool {
    clusters.windows(2).all(|pair| pair[0].byte_range.start <= pair[1].byte_range.start)
}

fn ranges_are_ordered(ranges: &[Range<usize>]) -> bool {
    ranges.windows(2).all(|pair| {
        pair[0].start < pair[1].start
            || (pair[0].start == pair[1].start && pair[0].end <= pair[1].end)
    })
}

struct RangeCursor<'a> {
    ranges: &'a [Range<usize>],
    index: usize,
}

impl<'a> RangeCursor<'a> {
    fn new(ranges: &'a [Range<usize>]) -> Self {
        Self { ranges, index: 0 }
    }

    fn overlaps(&mut self, candidate: &Range<usize>) -> bool {
        while self.index < self.ranges.len() && self.ranges[self.index].end <= candidate.start {
            self.index += 1;
        }
        self.ranges.get(self.index).is_some_and(|range| range.start < candidate.end)
    }
}

fn merge_gap_candidates(first: Vec<GapCandidate>, second: Vec<GapCandidate>) -> Vec<GapCandidate> {
    let mut merged = Vec::with_capacity(first.len() + second.len());
    let mut first_index = 0;
    let mut second_index = 0;
    while first_index < first.len() || second_index < second.len() {
        let next_boundary = match (first.get(first_index), second.get(second_index)) {
            (Some(left), Some(right)) => left.boundary.min(right.boundary),
            (Some(left), None) => left.boundary,
            (None, Some(right)) => right.boundary,
            (None, None) => break,
        };
        let mut advance: f32 = 0.0;
        if first.get(first_index).is_some_and(|candidate| candidate.boundary == next_boundary) {
            advance = advance.max(first[first_index].advance);
            first_index += 1;
        }
        if second.get(second_index).is_some_and(|candidate| candidate.boundary == next_boundary) {
            advance = advance.max(second[second_index].advance);
            second_index += 1;
        }
        merged.push(GapCandidate { boundary: next_boundary, advance });
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clusters(text: &str, font_size: f32) -> Vec<TypographyCluster> {
        text.char_indices()
            .map(|(start, character)| {
                TypographyCluster::new(start..start + character.len_utf8(), font_size)
            })
            .collect()
    }

    fn gaps(text: &str) -> Vec<(usize, f32)> {
        let cluster_list = clusters(text, 16.0);
        let input = TypographyInput::new(text, TextSpacingMode::Natural, &cluster_list, &[]);
        calculate_gaps(&input)
            .into_iter()
            .map(|candidate| (candidate.boundary, candidate.advance))
            .collect()
    }

    #[test]
    fn natural_mode_adds_mixed_script_and_punctuation_gaps() {
        let text = "中文Word中文2026年你好,世界.接着说:可以;没问题!真的吗?";
        let actual = gaps(text);
        let expected_boundaries = [6, 10, 16, 20, 29, 30, 36, 37, 46, 47, 53, 54, 63, 64, 73];

        assert_eq!(
            actual.iter().map(|(boundary, _)| *boundary).collect::<Vec<_>>(),
            expected_boundaries
        );
        assert_eq!(actual[0].1, 2.0);
        assert_eq!(actual[4].1, 1.0);
        assert_eq!(actual[5].1, 2.0);
    }

    #[test]
    fn bracket_gaps_follow_open_and_close_sides() {
        let text = "正文(说明)继续正文[补充]继续";
        let actual = gaps(text);
        let expected = vec![
            (6, 2.0),
            (7, 1.0),
            (13, 1.0),
            (14, 2.0),
            (26, 2.0),
            (27, 1.0),
            (33, 1.0),
            (34, 2.0),
        ];

        assert_eq!(actual, expected);
    }

    #[test]
    fn punctuation_groups_have_no_internal_gap_and_ignore_english() {
        assert_eq!(gaps("真的吗?!继续"), vec![(9, 1.0), (11, 2.0)]);
        assert_eq!(gaps("等等...再说"), vec![(6, 1.0), (9, 2.0)]);
        assert!(gaps("Hello,world!").is_empty());
        assert!(gaps("你好, 世界").iter().all(|(boundary, _)| *boundary != 10));
    }

    #[test]
    fn punctuation_left_gap_requires_han_on_the_left() {
        assert_eq!(gaps("Word,中文"), vec![(5, 2.0)]);
    }

    #[test]
    fn punctuation_right_gap_applies_when_either_side_is_han() {
        assert_eq!(gaps("版本:beta"), vec![(6, 1.0), (7, 2.0)]);
        assert_eq!(gaps("中文,Word"), vec![(6, 1.0), (7, 2.0)]);
    }

    #[test]
    fn paired_quotes_are_bracket_like_but_unpaired_and_escaped_quotes_are_unchanged() {
        assert_eq!(gaps("正文\"说明\"继续"), vec![(6, 2.0), (7, 1.0), (13, 1.0), (14, 2.0)]);
        assert!(gaps("正文\"说明").is_empty());
        assert!(gaps("正文\\\"说明\\\"继续").is_empty());
    }

    #[test]
    fn subset_quote_roles_match_complete_source_context() {
        let text = "前文\"说明很长\"后文";
        let complete = clusters(text, 16.0);
        let all_gaps =
            calculate_gaps(&TypographyInput::new(text, TextSpacingMode::Natural, &complete, &[]));
        for start in 0..complete.len() - 1 {
            for end in start + 2..=complete.len() {
                let subset = &complete[start..end];
                let expected: Vec<_> = all_gaps
                    .iter()
                    .copied()
                    .filter(|gap| {
                        gap.boundary > subset[0].byte_range.start
                            && gap.boundary < subset[subset.len() - 1].byte_range.end
                    })
                    .collect();
                let actual = calculate_gaps(&TypographyInput::new(
                    text,
                    TextSpacingMode::Natural,
                    subset,
                    &[],
                ));
                assert_eq!(actual, expected, "subset {start}..{end} lost quote context");
            }
        }
    }

    #[test]
    fn protected_ranges_and_technical_tokens_preserve_internal_boundaries() {
        let text = "中文https://a.com/x?a=1邮箱a@b.com路径/usr/local数值3.14时间12:30日期2026-09-20比例50%结束";
        let cluster_list = clusters(text, 16.0);
        let input = TypographyInput::new(text, TextSpacingMode::Natural, &cluster_list, &[]);
        let actual = calculate_gaps(&input);

        for token in
            ["https://a.com/x?a=1", "a@b.com", "/usr/local", "3.14", "12:30", "2026-09-20", "50%"]
        {
            let start = text.find(token).expect("token should exist");
            let end = start + token.len();
            assert!(
                actual
                    .iter()
                    .all(|candidate| candidate.boundary <= start || candidate.boundary >= end)
            );
        }

        let protected_start = text.find("邮箱").expect("protected text should exist");
        let protected_end = protected_start + "邮箱".len();
        let protected = protected_start..protected_end;
        let protected_input = TypographyInput::new(
            text,
            TextSpacingMode::Natural,
            &cluster_list,
            std::slice::from_ref(&protected),
        );
        assert!(calculate_gaps(&protected_input).iter().all(|candidate| {
            candidate.boundary != protected_start && candidate.boundary != protected_end
        }));

        let mixed_token_gaps = gaps("版本3.14发布");
        assert_eq!(mixed_token_gaps, vec![(6, 2.0), (10, 2.0)]);
        let punctuation_after_token_gaps = gaps("版本3.14,发布");
        assert_eq!(punctuation_after_token_gaps, vec![(6, 2.0), (11, 2.0)]);
    }

    #[test]
    fn whitespace_verbatim_mode_and_cluster_boundaries_are_respected() {
        assert!(gaps("中文, 世界").iter().all(|(boundary, _)| *boundary != 7));

        let text = "中文e\u{301}后";
        let clusters = vec![
            TypographyCluster::new(0..6, 20.0),
            TypographyCluster::new(6..9, 12.0),
            TypographyCluster::new(9..12, 12.0),
        ];
        let natural_input = TypographyInput::new(text, TextSpacingMode::Natural, &clusters, &[]);
        let natural = calculate_gaps(&natural_input);
        assert_eq!(
            natural,
            vec![
                GapCandidate { boundary: 6, advance: 1.5 },
                GapCandidate { boundary: 9, advance: 1.5 },
            ]
        );

        let verbatim_input = TypographyInput::new(text, TextSpacingMode::Verbatim, &clusters, &[]);
        assert!(calculate_gaps(&verbatim_input).is_empty());
    }

    #[test]
    fn precomposed_latin_and_combining_latin_have_the_same_gap() {
        assert_eq!(gaps("中文é"), vec![(6, 2.0)]);
        assert_eq!(gaps("中文e\u{301}"), vec![(6, 2.0)]);
    }

    #[test]
    fn duplicate_rules_are_merged_by_maximum_gap() {
        let text = "中Word";
        let cluster_list = clusters(text, 16.0);
        let input = TypographyInput::new(text, TextSpacingMode::Natural, &cluster_list, &[]);

        assert_eq!(calculate_gaps(&input), vec![GapCandidate { boundary: 3, advance: 2.0 }]);
    }

    #[test]
    fn duplicate_glyph_ranges_are_aggregated_before_boundary_rules() {
        let text = "中Word";
        let clusters = vec![
            TypographyCluster::new(0..3, 16.0),
            TypographyCluster::new(3..4, 16.0),
            TypographyCluster::new(3..4, 16.0),
            TypographyCluster::new(4..8, 16.0),
        ];
        let input = TypographyInput::new(text, TextSpacingMode::Natural, &clusters, &[]);

        assert_eq!(calculate_gaps(&input), vec![GapCandidate { boundary: 3, advance: 2.0 }]);
    }
}
