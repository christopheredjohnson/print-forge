use print_forge_application::{
    AssetSource, DatasetSource, DiagnosticPhase, MemoryOutputWriter, OutputMode, RenderOptions,
    RenderRequest, render,
};
use print_forge_dataset::Dataset;
use print_forge_template::Template;
use serde_json::json;

fn template() -> Template {
    serde_json::from_value(json!({
        "name": "Public API fixture",
        "document": {
            "width": { "value": 100, "unit": "points" },
            "height": { "value": 100, "unit": "points" }
        },
        "pages": [{
            "elements": [{
                "type": "text",
                "position": {
                    "x": { "value": 5, "unit": "points" },
                    "y": { "value": 30, "unit": "points" },
                    "width": { "value": 90, "unit": "points" },
                    "height": { "value": 20, "unit": "points" }
                },
                "value": "{{name}}",
                "font_size": { "value": 12, "unit": "points" }
            }]
        }]
    }))
    .unwrap()
}

#[test]
fn callers_can_run_complete_jobs_in_process() {
    let template = template();
    let dataset = Dataset {
        rows: vec![serde_json::from_value(json!({ "name": "Ada" })).unwrap()],
    };
    let mut output = MemoryOutputWriter::default();
    let outcome = render(
        RenderRequest {
            template: &template,
            dataset: DatasetSource::InMemory(&dataset),
            assets: AssetSource::default(),
            options: RenderOptions {
                output_mode: OutputMode::Combined,
                ..RenderOptions::default()
            },
        },
        &mut output,
    )
    .unwrap();

    assert!(outcome.is_success());
    assert_eq!(outcome.summary.successes, 1);
    assert!(output.artifacts()["combined.pdf"].starts_with(b"%PDF"));
}

#[test]
fn public_diagnostics_keep_render_context() {
    let mut template = template();
    let print_forge_template::Element::Text(text) = &mut template.pages[0].elements[0] else {
        panic!("fixture element must be text");
    };
    text.value = "{{missing}}".to_owned();
    let dataset = Dataset {
        rows: vec![serde_json::from_value(json!({})).unwrap()],
    };
    let mut output = MemoryOutputWriter::default();
    let outcome = render(
        RenderRequest {
            template: &template,
            dataset: DatasetSource::InMemory(&dataset),
            assets: AssetSource::default(),
            options: RenderOptions::default(),
        },
        &mut output,
    )
    .unwrap();

    let diagnostic = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.phase == DiagnosticPhase::Layout)
        .unwrap();
    assert_eq!(diagnostic.row_index, Some(0));
    assert_eq!(diagnostic.page_index, Some(0));
    assert_eq!(
        diagnostic.element_path.as_deref(),
        Some("pages[0].elements[0]")
    );
}
