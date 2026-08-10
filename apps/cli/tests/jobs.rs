use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

use printpdf::{PdfDocument, PdfParseOptions};
use serde_json::Value;

struct TestDir(PathBuf);

impl TestDir {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("print-forge-{name}-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn write(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.path().join(name);
        fs::write(&path, contents).unwrap();
        path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_print-forge"))
        .args(arguments)
        .output()
        .unwrap()
}

fn page_count(path: &Path) -> usize {
    let bytes = fs::read(path).unwrap();
    PdfDocument::parse(&bytes, &PdfParseOptions::default(), &mut Vec::new())
        .unwrap()
        .pages
        .len()
}

fn job_template() -> &'static str {
    r##"{
      "name": "Variable data fixture",
      "document": {
        "width": { "value": 100, "unit": "points" },
        "height": { "value": 100, "unit": "points" }
      },
      "fields": [
        { "name": "name", "field_type": "text", "required": true }
      ],
      "pages": [{
        "elements": [{
          "type": "text",
          "position": {
            "x": { "value": 10, "unit": "points" },
            "y": { "value": 40, "unit": "points" },
            "width": { "value": 80, "unit": "points" },
            "height": { "value": 20, "unit": "points" }
          },
          "value": "{{name}}",
          "font_size": { "value": 12, "unit": "points" }
        }]
      }]
    }"##
}

#[test]
fn combined_mode_renders_every_dataset_row() {
    let directory = TestDir::new("combined-job");
    let template = directory.write("template.json", job_template());
    let dataset = directory.write(
        "dataset.json",
        r#"[
          {"name":"One"},
          {"name":"Two"},
          {"name":"Three"},
          {"name":"Four"}
        ]"#,
    );
    let pdf = directory.path().join("combined.pdf");
    let output = run(&[
        "render",
        template.to_str().unwrap(),
        dataset.to_str().unwrap(),
        pdf.to_str().unwrap(),
    ]);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(page_count(&pdf), 4);
    assert!(String::from_utf8_lossy(&output.stdout).contains("rendered 4 dataset row(s)"));
}

#[test]
fn separate_mode_uses_safe_collision_free_field_names_and_writes_summary() {
    let directory = TestDir::new("separate-job");
    let template = directory.write("template.json", job_template());
    let dataset = directory.write(
        "dataset.json",
        r#"[
          {"name":"One","invoice_number":"ACME / 7"},
          {"name":"Two","invoice_number":"ACME / 7"}
        ]"#,
    );
    let output_directory = directory.path().join("pdfs");
    fs::create_dir_all(&output_directory).unwrap();
    fs::write(output_directory.join("ACME-7.pdf"), b"existing").unwrap();
    let summary_path = directory.path().join("summary.json");
    let output = run(&[
        "render",
        template.to_str().unwrap(),
        dataset.to_str().unwrap(),
        output_directory.to_str().unwrap(),
        "--output-mode",
        "separate",
        "--output-name",
        "{{invoice_number}}",
        "--summary",
        summary_path.to_str().unwrap(),
    ]);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(page_count(&output_directory.join("ACME-7-2.pdf")), 1);
    assert_eq!(page_count(&output_directory.join("ACME-7-3.pdf")), 1);

    let summary: Value = serde_json::from_slice(&fs::read(summary_path).unwrap()).unwrap();
    assert_eq!(summary["status"], "succeeded");
    assert_eq!(summary["successes"], 2);
    assert_eq!(summary["failures"], 0);
    assert_eq!(summary["outputs"].as_array().unwrap().len(), 2);
    assert!(summary["elapsed_ms"].is_u64());
}

#[test]
fn continue_on_error_produces_partial_combined_output_and_failure_summary() {
    let directory = TestDir::new("partial-job");
    let template = directory.write("template.json", job_template());
    let dataset = directory.write("dataset.json", r#"[{"name":"Ada"},{},{"name":"Grace"}]"#);
    let pdf = directory.path().join("partial.pdf");
    let summary_path = directory.path().join("summary.json");
    let output = run(&[
        "render",
        template.to_str().unwrap(),
        dataset.to_str().unwrap(),
        pdf.to_str().unwrap(),
        "--continue-on-error",
        "--summary",
        summary_path.to_str().unwrap(),
    ]);

    assert!(!output.status.success());
    assert_eq!(page_count(&pdf), 2);
    assert!(String::from_utf8_lossy(&output.stderr).contains("dataset row 1 failed"));

    let summary: Value = serde_json::from_slice(&fs::read(summary_path).unwrap()).unwrap();
    assert_eq!(summary["status"], "partial");
    assert_eq!(summary["selected_rows"], 3);
    assert_eq!(summary["successes"], 2);
    assert_eq!(summary["failures"], 1);
    assert!(
        summary["records"]
            .as_array()
            .unwrap()
            .iter()
            .any(|record| { record["row_index"] == 1 && record["status"] == "failed" })
    );
}

#[test]
fn print_ready_mode_generates_valid_pdf_x_with_bleed_and_trim_boxes() {
    let directory = TestDir::new("print-ready-job");
    let examples = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let template = examples.join("business-card.json");
    let dataset = examples.join("people.csv");
    let pdf = directory.path().join("print-ready.pdf");
    let output = run(&[
        "render",
        template.to_str().unwrap(),
        dataset.to_str().unwrap(),
        pdf.to_str().unwrap(),
        "--print-ready",
    ]);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let parsed = lopdf::Document::load(&pdf).unwrap();
    let page = parsed.get_dictionary(parsed.get_pages()[&1]).unwrap();

    assert_eq!(parsed.version, "1.6");
    assert!(parsed.catalog().unwrap().has(b"OutputIntents"));
    assert!(page.has(b"MediaBox"));
    assert!(page.has(b"BleedBox"));
    assert!(page.has(b"TrimBox"));
}

#[test]
fn flow_layout_fixture_renders_explicit_and_automatic_continuation_pages() {
    let directory = TestDir::new("flow-layout-job");
    let examples = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let template = examples.join("flow-layout.json");
    let dataset = examples.join("flow-layout-data.json");
    let pdf = directory.path().join("flow-layout.pdf");
    let output = run(&[
        "render",
        template.to_str().unwrap(),
        dataset.to_str().unwrap(),
        pdf.to_str().unwrap(),
    ]);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(page_count(&pdf), 3);
    assert!(String::from_utf8_lossy(&output.stdout).contains("3 page(s)"));
}

#[test]
fn mvp_table_fixture_wraps_rows_and_paginates() {
    let directory = TestDir::new("table-job");
    let examples = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let template = examples.join("table-invoice.json");
    let dataset = examples.join("table-invoice-data.json");
    let pdf = directory.path().join("table-invoice.pdf");
    let output = run(&[
        "render",
        template.to_str().unwrap(),
        dataset.to_str().unwrap(),
        pdf.to_str().unwrap(),
    ]);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(page_count(&pdf), 2);
    assert!(String::from_utf8_lossy(&output.stdout).contains("2 page(s)"));
}

#[test]
fn composition_fixtures_render_repeated_labels_and_catalog_pages() {
    let directory = TestDir::new("composition-job");
    let examples = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    for (template_name, dataset_name, output_name, expected_pages) in [
        (
            "label-sheet.json",
            "label-sheet-data.json",
            "label-sheet.pdf",
            1,
        ),
        (
            "product-catalog.json",
            "product-catalog-data.json",
            "product-catalog.pdf",
            2,
        ),
    ] {
        let template = examples.join(template_name);
        let dataset = examples.join(dataset_name);
        let pdf = directory.path().join(output_name);
        let output = run(&[
            "render",
            template.to_str().unwrap(),
            dataset.to_str().unwrap(),
            pdf.to_str().unwrap(),
        ]);

        assert!(
            output.status.success(),
            "{}: {}",
            template_name,
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(page_count(&pdf), expected_pages, "{template_name}");
    }
}
