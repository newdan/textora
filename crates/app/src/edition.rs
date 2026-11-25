#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edition {
    Standard,
    Lite,
}

impl Edition {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Standard => "textora",
            Self::Lite => "textora_lite",
        }
    }

    pub(crate) fn supports_markdown_views(self) -> bool {
        cfg!(feature = "markdown") && self == Self::Standard
    }
}
