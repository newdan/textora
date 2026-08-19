use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use textora_markdown::augmenter::augment_edit;
use textora_markdown::builder::MarkdownDoc;
use textora_markdown::layout::layout_doc_for_rendering;
use textora_markdown::parser::parse_markdown;
use textora_markdown::style::MarkdownStyle;
use ui::plugin::AugmentKind;

const VIEWPORT_WIDTH: f32 = 720.0;
const MIXED_DOCUMENT_SIZES_KIB: &[usize] = &[5, 22, 87, 218];
const LONG_PARAGRAPH_SIZES_KIB: &[usize] = &[10, 40, 82, 163, 326];

fn benchmark_style() -> MarkdownStyle {
    MarkdownStyle::from_theme(&ui::theme::test_theme(), 15.0, 24.0)
}

fn floor_char_boundary(source: &str, mut byte: usize) -> usize {
    byte = byte.min(source.len());
    while !source.is_char_boundary(byte) {
        byte -= 1;
    }
    byte
}

fn mixed_document(target_bytes: usize) -> String {
    const BLOCK_PATTERN: &str = concat!(
        "## 性能章节\n\n",
        "这是包含 **粗体**、*斜体*、`inline code` 与 emoji 👩‍💻 的正文。\n\n",
        "- 第一项\n",
        "- 第二项\n\n",
        "> 引用段落用于覆盖投影折叠。\n\n",
        "```rust\n",
        "fn main() { println!(\"textora\"); }\n",
        "```\n\n",
    );
    let mut source = String::with_capacity(target_bytes + BLOCK_PATTERN.len());
    while source.len() < target_bytes {
        source.push_str(BLOCK_PATTERN);
    }
    source.truncate(floor_char_boundary(&source, target_bytes));
    source
}

fn long_cjk_paragraph(target_bytes: usize) -> String {
    const SENTENCE: &str = "长段落用于验证折行投影复杂度保持线性，并覆盖中文写作场景。";
    let mut source = String::with_capacity(target_bytes + SENTENCE.len());
    while source.len() < target_bytes {
        source.push_str(SENTENCE);
    }
    source.truncate(floor_char_boundary(&source, target_bytes));
    source
}

fn run_editing_pipeline(source: &str, cursor_byte: usize, style: &MarkdownStyle) -> f32 {
    black_box(augment_edit(source, cursor_byte, AugmentKind::InsertText(String::from("中"))));
    let parsed = parse_markdown(source);
    let document = MarkdownDoc::build(&parsed, style);
    let document_view = core::document::StringDocView::new(source);
    layout_doc_for_rendering(&document.blocks, style, VIEWPORT_WIDTH, &document_view)
        .document()
        .total_height
}

fn benchmark_mixed_document_editing(criterion: &mut Criterion) {
    let style = benchmark_style();
    let mut group = criterion.benchmark_group("mixed_document_single_key");
    for &size_kib in MIXED_DOCUMENT_SIZES_KIB {
        let source = mixed_document(size_kib * 1024);
        let cursor_byte = floor_char_boundary(&source, source.len() / 2);
        group.throughput(Throughput::Bytes(source.len() as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(size_kib),
            &source,
            |bencher, source| {
                bencher.iter(|| {
                    black_box(run_editing_pipeline(
                        black_box(source),
                        cursor_byte,
                        black_box(&style),
                    ))
                });
            },
        );
    }
    group.finish();
}

fn benchmark_long_paragraph_layout(criterion: &mut Criterion) {
    let style = benchmark_style();
    let mut group = criterion.benchmark_group("long_cjk_paragraph_layout");
    for &size_kib in LONG_PARAGRAPH_SIZES_KIB {
        let source = long_cjk_paragraph(size_kib * 1024);
        let parsed = parse_markdown(&source);
        let document = MarkdownDoc::build(&parsed, &style);
        let document_view = core::document::StringDocView::new(&source);
        group.throughput(Throughput::Bytes(source.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size_kib), &size_kib, |bencher, _| {
            bencher.iter(|| {
                black_box(
                    layout_doc_for_rendering(
                        black_box(&document.blocks),
                        black_box(&style),
                        VIEWPORT_WIDTH,
                        black_box(&document_view),
                    )
                    .document()
                    .total_height,
                )
            });
        });
    }
    group.finish();
}

criterion_group!(
    editing_benches,
    benchmark_mixed_document_editing,
    benchmark_long_paragraph_layout
);
criterion_main!(editing_benches);
