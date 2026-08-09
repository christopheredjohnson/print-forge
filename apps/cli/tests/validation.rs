use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

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

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn valid_document(elements: &str) -> String {
    format!(
        r##"{{
          "name": "Validation fixture",
          "document": {{
            "width": {{ "value": 100, "unit": "points" }},
            "height": {{ "value": 100, "unit": "points" }}
          }},
          "pages": [{{ "elements": [{elements}] }}]
        }}"##
    )
}

#[test]
fn malformed_templates_fail_before_validation() {
    let directory = TestDir::new("malformed");
    let template = directory.write("template.json", "{");
    let output = run(&["validate", template.to_str().unwrap()]);

    assert!(!output.status.success());
    assert!(stderr(&output).contains("failed to parse template"));
}

#[test]
fn empty_datasets_fail_before_creating_output() {
    let directory = TestDir::new("empty-dataset");
    let template = directory.write("template.json", &valid_document(""));
    let dataset = directory.write("dataset.json", "[]");
    let pdf = directory.path().join("output.pdf");
    let output = run(&[
        "render",
        template.to_str().unwrap(),
        dataset.to_str().unwrap(),
        pdf.to_str().unwrap(),
    ]);
    let errors = stderr(&output);

    assert!(!output.status.success());
    assert!(errors.contains("error[dataset.empty] rows"));
    assert!(errors.contains("validation failed with 1 error(s)"));
    assert!(!pdf.exists());
}

#[test]
fn validate_with_dataset_reports_required_field_paths() {
    let directory = TestDir::new("required-field");
    let template = directory.write(
        "template.json",
        r#"{
          "name": "Required field",
          "document": {
            "width": { "value": 100, "unit": "points" },
            "height": { "value": 100, "unit": "points" }
          },
          "fields": [
            { "name": "customer.name", "field_type": "text", "required": true }
          ],
          "pages": [{ "elements": [] }]
        }"#,
    );
    let dataset = directory.write("dataset.json", r#"[{"customer":{}}]"#);
    let output = run(&[
        "validate",
        template.to_str().unwrap(),
        "--dataset",
        dataset.to_str().unwrap(),
    ]);
    let errors = stderr(&output);

    assert!(!output.status.success());
    assert!(errors.contains("rows[0].customer.name"));
    assert!(errors.contains("required field 'customer.name' is missing"));
}

#[test]
fn missing_variables_report_row_page_and_element() {
    let directory = TestDir::new("missing-variable");
    let template = directory.write(
        "template.json",
        &valid_document(
            r##"{
              "type": "text",
              "position": {
                "x": { "value": 10, "unit": "points" },
                "y": { "value": 50, "unit": "points" },
                "width": { "value": 80, "unit": "points" },
                "height": { "value": 20, "unit": "points" }
              },
              "value": "{{missing}}",
              "font_size": { "value": 12, "unit": "points" }
            }"##,
        ),
    );
    let dataset = directory.write("dataset.json", "[{}]");
    let pdf = directory.path().join("output.pdf");
    let output = run(&[
        "render",
        template.to_str().unwrap(),
        dataset.to_str().unwrap(),
        pdf.to_str().unwrap(),
    ]);
    let errors = stderr(&output);

    assert!(!output.status.success());
    assert!(errors.contains("dataset row 0"));
    assert!(errors.contains("page 0, element pages[0].elements[0]"));
    assert!(errors.contains("template variable is missing: missing"));
    assert!(!pdf.exists());
}

#[test]
fn unsupported_elements_report_row_page_and_element() {
    let directory = TestDir::new("unsupported-element");
    let template = directory.write(
        "template.json",
        &valid_document(
            r##"{
              "type": "qr_code",
              "position": {
                "x": { "value": 10, "unit": "points" },
                "y": { "value": 10, "unit": "points" },
                "width": { "value": 40, "unit": "points" },
                "height": { "value": 40, "unit": "points" }
              },
              "value": "https://example.com"
            }"##,
        ),
    );
    let dataset = directory.write("dataset.json", "[{}]");
    let pdf = directory.path().join("output.pdf");
    let output = run(&[
        "render",
        template.to_str().unwrap(),
        dataset.to_str().unwrap(),
        pdf.to_str().unwrap(),
    ]);
    let errors = stderr(&output);

    assert!(!output.status.success());
    assert!(errors.contains("dataset row 0"));
    assert!(errors.contains("page 0, element pages[0].elements[0]"));
    assert!(errors.contains("not supported by this layout engine: qr_code"));
    assert!(!pdf.exists());
}
