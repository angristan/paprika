#[derive(Debug)]
pub(super) struct SemanticPage {
    pub(super) number: usize,
    pub(super) title: String,
    pub(super) html: String,
    pub(super) images: Vec<PageImage>,
    pub(super) has_text: bool,
}

#[derive(Debug)]
pub(super) struct PageImage {
    pub(super) href: String,
    pub(super) bytes: Vec<u8>,
    pub(super) alt: String,
    pub(super) placement: ImagePlacement,
}

#[derive(Debug)]
pub(super) enum ImagePlacement {
    Caption(String),
    EquationAnchor(String),
    VisualPageFallback,
    EndOfPage,
}
