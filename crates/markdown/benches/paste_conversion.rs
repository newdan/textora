use criterion::{Criterion, black_box, criterion_group, criterion_main};
use textora_markdown::paste::{PasteRepresentations, PreparedPaste, prepare_paste};

const PARAGRAPH_COUNT: usize = 128;
const LIST_DEPTH: usize = 4;
const LIST_BREADTH: usize = 4;
const TABLE_ROWS: usize = 48;
const TABLE_COLUMNS: usize = 6;
const OFFICE_SPAN_COUNT: usize = 256;
const CODE_BLOCK_LINES: usize = 96;
const RTF_PARAGRAPH_COUNT: usize = 128;

fn paragraph_html(count: usize) -> (String, String) {
    let mut html = String::new();
    let mut plain_lines = Vec::with_capacity(count);
    for index in 0..count {
        let line = format!("Paragraph {index} emphasizes visible text.");
        html.push_str(&format!(
            "<p>Paragraph {index} <strong>emphasizes</strong> visible text.</p>"
        ));
        plain_lines.push(line);
    }
    (html, plain_lines.join("\n"))
}

fn nested_list_html(depth: usize, breadth: usize) -> (String, String) {
    let mut html = String::new();
    let mut plain_lines = Vec::new();
    append_list_level(&mut html, &mut plain_lines, depth, breadth, 0, 0);
    (html, plain_lines.join("\n"))
}

fn append_list_level(
    html: &mut String,
    plain_lines: &mut Vec<String>,
    remaining_depth: usize,
    breadth: usize,
    level: usize,
    parent_index: usize,
) {
    html.push_str("<ul>");
    for item_index in 0..breadth {
        let label = format!("level-{level}-parent-{parent_index}-item-{item_index}");
        html.push_str("<li><strong>");
        html.push_str(&label);
        html.push_str("</strong>");
        plain_lines.push(label);
        if remaining_depth > 1 {
            append_list_level(
                html,
                plain_lines,
                remaining_depth - 1,
                breadth,
                level + 1,
                item_index,
            );
        }
        html.push_str("</li>");
    }
    html.push_str("</ul>");
}

fn table_html(rows: usize, columns: usize) -> (String, String) {
    let mut html = String::from("<table><thead><tr>");
    let mut plain_lines = Vec::with_capacity(rows.saturating_add(1));
    let header = (0..columns).map(|column| format!("Header {column}")).collect::<Vec<_>>();
    for cell in &header {
        html.push_str("<th><strong>");
        html.push_str(cell);
        html.push_str("</strong></th>");
    }
    html.push_str("</tr></thead><tbody>");
    plain_lines.push(header.join("\t"));
    for row in 0..rows {
        let cells =
            (0..columns).map(|column| format!("row-{row}-column-{column}")).collect::<Vec<_>>();
        html.push_str("<tr>");
        for cell in &cells {
            html.push_str("<td><em>");
            html.push_str(cell);
            html.push_str("</em></td>");
        }
        html.push_str("</tr>");
        plain_lines.push(cells.join("\t"));
    }
    html.push_str("</tbody></table>");
    (html, plain_lines.join("\n"))
}

fn office_span_html(count: usize) -> (String, String) {
    let mut html = String::new();
    let mut plain_lines = Vec::with_capacity(count);
    for index in 0..count {
        let line = format!("Office segment {index} trailing text {index}");
        html.push_str(&format!(
            concat!(
                "<p class=\"MsoNormal\"><span style=\"font-weight:700\">",
                "Office segment {index}</span><span style=\"font-style:italic\">",
                "&nbsp;trailing text {index}</span></p>"
            ),
            index = index
        ));
        plain_lines.push(line);
    }
    (html, plain_lines.join("\n"))
}

fn code_block_html(lines: usize) -> (String, String) {
    let mut code_lines = Vec::with_capacity(lines.saturating_mul(2));
    for index in 0..lines {
        code_lines.push(format!("    let inline_{index} = ````;"));
        code_lines.push(format!("        println!(\"indented {index}\");"));
    }
    let plain = code_lines.join("\n");
    (format!("<pre><code class=\"language-rust\">{plain}</code></pre>"), plain)
}

fn rtf_paragraphs(count: usize) -> (Vec<u8>, String) {
    let mut rtf = String::from("{\\rtf1\\ansi ");
    let mut plain_lines = Vec::with_capacity(count);
    for index in 0..count {
        let line = format!("RTF paragraph {index} bold text");
        rtf.push_str(&format!("RTF paragraph {index} \\b bold\\b0 text"));
        if index + 1 < count {
            rtf.push_str("\\par ");
        }
        plain_lines.push(line);
    }
    rtf.push('}');
    (rtf.into_bytes(), plain_lines.join("\n"))
}

fn assert_html_fixture_is_convertible(html: &str, plain: &str) {
    let prepared = prepare_paste(PasteRepresentations {
        markdown: None,
        html: Some(html),
        rtf: None,
        plain: Some(plain),
        source_url: None,
    });
    assert!(!matches!(prepared, PreparedPaste::Empty), "HTML fixture must produce paste text");
}

fn assert_rtf_fixture_is_convertible(rtf: &[u8], plain: &str) {
    let prepared = prepare_paste(PasteRepresentations {
        markdown: None,
        html: None,
        rtf: Some(rtf),
        plain: Some(plain),
        source_url: None,
    });
    assert!(!matches!(prepared, PreparedPaste::Empty), "RTF fixture must produce paste text");
}

fn benchmark_paste_conversion(criterion: &mut Criterion) {
    let (paragraph_html, paragraph_plain) = paragraph_html(PARAGRAPH_COUNT);
    let (list_html, list_plain) = nested_list_html(LIST_DEPTH, LIST_BREADTH);
    let (table_html, table_plain) = table_html(TABLE_ROWS, TABLE_COLUMNS);
    let (office_html, office_plain) = office_span_html(OFFICE_SPAN_COUNT);
    let (code_html, code_plain) = code_block_html(CODE_BLOCK_LINES);
    let (rtf, rtf_plain) = rtf_paragraphs(RTF_PARAGRAPH_COUNT);

    assert_html_fixture_is_convertible(&paragraph_html, &paragraph_plain);
    assert_html_fixture_is_convertible(&list_html, &list_plain);
    assert_html_fixture_is_convertible(&table_html, &table_plain);
    assert_html_fixture_is_convertible(&office_html, &office_plain);
    assert_html_fixture_is_convertible(&code_html, &code_plain);
    assert_rtf_fixture_is_convertible(&rtf, &rtf_plain);

    let mut group = criterion.benchmark_group("paste_conversion");
    group.bench_function("paragraph_html", |bencher| {
        bencher.iter(|| {
            black_box(prepare_paste(PasteRepresentations {
                markdown: None,
                html: Some(black_box(paragraph_html.as_str())),
                rtf: None,
                plain: Some(black_box(paragraph_plain.as_str())),
                source_url: None,
            }))
        });
    });
    group.bench_function("nested_list_html", |bencher| {
        bencher.iter(|| {
            black_box(prepare_paste(PasteRepresentations {
                markdown: None,
                html: Some(black_box(list_html.as_str())),
                rtf: None,
                plain: Some(black_box(list_plain.as_str())),
                source_url: None,
            }))
        });
    });
    group.bench_function("table_html", |bencher| {
        bencher.iter(|| {
            black_box(prepare_paste(PasteRepresentations {
                markdown: None,
                html: Some(black_box(table_html.as_str())),
                rtf: None,
                plain: Some(black_box(table_plain.as_str())),
                source_url: None,
            }))
        });
    });
    group.bench_function("office_span_html", |bencher| {
        bencher.iter(|| {
            black_box(prepare_paste(PasteRepresentations {
                markdown: None,
                html: Some(black_box(office_html.as_str())),
                rtf: None,
                plain: Some(black_box(office_plain.as_str())),
                source_url: None,
            }))
        });
    });
    group.bench_function("code_block_html", |bencher| {
        bencher.iter(|| {
            black_box(prepare_paste(PasteRepresentations {
                markdown: None,
                html: Some(black_box(code_html.as_str())),
                rtf: None,
                plain: Some(black_box(code_plain.as_str())),
                source_url: None,
            }))
        });
    });
    group.bench_function("rtf_paragraphs", |bencher| {
        bencher.iter(|| {
            black_box(prepare_paste(PasteRepresentations {
                markdown: None,
                html: None,
                rtf: Some(black_box(rtf.as_slice())),
                plain: Some(black_box(rtf_plain.as_str())),
                source_url: None,
            }))
        });
    });
    group.finish();
}

criterion_group!(paste_conversion_benches, benchmark_paste_conversion);
criterion_main!(paste_conversion_benches);
