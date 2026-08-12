use print_forge_dataset::DataRow;
use print_forge_engine::{LayoutEngine, LayoutError, LayoutOptions, ResolvedDocument};
use print_forge_template::Template;

struct CustomEngine;

impl LayoutEngine for CustomEngine {
    fn layout(
        &self,
        _template: &Template,
        _data: &DataRow,
    ) -> Result<ResolvedDocument, LayoutError> {
        Err(LayoutError::Document {
            message: "custom engine invoked".to_owned(),
        })
    }
}

#[test]
fn custom_engines_work_behind_the_public_trait_boundary() {
    let engine: &dyn LayoutEngine = &CustomEngine;
    let template: Template = serde_json::from_str(include_str!(
        "../../../examples/business-card/template.json"
    ))
    .unwrap();
    let error = engine
        .layout_with_options(&template, &DataRow::new(), &LayoutOptions::default())
        .unwrap_err();

    assert!(error.to_string().contains("custom engine invoked"));
}
