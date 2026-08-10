use print_forge_engine::ResolvedDocument;
use print_forge_pdf::{DocumentRenderer, PdfResult};

struct CustomRenderer;

impl DocumentRenderer for CustomRenderer {
    fn render(&self, _document: &ResolvedDocument) -> PdfResult<Vec<u8>> {
        Ok(b"custom".to_vec())
    }
}

#[test]
fn custom_renderers_work_behind_the_public_trait_boundary() {
    fn render(renderer: &dyn DocumentRenderer, document: &ResolvedDocument) -> PdfResult<Vec<u8>> {
        renderer.render(document)
    }

    let document = ResolvedDocument {
        title: "API fixture".to_owned(),
        width_pt: 72.0,
        height_pt: 72.0,
        bleed_pt: 0.0,
        metadata: Default::default(),
        pages: Vec::new(),
    };
    assert_eq!(render(&CustomRenderer, &document).unwrap(), b"custom");
}
