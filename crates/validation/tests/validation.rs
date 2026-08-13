use print_forge_dataset::Dataset;
use print_forge_template::Template;
use print_forge_validation::{Severity, validate_job, validate_template};

fn template(json: &str) -> Template {
    serde_json::from_str(json).unwrap()
}

#[test]
fn accepts_the_business_card_fixture() {
    let template: Template = serde_json::from_str(include_str!(
        "../../../examples/business-card/template.json"
    ))
    .unwrap();
    let dataset = Dataset::from_csv_reader(
        include_bytes!("../../../examples/business-card/data.csv").as_slice(),
    )
    .unwrap();

    let report = validate_job(&template, &dataset);

    assert!(report.is_valid(), "{:#?}", report.diagnostics());
    assert_eq!(report.warning_count(), 0);
}

#[test]
fn rejects_the_invalid_csv_fixture_with_exact_row_paths() {
    let template: Template = serde_json::from_str(include_str!(
        "../../../examples/business-card/template.json"
    ))
    .unwrap();
    let dataset = Dataset::from_csv_reader(
        include_bytes!("../../../examples/business-card/invalid-data.csv").as_slice(),
    )
    .unwrap();

    let report = validate_job(&template, &dataset);
    let paths: Vec<_> = report
        .errors()
        .map(|diagnostic| diagnostic.path.as_str())
        .collect();

    assert_eq!(report.error_count(), 3);
    assert!(paths.contains(&"rows[0].title"));
    assert!(paths.contains(&"rows[1].last_name"));
    assert!(paths.contains(&"rows[1].title"));
}

#[test]
fn rejects_incompatible_schema_and_invalid_document() {
    let template = template(
        r#"{
          "schema_version": 2,
          "name": "Broken",
          "document": {
            "width": { "value": 0, "unit": "points" },
            "height": { "value": 100, "unit": "points" }
          },
          "pages": []
        }"#,
    );

    let report = validate_template(&template);
    let codes: Vec<_> = report.errors().map(|diagnostic| diagnostic.code).collect();

    assert!(codes.contains(&"schema.unsupported_version"));
    assert!(codes.contains(&"value.not_positive"));
    assert!(codes.contains(&"document.no_pages"));
    assert!(
        report
            .errors()
            .find(|diagnostic| diagnostic.code == "schema.unsupported_version")
            .unwrap()
            .message
            .contains("Migrate the template")
    );
}

#[test]
fn rejects_invalid_element_dimensions_and_table_widths() {
    let template = template(
        r##"{
          "name": "Invalid elements",
          "document": {
            "width": { "value": 100, "unit": "points" },
            "height": { "value": 100, "unit": "points" }
          },
          "pages": [{
            "elements": [
              {
                "type": "text",
                "position": {
                  "x": { "value": 10, "unit": "points" },
                  "y": { "value": 10, "unit": "points" },
                  "width": { "value": 0, "unit": "points" },
                  "height": { "value": 20, "unit": "points" }
                },
                "value": "invalid",
                "font_size": { "value": 0, "unit": "points" }
              },
              {
                "type": "rectangle",
                "position": {
                  "x": { "value": 10, "unit": "points" },
                  "y": { "value": 10, "unit": "points" },
                  "width": { "value": 20, "unit": "points" },
                  "height": { "value": 20, "unit": "points" }
                },
                "stroke": {
                  "width": { "value": -1, "unit": "points" }
                }
              },
              {
                "type": "table",
                "position": {
                  "x": { "value": 10, "unit": "points" },
                  "y": { "value": 10, "unit": "points" },
                  "width": { "value": 80, "unit": "points" },
                  "height": { "value": 80, "unit": "points" }
                },
                "source": "items",
                "columns": [
                  { "field": "name", "header": "Name", "width": { "type": "percent", "value": 40 } },
                  { "field": "price", "header": "Price", "width": { "type": "percent", "value": 40 } }
                ]
              }
            ]
          }]
        }"##,
    );

    let report = validate_template(&template);
    let paths: Vec<_> = report
        .errors()
        .map(|diagnostic| diagnostic.path.as_str())
        .collect();

    assert!(paths.contains(&"pages[0].elements[0].position.width"));
    assert!(paths.contains(&"pages[0].elements[0].font_size"));
    assert!(paths.contains(&"pages[0].elements[1].stroke.width"));
    assert!(paths.contains(&"pages[0].elements[2].columns"));
}

#[test]
fn rejects_non_finite_element_rotation() {
    let mut invalid: Template = serde_json::from_str(include_str!(
        "../../../examples/business-card/template.json"
    ))
    .unwrap();
    let print_forge_template::Element::Text(text) = &mut invalid.pages[0].elements[0] else {
        panic!("fixture should start with text");
    };
    text.rotation = f32::NAN;

    let report = validate_template(&invalid);
    let diagnostic = report
        .errors()
        .find(|diagnostic| diagnostic.code == "element.invalid_rotation")
        .unwrap();

    assert_eq!(diagnostic.path, "pages[0].elements[0].rotation");
}

#[test]
fn reports_empty_datasets_and_missing_required_fields_with_row_paths() {
    let template = template(
        r#"{
          "name": "Required fields",
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

    let empty_report = validate_job(&template, &Dataset::default());
    assert!(
        empty_report
            .errors()
            .any(|diagnostic| diagnostic.code == "dataset.empty")
    );

    let dataset = Dataset::from_json_reader(r#"[{"customer":{}}]"#.as_bytes()).unwrap();
    let report = validate_job(&template, &dataset);
    let diagnostic = report
        .errors()
        .find(|diagnostic| diagnostic.code == "dataset.missing_required_field")
        .unwrap();

    assert_eq!(diagnostic.path, "rows[0].customer.name");
}

#[test]
fn warns_when_an_element_exceeds_the_page_and_bleed() {
    let template = template(
        r##"{
          "name": "Clipped",
          "document": {
            "width": { "value": 100, "unit": "points" },
            "height": { "value": 100, "unit": "points" },
            "bleed": { "value": 5, "unit": "points" }
          },
          "pages": [{
            "elements": [{
              "type": "text",
              "position": {
                "x": { "value": 90, "unit": "points" },
                "y": { "value": 90, "unit": "points" },
                "width": { "value": 20, "unit": "points" },
                "height": { "value": 20, "unit": "points" }
              },
              "value": "clipped",
              "font_size": { "value": 10, "unit": "points" }
            }]
          }]
        }"##,
    );

    let report = validate_template(&template);
    let diagnostic = report
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.code == "element.outside_bleed")
        .unwrap();

    assert_eq!(diagnostic.severity, Severity::Warning);
    assert_eq!(diagnostic.path, "pages[0].elements[0].position");
}

#[test]
fn rejects_invalid_font_and_text_layout_settings() {
    let template = template(
        r#"{
          "name": "Invalid typography",
          "document": {
            "width": { "value": 100, "unit": "points" },
            "height": { "value": 100, "unit": "points" }
          },
          "fonts": [
            { "name": "Fixture", "regular": "" },
            { "name": "Fixture", "regular": "font.ttf", "italic_face_index": 2 }
          ],
          "pages": [{
            "elements": [{
              "type": "text",
              "position": {
                "x": { "value": 10, "unit": "points" },
                "y": { "value": 10, "unit": "points" },
                "width": { "value": 80, "unit": "points" },
                "height": { "value": 20, "unit": "points" }
              },
              "value": "text",
              "font_size": { "value": 10, "unit": "points" },
              "line_height": { "value": 0, "unit": "points" },
              "min_font_size": { "value": 12, "unit": "points" }
            }]
          }]
        }"#,
    );

    let report = validate_template(&template);
    let codes: Vec<_> = report.errors().map(|diagnostic| diagnostic.code).collect();

    assert!(codes.contains(&"font.empty_source"));
    assert!(codes.contains(&"font.duplicate_name"));
    assert!(codes.contains(&"font.orphan_face_index"));
    assert!(codes.contains(&"value.not_positive"));
    assert!(codes.contains(&"text.invalid_min_font_size"));
}

#[test]
fn rejects_invalid_print_colors_and_empty_metadata() {
    let template = template(
        r##"{
          "name": "Invalid print settings",
          "document": {
            "width": { "value": 100, "unit": "points" },
            "height": { "value": 100, "unit": "points" },
            "metadata": { "author": "", "keywords": [""] }
          },
          "pages": [{
            "elements": [{
              "type": "rectangle",
              "position": {
                "x": { "value": 10, "unit": "points" },
                "y": { "value": 10, "unit": "points" },
                "width": { "value": 20, "unit": "points" },
                "height": { "value": 20, "unit": "points" }
              },
              "fill": "cmyk(0, 0%, 0%, 0%)",
              "stroke": {
                "width": { "value": 1, "unit": "points" },
                "color": "rgb(300, 0, 0)"
              }
            }]
          }]
        }"##,
    );

    let report = validate_template(&template);
    let paths: Vec<_> = report
        .errors()
        .map(|diagnostic| diagnostic.path.as_str())
        .collect();

    assert!(paths.contains(&"document.metadata.author"));
    assert!(paths.contains(&"document.metadata.keywords[0]"));
    assert!(paths.contains(&"pages[0].elements[0].fill"));
    assert!(paths.contains(&"pages[0].elements[0].stroke.color"));
}

#[test]
fn accepts_color_template_variables() {
    let template = template(
        r##"{
          "name": "Theme variables",
          "document": {
            "width": { "value": 100, "unit": "points" },
            "height": { "value": 100, "unit": "points" }
          },
          "pages": [{
            "elements": [{
              "type": "rectangle",
              "position": {
                "x": { "value": 10, "unit": "points" },
                "y": { "value": 10, "unit": "points" },
                "width": { "value": 20, "unit": "points" },
                "height": { "value": 20, "unit": "points" }
              },
              "fill": "{{theme_primary}}",
              "stroke": {
                "width": { "value": 1, "unit": "points" },
                "color": "{{ theme_accent }}"
              }
            }]
          }]
        }"##,
    );

    let report = validate_template(&template);

    assert!(report.is_valid(), "{:#?}", report.diagnostics());
}

#[test]
fn rejects_invalid_flow_layout_contracts() {
    let template = template(
        r##"{
          "name": "Invalid flow",
          "document": {
            "width": { "value": 100, "unit": "points" },
            "height": { "value": 100, "unit": "points" }
          },
          "pages": [{
            "header": [{ "type": "page_break" }],
            "elements": [{
              "type": "stack",
              "direction": "horizontal",
              "position": {
                "x": { "value": 10, "unit": "points" },
                "y": { "value": 10, "unit": "points" },
                "width": { "value": 80, "unit": "points" },
                "height": { "value": 40, "unit": "points" }
              },
              "padding": { "value": 20, "unit": "points" },
              "orphans": 0,
              "keep_together": true,
              "children": [
                {
                  "type": "text",
                  "value": "missing horizontal size",
                  "font_size": { "value": 10, "unit": "points" }
                },
                { "type": "page_break" }
              ]
            }]
          }]
        }"##,
    );

    let report = validate_template(&template);
    let codes = report
        .errors()
        .map(|diagnostic| diagnostic.code)
        .collect::<Vec<_>>();

    assert!(codes.contains(&"flow.page_break_in_repeating_content"));
    assert!(codes.contains(&"flow.padding_exceeds_bounds"));
    assert!(codes.contains(&"flow.invalid_orphans"));
    assert!(codes.contains(&"flow.keep_together_page_break"));
    assert!(codes.contains(&"flow.missing_size_hint"));
    assert!(codes.contains(&"flow.page_break_in_horizontal_stack"));
}

#[test]
fn validates_specialty_element_scan_constraints() {
    let valid_template: Template = serde_json::from_str(include_str!(
        "../../../examples/specialty-elements/template.json"
    ))
    .unwrap();
    let dataset = Dataset::from_json_reader(
        include_bytes!("../../../examples/specialty-elements/data.json").as_slice(),
    )
    .unwrap();
    assert!(validate_job(&valid_template, &dataset).is_valid());

    let invalid = template(
        r##"{
          "name": "Unsafe codes",
          "document": {
            "width": { "value": 200, "unit": "points" },
            "height": { "value": 200, "unit": "points" }
          },
          "pages": [{ "elements": [
            {
              "type": "qr_code",
              "position": {
                "x": { "value": 10, "unit": "points" },
                "y": { "value": 100, "unit": "points" },
                "width": { "value": 80, "unit": "points" },
                "height": { "value": 70, "unit": "points" }
              },
              "value": "value",
              "quiet_zone": 2,
              "color": "red"
            },
            {
              "type": "barcode",
              "position": {
                "x": { "value": 10, "unit": "points" },
                "y": { "value": 20, "unit": "points" },
                "width": { "value": 180, "unit": "points" },
                "height": { "value": 10, "unit": "points" }
              },
              "value": "PF-1",
              "quiet_zone": 4
            }
          ] }]
        }"##,
    );
    let report = validate_template(&invalid);
    let codes = report
        .errors()
        .map(|diagnostic| diagnostic.code)
        .collect::<Vec<_>>();

    assert!(codes.contains(&"qr_code.quiet_zone"));
    assert!(codes.contains(&"qr_code.not_square"));
    assert!(codes.contains(&"color.invalid"));
    assert!(codes.contains(&"barcode.quiet_zone"));
    assert!(codes.contains(&"barcode.too_short"));
}

#[test]
fn validates_repeater_sources_and_nested_item_table_data() {
    let template = template(
        r##"{
          "name": "Repeated items",
          "document": {
            "width": { "value": 200, "unit": "points" },
            "height": { "value": 200, "unit": "points" }
          },
          "pages": [{ "elements": [{
            "type": "repeater",
            "source": "catalog.sections",
            "layout": "vertical",
            "template": {
              "type": "group",
              "position": {
                "x": { "value": 10, "unit": "points" },
                "y": { "value": 110, "unit": "points" },
                "width": { "value": 180, "unit": "points" },
                "height": { "value": 80, "unit": "points" }
              },
              "children": [{
                "type": "text",
                "position": {
                  "x": { "value": 5, "unit": "points" },
                  "y": { "value": 5, "unit": "points" },
                  "width": { "value": 170, "unit": "points" },
                  "height": { "value": 20, "unit": "points" }
                },
                "value": "{{name}} / {{root.company}} / {{index}}",
                "font_size": { "value": 10, "unit": "points" }
              }]
            }
          }] }]
        }"##,
    );
    let valid = Dataset::from_json_reader(
        r#"[{"company":"Forge","catalog":{"sections":[{"name":"One"}]}}]"#.as_bytes(),
    )
    .unwrap();
    assert!(validate_job(&template, &valid).is_valid());

    let invalid = Dataset::from_json_reader(
        r#"[{"company":"Forge","catalog":{"sections":{"name":"One"}}}]"#.as_bytes(),
    )
    .unwrap();
    let report = validate_job(&template, &invalid);
    let diagnostic = report
        .errors()
        .find(|diagnostic| diagnostic.code == "repeater.invalid_source")
        .unwrap();
    assert_eq!(diagnostic.path, "rows[0].catalog.sections");
}

#[test]
fn accepts_mixed_width_tables_and_scalar_formatted_cells() {
    let template = template(
        r##"{
          "name": "Valid table",
          "document": {
            "width": { "value": 120, "unit": "points" },
            "height": { "value": 160, "unit": "points" }
          },
          "fields": [
            { "name": "items", "field_type": "collection", "required": true }
          ],
          "pages": [{ "elements": [{
            "type": "table",
            "position": {
              "x": { "value": 10, "unit": "points" },
              "y": { "value": 10, "unit": "points" },
              "width": { "value": 100, "unit": "points" },
              "height": { "value": 140, "unit": "points" }
            },
            "source": "items",
            "header": true,
            "header_background": "#EEEEEE",
            "alternate_row_background": "rgb(248, 248, 248)",
            "border": {
              "width": { "value": 0.5, "unit": "points" },
              "color": "#333333"
            },
            "columns": [
              {
                "field": "description",
                "header": "Description",
                "width": {
                  "type": "fixed",
                  "value": { "value": 40, "unit": "points" }
                }
              },
              {
                "field": "amount",
                "header": "Amount",
                "width": { "type": "percent", "value": 60 },
                "align": "right",
                "format": { "type": "currency", "symbol": "$", "decimals": 2 }
              }
            ]
          }] }]
        }"##,
    );
    let dataset = Dataset::from_json_reader(
        r#"[{"items":[{"description":"Paper","amount":1234.5}]}]"#.as_bytes(),
    )
    .unwrap();

    let report = validate_job(&template, &dataset);
    assert!(report.is_valid(), "{:#?}", report.diagnostics());
}

#[test]
fn rejects_invalid_table_formats_and_nested_cell_content() {
    let template = template(
        r##"{
          "name": "Invalid table data",
          "document": {
            "width": { "value": 120, "unit": "points" },
            "height": { "value": 160, "unit": "points" }
          },
          "pages": [{ "elements": [{
            "type": "table",
            "position": {
              "x": { "value": 10, "unit": "points" },
              "y": { "value": 10, "unit": "points" },
              "width": { "value": 100, "unit": "points" },
              "height": { "value": 140, "unit": "points" }
            },
            "source": "items",
            "columns": [
              {
                "field": "amount",
                "header": "Amount",
                "width": { "type": "percent", "value": 30 },
                "format": { "type": "currency", "symbol": "", "decimals": 13 }
              },
              {
                "field": "date",
                "header": "Date",
                "width": { "type": "percent", "value": 30 },
                "format": { "type": "date", "style": "us" }
              },
              {
                "field": "details",
                "header": "Details",
                "width": { "type": "percent", "value": 40 }
              }
            ]
          }] }]
        }"##,
    );
    let dataset = Dataset::from_json_reader(
        r#"[{"items":[{"amount":"not-a-number","date":"2026-02-30","details":{"nested":true}}]}]"#
            .as_bytes(),
    )
    .unwrap();

    let report = validate_job(&template, &dataset);
    let codes = report
        .errors()
        .map(|diagnostic| diagnostic.code)
        .collect::<Vec<_>>();
    assert!(codes.contains(&"table.invalid_format"));
    assert!(codes.contains(&"table.invalid_number"));
    assert!(codes.contains(&"table.invalid_date"));
    assert!(codes.contains(&"table.nested_cell"));
}
