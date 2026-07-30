use textora_markdown::layout::{LaidOutBlock, LaidOutBlockKind};
use ui::core::geom::Rect;

#[test]
fn laid_out_block_literal_keeps_its_public_field_shape() {
    let block = LaidOutBlock {
        kind: LaidOutBlockKind::HorizontalRule,
        rect: Rect::new(0.0, 0.0, 1.0, 1.0),
    };

    assert!(matches!(block.kind, LaidOutBlockKind::HorizontalRule));
}
