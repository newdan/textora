use criterion::{
    BatchSize, BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main,
};
use shaping::Shaper;
use std::cell::RefCell;
use textora_core::document::StringDocView;
use textora_markdown::augmenter::augment_edit;
use textora_markdown::builder::MarkdownDoc;
use textora_markdown::layout::layout_doc_for_rendering;
use textora_markdown::parser::parse_markdown;
use textora_markdown::style::MarkdownStyle;
use textora_markdown::view::MarkdownEditorView;
use ui::Rect;
use ui::core::DrawList;
use ui::plugin::{
    AugmentKind, EditIntent, EditPlan, EditRequest, EditSelection, EditTransaction, SyncReason,
    ViewPlugin,
};
use ui::plugin::{EditPolicy, EditableView};
use ui::theme::Theme;

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
    let document_view = textora_core::document::StringDocView::new(source);
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
        let document_view = textora_core::document::StringDocView::new(&source);
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

const VIEWPORT_HEIGHT: f32 = 600.0;
const REFLOWED_VIEWPORT_WIDTH: f32 = 540.0;
const DPI_SCALE: f32 = 1.0;
const INITIAL_GENERATION: u32 = 1;
const INSERTED_TEXT: &str = "中";
const TABLE_HEADER: &str = "| 名称 | 说明 |";

struct EditorFrame {
    view: MarkdownEditorView,
    source: String,
}

impl EditorFrame {
    fn prepare(source: &str, caret: usize, theme: &Theme, shaper: &mut Shaper) -> Self {
        let mut frame = Self { view: MarkdownEditorView::new(), source: source.to_owned() };
        frame.view.set_source(source.to_owned(), INITIAL_GENERATION);
        frame.set_caret(caret);
        black_box(frame.draw(VIEWPORT_WIDTH, theme, shaper));
        frame
    }

    fn set_caret(&mut self, caret: usize) {
        self.view.set_cursor_byte(caret);
    }

    fn draw(&mut self, width: f32, theme: &Theme, shaper: &mut Shaper) -> DrawList {
        let document = StringDocView::new(&self.source);
        let bounds = Rect::new(0.0, 0.0, width, VIEWPORT_HEIGHT);
        self.view.render(&document, bounds, theme, shaper, DPI_SCALE)
    }

    fn apply_edit(&mut self, request: &EditRequest) {
        let plan = self.view.plan_edit(request);
        let transaction = match (plan, &request.intent) {
            (EditPlan::Apply(transaction), _) => transaction,
            (EditPlan::UseDefault, EditIntent::InsertText(text)) => {
                let range =
                    request.selection.clone().unwrap_or(request.cursor_byte..request.cursor_byte);
                let caret_after = range.start + text.len();
                EditTransaction::replace(
                    request.source_generation,
                    range,
                    text.clone(),
                    caret_after,
                )
            }
            (plan, _) => panic!("editor benchmark expected a text replacement: {plan:?}"),
        };
        for replacement in transaction.replacements.into_iter().rev() {
            self.source.replace_range(replacement.range, &replacement.text);
        }
        let EditSelection::Caret(caret) = transaction.selection_after else {
            panic!("editor benchmark expected a caret after the replacement");
        };
        self.view.sync_document(
            transaction.source_generation.wrapping_add(1),
            SyncReason::Normal,
            &mut || self.source.clone(),
        );
        self.set_caret(caret);
    }
}

fn benchmark_editor_insert(
    criterion: &mut Criterion,
    group_name: &str,
    sizes_kib: &[usize],
    source_for_size: fn(usize) -> String,
    insertion_caret: fn(&str) -> usize,
) {
    let mut group = criterion.benchmark_group(group_name);
    let theme = ui::theme::test_theme();
    let shaper = RefCell::new(Shaper::new().expect("benchmark font shaper must initialize"));
    for &size_kib in sizes_kib {
        let source = source_for_size(size_kib * 1024);
        let caret = insertion_caret(&source);
        let request = EditRequest {
            source_generation: INITIAL_GENERATION,
            cursor_byte: caret,
            selection: None,
            intent: EditIntent::InsertText(INSERTED_TEXT.into()),
        };
        group.throughput(Throughput::Bytes(source.len() as u64));
        group.bench_function(BenchmarkId::new("insert", size_kib), |bencher| {
            bencher.iter_batched_ref(
                || EditorFrame::prepare(&source, caret, &theme, &mut shaper.borrow_mut()),
                |frame| {
                    frame.apply_edit(&request);
                    frame.draw(VIEWPORT_WIDTH, &theme, &mut shaper.borrow_mut())
                },
                BatchSize::PerIteration,
            );
        });
        group.bench_function(BenchmarkId::new("reflow", size_kib), |bencher| {
            bencher.iter_batched_ref(
                || EditorFrame::prepare(&source, caret, &theme, &mut shaper.borrow_mut()),
                |frame| frame.draw(REFLOWED_VIEWPORT_WIDTH, &theme, &mut shaper.borrow_mut()),
                BatchSize::PerIteration,
            );
        });
    }
    group.finish();
}

fn benchmark_editor_table_enter(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("markdown_editor_mixed");
    let theme = ui::theme::test_theme();
    let shaper = RefCell::new(Shaper::new().expect("benchmark font shaper must initialize"));
    for &size_kib in MIXED_DOCUMENT_SIZES_KIB {
        let source = format!(
            "{TABLE_HEADER}

{}",
            mixed_document(size_kib * 1024)
        );
        let caret = TABLE_HEADER.len();
        let request = EditRequest {
            source_generation: INITIAL_GENERATION,
            cursor_byte: caret,
            selection: None,
            intent: EditIntent::InsertParagraphBreak,
        };
        group.throughput(Throughput::Bytes(source.len() as u64));
        group.bench_function(BenchmarkId::new("table_enter", size_kib), |bencher| {
            bencher.iter_batched_ref(
                || EditorFrame::prepare(&source, caret, &theme, &mut shaper.borrow_mut()),
                |frame| {
                    frame.apply_edit(&request);
                    frame.draw(VIEWPORT_WIDTH, &theme, &mut shaper.borrow_mut())
                },
                BatchSize::PerIteration,
            );
        });
    }
    group.finish();
}

fn benchmark_editor_selection_replacement(criterion: &mut Criterion) {
    const SELECTED_PARAGRAPH: &str = "普通段落";
    const REPLACEMENT_TEXT: &str = " 新";

    let mut group = criterion.benchmark_group("markdown_editor_mixed");
    let theme = ui::theme::test_theme();
    let shaper = RefCell::new(Shaper::new().expect("benchmark font shaper must initialize"));
    for &size_kib in MIXED_DOCUMENT_SIZES_KIB {
        let source = format!("{SELECTED_PARAGRAPH}\n\n{}", mixed_document(size_kib * 1024));
        let caret = SELECTED_PARAGRAPH.len();
        let request = EditRequest {
            source_generation: INITIAL_GENERATION,
            cursor_byte: caret,
            selection: Some(0..caret),
            intent: EditIntent::InsertText(REPLACEMENT_TEXT.into()),
        };
        group.throughput(Throughput::Bytes(source.len() as u64));
        group.bench_function(BenchmarkId::new("replace_selection", size_kib), |bencher| {
            bencher.iter_batched_ref(
                || EditorFrame::prepare(&source, caret, &theme, &mut shaper.borrow_mut()),
                |frame| {
                    frame.apply_edit(&request);
                    frame.draw(VIEWPORT_WIDTH, &theme, &mut shaper.borrow_mut())
                },
                BatchSize::PerIteration,
            );
        });
    }
    group.finish();
}

fn benchmark_editor_view(criterion: &mut Criterion) {
    benchmark_editor_insert(
        criterion,
        "markdown_editor_mixed",
        MIXED_DOCUMENT_SIZES_KIB,
        mixed_document,
        |source| {
            let paragraph = "这是包含";
            let (start, _) = source
                .match_indices(paragraph)
                .min_by_key(|(start, _)| start.abs_diff(source.len() / 2))
                .expect("mixed benchmark fixtures contain a body paragraph");
            start + paragraph.len()
        },
    );
    benchmark_editor_insert(
        criterion,
        "markdown_editor_long_cjk",
        LONG_PARAGRAPH_SIZES_KIB,
        long_cjk_paragraph,
        |source| floor_char_boundary(source, source.len() / 2),
    );
    benchmark_editor_table_enter(criterion);
    benchmark_editor_selection_replacement(criterion);
}

criterion_group!(
    editing_benches,
    benchmark_mixed_document_editing,
    benchmark_long_paragraph_layout,
    benchmark_editor_view
);
criterion_main!(editing_benches);
