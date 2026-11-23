pub mod indices;
pub use indices::{ByteIndex, GraphemeLineLookup, GraphemeOffset, LogicalPoint, VisualPoint};

/// The visual side of a source position shared by automatically wrapped rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum CursorAffinity {
    Upstream,
    #[default]
    Downstream,
}
