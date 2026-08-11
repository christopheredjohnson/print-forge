//! Semantic validation and preflight diagnostics.

use std::{collections::HashSet, fmt};

use print_forge_dataset::{DataRow, Dataset};
use print_forge_template::{
    Bounds, Color, DocumentMetadata, Element, Field, FieldType, FontFamily, Length, Page,
    StackDirection, StackElement, TableColumnWidth, TableElement, TableValueFormat, Template,
};
use serde::Serialize;

pub const SUPPORTED_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
}

impl fmt::Display for Severity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Error => formatter.write_str("error"),
            Self::Warning => formatter.write_str("warning"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: &'static str,
    pub path: String,
    pub message: String,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}[{}] {}: {}",
            self.severity, self.code, self.path, self.message
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValidationReport {
    diagnostics: Vec<Diagnostic>,
}

impl ValidationReport {
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn errors(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == Severity::Error)
    }

    pub fn warnings(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == Severity::Warning)
    }

    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.errors().next().is_none()
    }

    #[must_use]
    pub fn error_count(&self) -> usize {
        self.errors().count()
    }

    #[must_use]
    pub fn warning_count(&self) -> usize {
        self.warnings().count()
    }

    fn error(&mut self, code: &'static str, path: impl Into<String>, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic {
            severity: Severity::Error,
            code,
            path: path.into(),
            message: message.into(),
        });
    }

    fn warning(&mut self, code: &'static str, path: impl Into<String>, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            code,
            path: path.into(),
            message: message.into(),
        });
    }

    fn append(&mut self, mut other: Self) {
        self.diagnostics.append(&mut other.diagnostics);
    }

    /// Appends diagnostics from another report while preserving their order.
    pub fn extend(&mut self, other: Self) {
        self.append(other);
    }
}

#[must_use]
pub fn validate_template(template: &Template) -> ValidationReport {
    let mut report = ValidationReport::default();

    if template.schema_version != SUPPORTED_SCHEMA_VERSION {
        report.error(
            "schema.unsupported_version",
            "schema_version",
            format!(
                "schema version {} is not supported; this build supports version {}. \
                 Migrate the template to schema_version {} before rendering",
                template.schema_version, SUPPORTED_SCHEMA_VERSION, SUPPORTED_SCHEMA_VERSION
            ),
        );
    }

    if template.name.trim().is_empty() {
        report.error(
            "template.empty_name",
            "name",
            "template name cannot be empty",
        );
    }

    validate_positive_length(
        template.document.width,
        "document.width",
        "document width",
        &mut report,
    );
    validate_positive_length(
        template.document.height,
        "document.height",
        "document height",
        &mut report,
    );

    if let Some(bleed) = template.document.bleed {
        validate_nonnegative_length(bleed, "document.bleed", "document bleed", &mut report);
    }

    validate_fields(&template.fields, &mut report);
    validate_fonts(&template.fonts, &mut report);
    validate_metadata(&template.document.metadata, &mut report);

    if template.pages.is_empty() {
        report.error(
            "document.no_pages",
            "pages",
            "template must contain at least one page",
        );
    }

    let canvas = Canvas {
        width: template.document.width.to_points(),
        height: template.document.height.to_points(),
        bleed: template.document.bleed.map_or(0.0, Length::to_points),
    };

    for (page_index, page) in template.pages.iter().enumerate() {
        validate_page(page, page_index, canvas, &mut report);
    }

    report
}

#[must_use]
pub fn validate_job(template: &Template, dataset: &Dataset) -> ValidationReport {
    let mut report = validate_template(template);
    report.append(validate_dataset(template, dataset));
    report
}

#[must_use]
pub fn validate_dataset(template: &Template, dataset: &Dataset) -> ValidationReport {
    let mut report = ValidationReport::default();

    if dataset.rows.is_empty() {
        report.error(
            "dataset.empty",
            "rows",
            "dataset must contain at least one row",
        );
        return report;
    }

    for (row_index, row) in dataset.rows.iter().enumerate() {
        report.append(validate_data_row(template, row, row_index));
    }

    report
}

/// Validate one dataset row while retaining its original zero-based row index
/// in every diagnostic path.
#[must_use]
pub fn validate_data_row(template: &Template, row: &DataRow, row_index: usize) -> ValidationReport {
    let mut report = ValidationReport::default();
    for field in &template.fields {
        validate_row_field(row, row_index, field, &mut report);
    }
    for (page_index, page) in template.pages.iter().enumerate() {
        for (section, elements) in [
            ("header", &page.header),
            ("footer", &page.footer),
            ("elements", &page.elements),
        ] {
            for (element_index, element) in elements.iter().enumerate() {
                validate_table_data(
                    element,
                    &format!("pages[{page_index}].{section}[{element_index}]"),
                    row,
                    row_index,
                    &mut report,
                );
            }
        }
    }
    report
}

fn validate_table_data(
    element: &Element,
    template_path: &str,
    row: &DataRow,
    row_index: usize,
    report: &mut ValidationReport,
) {
    match element {
        Element::Table(table) => {
            let source_path = format!("rows[{row_index}].{}", table.source);
            let Some(value) = lookup_value(row, &table.source) else {
                report.error(
                    "table.missing_source",
                    source_path,
                    format!("{template_path} table source is missing"),
                );
                return;
            };
            let Some(rows) = value.as_array() else {
                report.error(
                    "table.invalid_source",
                    source_path,
                    format!("{template_path} table source must be an array"),
                );
                return;
            };

            for (table_row_index, table_row) in rows.iter().enumerate() {
                let table_row_path = format!("{source_path}[{table_row_index}]");
                let Some(table_row) = table_row.as_object() else {
                    report.error(
                        "table.invalid_row",
                        table_row_path,
                        "table rows must be objects",
                    );
                    continue;
                };
                for (column_index, column) in table.columns.iter().enumerate() {
                    let cell_path = format!("{table_row_path}.{}", column.field);
                    let Some(cell) = lookup_object_value(table_row, &column.field) else {
                        report.error(
                            "table.missing_cell",
                            cell_path,
                            format!(
                                "table column {column_index} field {:?} is missing",
                                column.field
                            ),
                        );
                        continue;
                    };
                    if cell.is_array() || cell.is_object() {
                        report.error(
                            "table.nested_cell",
                            cell_path,
                            "table cells must be scalar values; nested tables and arbitrary cell layouts are not supported",
                        );
                        continue;
                    }
                    if let Some(format) = &column.format {
                        validate_formatted_cell(cell, format, cell_path, report);
                    }
                }
            }
        }
        Element::Group(group) => {
            for (index, child) in group.children.iter().enumerate() {
                validate_table_data(
                    child,
                    &format!("{template_path}.children[{index}]"),
                    row,
                    row_index,
                    report,
                );
            }
        }
        Element::Stack(stack) => {
            for (index, child) in stack.children.iter().enumerate() {
                validate_table_data(
                    child,
                    &format!("{template_path}.children[{index}]"),
                    row,
                    row_index,
                    report,
                );
            }
        }
        Element::Repeater(repeater) => {
            let source_path = format!("rows[{row_index}].{}", repeater.source);
            let Some(value) = lookup_value(row, &repeater.source) else {
                report.error(
                    "repeater.missing_source",
                    source_path,
                    format!("repeater source {:?} is missing", repeater.source),
                );
                return;
            };
            let Some(items) = value.as_array() else {
                report.error(
                    "repeater.invalid_source",
                    source_path,
                    "repeater source must be an array",
                );
                return;
            };
            for (item_index, item) in items.iter().enumerate() {
                let scope = repeat_item_scope(row, item, item_index);
                validate_table_data(
                    &repeater.template,
                    &format!("{template_path}.items[{item_index}].template"),
                    &scope,
                    row_index,
                    report,
                );
            }
        }
        _ => {}
    }
}

fn validate_formatted_cell(
    value: &serde_json::Value,
    format: &TableValueFormat,
    path: String,
    report: &mut ValidationReport,
) {
    match format {
        TableValueFormat::Number { .. } | TableValueFormat::Currency { .. } => {
            let number = value
                .as_f64()
                .or_else(|| value.as_str().and_then(|value| value.parse::<f64>().ok()));
            if number.is_none_or(|number| !number.is_finite()) {
                report.error(
                    "table.invalid_number",
                    path,
                    "number and currency formats require a finite numeric value",
                );
            }
        }
        TableValueFormat::Date { .. } => {
            if value.as_str().and_then(parse_iso_date).is_none() {
                report.error(
                    "table.invalid_date",
                    path,
                    "date formats require a valid ISO date in YYYY-MM-DD form",
                );
            }
        }
    }
}

fn lookup_object_value<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    path: &str,
) -> Option<&'a serde_json::Value> {
    let mut segments = path.split('.');
    let mut value = object.get(segments.next()?)?;
    for segment in segments {
        value = value.get(segment)?;
    }
    Some(value)
}

fn parse_iso_date(value: &str) -> Option<(i32, u32, u32)> {
    let parts = value.split('-').collect::<Vec<_>>();
    let [year, month, day] = parts.as_slice() else {
        return None;
    };
    if year.len() != 4 || month.len() != 2 || day.len() != 2 {
        return None;
    }
    let year = year.parse::<i32>().ok()?;
    let month = month.parse::<u32>().ok()?;
    let day = day.parse::<u32>().ok()?;
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => return None,
    };
    (1..=max_day).contains(&day).then_some((year, month, day))
}

const fn is_leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn validate_fields(fields: &[Field], report: &mut ValidationReport) {
    let mut names = HashSet::new();

    for (index, field) in fields.iter().enumerate() {
        let path = format!("fields[{index}].name");
        let name = field.name.trim();

        if name.is_empty() {
            report.error("field.empty_name", path, "field name cannot be empty");
        } else if !names.insert(name) {
            report.error(
                "field.duplicate_name",
                path,
                format!("field name '{}' is declared more than once", field.name),
            );
        }
    }
}

fn validate_fonts(fonts: &[FontFamily], report: &mut ValidationReport) {
    let mut names = HashSet::new();
    for (index, font) in fonts.iter().enumerate() {
        let path = format!("fonts[{index}]");
        let name = font.name.trim();
        if name.is_empty() {
            report.error(
                "font.empty_name",
                format!("{path}.name"),
                "font family name cannot be empty",
            );
        } else if !names.insert(name) {
            report.error(
                "font.duplicate_name",
                format!("{path}.name"),
                format!(
                    "font family name {:?} is declared more than once",
                    font.name
                ),
            );
        }

        for (variant, source) in [
            ("regular", Some(font.regular.as_str())),
            ("bold", font.bold.as_deref()),
            ("italic", font.italic.as_deref()),
            ("bold_italic", font.bold_italic.as_deref()),
        ] {
            if source.is_some_and(|source| source.trim().is_empty()) {
                report.error(
                    "font.empty_source",
                    format!("{path}.{variant}"),
                    "font asset path cannot be empty",
                );
            }
        }
    }
}

fn validate_metadata(metadata: &DocumentMetadata, report: &mut ValidationReport) {
    for (field, value) in [
        ("title", metadata.title.as_deref()),
        ("author", metadata.author.as_deref()),
        ("subject", metadata.subject.as_deref()),
        ("identifier", metadata.identifier.as_deref()),
    ] {
        if value.is_some_and(|value| value.trim().is_empty()) {
            report.error(
                "metadata.empty_value",
                format!("document.metadata.{field}"),
                format!("metadata {field} cannot be empty when provided"),
            );
        }
    }

    for (index, keyword) in metadata.keywords.iter().enumerate() {
        if keyword.trim().is_empty() {
            report.error(
                "metadata.empty_keyword",
                format!("document.metadata.keywords[{index}]"),
                "metadata keywords cannot be empty",
            );
        }
    }
}

fn validate_color(value: &str, path: impl Into<String>, report: &mut ValidationReport) {
    if is_template_variable(value) {
        return;
    }
    if let Err(error) = value.parse::<Color>() {
        report.error("color.invalid", path, error.to_string());
    }
}

fn is_template_variable(value: &str) -> bool {
    let value = value.trim();
    let Some(variable) = value
        .strip_prefix("{{")
        .and_then(|value| value.strip_suffix("}}"))
    else {
        return false;
    };
    let variable = variable.trim();
    !variable.is_empty() && !variable.contains("{{") && !variable.contains("}}")
}

fn validate_page(page: &Page, page_index: usize, canvas: Canvas, report: &mut ValidationReport) {
    for (section, elements) in [("header", &page.header), ("footer", &page.footer)] {
        for (element_index, element) in elements.iter().enumerate() {
            let path = format!("pages[{page_index}].{section}[{element_index}]");
            validate_element(element, &path, canvas, PositionContext::Repeating, report);
        }
    }
    for (element_index, element) in page.elements.iter().enumerate() {
        let path = format!("pages[{page_index}].elements[{element_index}]");
        validate_element(element, &path, canvas, PositionContext::Absolute, report);
    }
}

fn validate_element(
    element: &Element,
    path: &str,
    canvas: Canvas,
    position_context: PositionContext,
    report: &mut ValidationReport,
) {
    if let Some(rotation) = element_rotation(element)
        && !rotation.is_finite()
    {
        report.error(
            "element.invalid_rotation",
            format!("{path}.rotation"),
            "rotation must be a finite number of clockwise degrees",
        );
    }
    match element {
        Element::Text(text) => {
            validate_optional_bounds(
                text.position.as_ref(),
                path,
                "text",
                canvas,
                position_context,
                report,
            );
            if position_context == PositionContext::Flow(StackDirection::Horizontal)
                && text.position.is_none()
            {
                report.error(
                    "flow.missing_size_hint",
                    format!("{path}.position"),
                    "horizontal flow text requires position width and height as size hints",
                );
            }
            validate_positive_length(
                text.font_size,
                format!("{path}.font_size"),
                "font size",
                report,
            );
            if let Some(line_height) = text.line_height {
                validate_positive_length(
                    line_height,
                    format!("{path}.line_height"),
                    "line height",
                    report,
                );
            }
            if let Some(min_font_size) = text.min_font_size {
                validate_positive_length(
                    min_font_size,
                    format!("{path}.min_font_size"),
                    "minimum font size",
                    report,
                );
                if min_font_size.to_points() > text.font_size.to_points() {
                    report.error(
                        "text.invalid_min_font_size",
                        format!("{path}.min_font_size"),
                        "minimum font size cannot exceed font size",
                    );
                }
            }
            validate_color(&text.color, format!("{path}.color"), report);
        }
        Element::Image(image) => {
            validate_optional_bounds(
                image.position.as_ref(),
                path,
                "image",
                canvas,
                position_context,
                report,
            );
            validate_required_flow_size_hint(
                image.position.as_ref(),
                path,
                "image",
                position_context,
                report,
            );
            if image.source.trim().is_empty() {
                report.error(
                    "image.empty_source",
                    format!("{path}.source"),
                    "image asset path cannot be empty",
                );
            }
        }
        Element::Rectangle(rectangle) => {
            validate_optional_bounds(
                rectangle.position.as_ref(),
                path,
                "rectangle",
                canvas,
                position_context,
                report,
            );
            validate_required_flow_size_hint(
                rectangle.position.as_ref(),
                path,
                "rectangle",
                position_context,
                report,
            );
            if let Some(stroke) = &rectangle.stroke {
                validate_positive_length(
                    stroke.width,
                    format!("{path}.stroke.width"),
                    "stroke width",
                    report,
                );
                validate_color(&stroke.color, format!("{path}.stroke.color"), report);
            }
            if let Some(fill) = &rectangle.fill {
                validate_color(fill, format!("{path}.fill"), report);
            }
        }
        Element::Line(line) => {
            if matches!(position_context, PositionContext::Flow(_)) {
                report.error(
                    "flow.unsupported_coordinates",
                    path,
                    "line elements use absolute coordinates and cannot be flow children",
                );
            }
            validate_finite_length(line.x1, format!("{path}.x1"), "line coordinate", report);
            validate_finite_length(line.y1, format!("{path}.y1"), "line coordinate", report);
            validate_finite_length(line.x2, format!("{path}.x2"), "line coordinate", report);
            validate_finite_length(line.y2, format!("{path}.y2"), "line coordinate", report);
            validate_positive_length(line.width, format!("{path}.width"), "line width", report);
            validate_color(&line.color, format!("{path}.color"), report);

            let start = (line.x1.to_points(), line.y1.to_points());
            let end = (line.x2.to_points(), line.y2.to_points());
            if coordinates_are_finite(&[start.0, start.1, end.0, end.1])
                && (!canvas.contains_point(start) || !canvas.contains_point(end))
            {
                report.warning(
                    "element.outside_bleed",
                    path,
                    "line extends outside the page bleed area and may be clipped",
                );
            }
        }
        Element::Svg(svg) => {
            validate_optional_bounds(
                svg.position.as_ref(),
                path,
                "svg",
                canvas,
                position_context,
                report,
            );
            validate_required_flow_size_hint(
                svg.position.as_ref(),
                path,
                "svg",
                position_context,
                report,
            );
            if svg.source.trim().is_empty() {
                report.error(
                    "svg.empty_source",
                    format!("{path}.source"),
                    "SVG asset path cannot be empty",
                );
            }
        }
        Element::QrCode(qr_code) => {
            validate_optional_bounds(
                qr_code.position.as_ref(),
                path,
                "qr_code",
                canvas,
                position_context,
                report,
            );
            validate_required_flow_size_hint(
                qr_code.position.as_ref(),
                path,
                "qr_code",
                position_context,
                report,
            );
            if qr_code.value.trim().is_empty() {
                report.error(
                    "qr_code.empty_value",
                    format!("{path}.value"),
                    "QR code value cannot be empty",
                );
            }
            if qr_code.quiet_zone < 4 {
                report.error(
                    "qr_code.quiet_zone",
                    format!("{path}.quiet_zone"),
                    "QR code quiet zone must be at least 4 modules",
                );
            }
            if let Some(position) = &qr_code.position
                && (position.width.to_points() - position.height.to_points()).abs() > 0.01
            {
                report.error(
                    "qr_code.not_square",
                    format!("{path}.position"),
                    "QR code bounds must be square",
                );
            }
            validate_color(&qr_code.color, format!("{path}.color"), report);
            validate_color(&qr_code.background, format!("{path}.background"), report);
        }
        Element::Barcode(barcode) => {
            validate_optional_bounds(
                barcode.position.as_ref(),
                path,
                "barcode",
                canvas,
                position_context,
                report,
            );
            validate_required_flow_size_hint(
                barcode.position.as_ref(),
                path,
                "barcode",
                position_context,
                report,
            );
            if barcode.value.trim().is_empty() {
                report.error(
                    "barcode.empty_value",
                    format!("{path}.value"),
                    "barcode value cannot be empty",
                );
            }
            if barcode.quiet_zone < 10 {
                report.error(
                    "barcode.quiet_zone",
                    format!("{path}.quiet_zone"),
                    "Code 128 quiet zone must be at least 10 modules",
                );
            }
            if let Some(position) = &barcode.position
                && position.height.to_points() + 0.01 < 14.4
            {
                report.error(
                    "barcode.too_short",
                    format!("{path}.position.height"),
                    "Code 128 bar height must be at least 14.4pt",
                );
            }
            validate_color(&barcode.color, format!("{path}.color"), report);
            validate_color(&barcode.background, format!("{path}.background"), report);
        }
        Element::Group(group) => {
            validate_required_flow_size_hint(
                group.position.as_ref(),
                path,
                "group",
                position_context,
                report,
            );
            if let Some(position) = &group.position {
                validate_bounds(position, &format!("{path}.position"), canvas, report);
            }
            let child_canvas = group.position.as_ref().map_or(canvas, |position| Canvas {
                width: position.width.to_points(),
                height: position.height.to_points(),
                bleed: 0.0,
            });
            for (index, child) in group.children.iter().enumerate() {
                validate_element(
                    child,
                    &format!("{path}.children[{index}]"),
                    child_canvas,
                    PositionContext::Absolute,
                    report,
                );
            }
        }
        Element::Stack(stack) => validate_stack(stack, path, canvas, position_context, report),
        Element::Table(table) => {
            validate_optional_bounds(
                table.position.as_ref(),
                path,
                "table",
                canvas,
                position_context,
                report,
            );
            validate_required_flow_size_hint(
                table.position.as_ref(),
                path,
                "table",
                position_context,
                report,
            );
            if position_context != PositionContext::Absolute {
                report.error(
                    "table.top_level_only",
                    path,
                    "MVP tables must be positioned top-level page elements",
                );
            }
            validate_table(table, path, report);
        }
        Element::Repeater(repeater) => {
            if repeater.source.trim().is_empty() {
                report.error(
                    "repeater.empty_source",
                    format!("{path}.source"),
                    "repeater source cannot be empty",
                );
            }
            if position_context != PositionContext::Absolute {
                report.error(
                    "repeater.top_level_only",
                    path,
                    "repeaters must be top-level page elements",
                );
            }
            if position_bounds(&repeater.template).is_none() {
                report.error(
                    "repeater.missing_item_bounds",
                    format!("{path}.template.position"),
                    "repeater template requires position bounds for its first item slot",
                );
            }
            validate_element(
                &repeater.template,
                &format!("{path}.template"),
                canvas,
                PositionContext::Absolute,
                report,
            );
        }
        Element::PageBreak => match position_context {
            PositionContext::Repeating => report.error(
                "flow.page_break_in_repeating_content",
                path,
                "page breaks are not allowed in repeating headers or footers",
            ),
            PositionContext::Flow(StackDirection::Horizontal) => report.error(
                "flow.page_break_in_horizontal_stack",
                path,
                "page breaks are not valid in horizontal stacks",
            ),
            PositionContext::Absolute | PositionContext::Flow(StackDirection::Vertical) => {}
        },
    }
}

fn element_rotation(element: &Element) -> Option<f32> {
    match element {
        Element::Text(element) => Some(element.rotation),
        Element::Image(element) => Some(element.rotation),
        Element::Rectangle(element) => Some(element.rotation),
        Element::Svg(element) => Some(element.rotation),
        Element::QrCode(element) => Some(element.rotation),
        Element::Barcode(element) => Some(element.rotation),
        Element::Line(_)
        | Element::Group(_)
        | Element::Stack(_)
        | Element::Table(_)
        | Element::Repeater(_)
        | Element::PageBreak => None,
    }
}

fn validate_stack(
    stack: &StackElement,
    path: &str,
    canvas: Canvas,
    position_context: PositionContext,
    report: &mut ValidationReport,
) {
    validate_optional_bounds(
        stack.position.as_ref(),
        path,
        "stack",
        canvas,
        position_context,
        report,
    );
    validate_nonnegative_length(stack.gap, format!("{path}.gap"), "stack gap", report);
    validate_nonnegative_length(
        stack.padding,
        format!("{path}.padding"),
        "stack padding",
        report,
    );
    if stack.orphans == 0 {
        report.error(
            "flow.invalid_orphans",
            format!("{path}.orphans"),
            "stack orphans must be at least 1",
        );
    }
    if stack.keep_together
        && stack
            .children
            .iter()
            .any(|element| matches!(element, Element::PageBreak))
    {
        report.error(
            "flow.keep_together_page_break",
            format!("{path}.children"),
            "a keep-together stack cannot contain an explicit page break",
        );
    }
    if let Some(position) = &stack.position {
        let padding = stack.padding.to_points();
        if padding.is_finite()
            && padding >= 0.0
            && (position.width.to_points() <= padding * 2.0
                || position.height.to_points() <= padding * 2.0)
        {
            report.error(
                "flow.padding_exceeds_bounds",
                format!("{path}.padding"),
                "stack padding must leave positive content width and height",
            );
        }
    }

    for (index, child) in stack.children.iter().enumerate() {
        validate_element(
            child,
            &format!("{path}.children[{index}]"),
            canvas,
            PositionContext::Flow(stack.direction),
            report,
        );
    }
}

fn validate_table(table: &TableElement, path: &str, report: &mut ValidationReport) {
    if table.source.trim().is_empty() {
        report.error(
            "table.empty_source",
            format!("{path}.source"),
            "table source cannot be empty",
        );
    }

    if table.columns.is_empty() {
        report.error(
            "table.no_columns",
            format!("{path}.columns"),
            "table must define at least one column",
        );
        return;
    }

    validate_positive_length(
        table.font_size,
        format!("{path}.font_size"),
        "table font size",
        report,
    );
    if let Some(line_height) = table.line_height {
        validate_positive_length(
            line_height,
            format!("{path}.line_height"),
            "table line height",
            report,
        );
    }
    validate_nonnegative_length(
        table.cell_padding,
        format!("{path}.cell_padding"),
        "table cell padding",
        report,
    );
    validate_color(&table.color, format!("{path}.color"), report);
    for (field, color) in [
        ("header_background", table.header_background.as_deref()),
        ("row_background", table.row_background.as_deref()),
        (
            "alternate_row_background",
            table.alternate_row_background.as_deref(),
        ),
    ] {
        if let Some(color) = color {
            validate_color(color, format!("{path}.{field}"), report);
        }
    }
    if let Some(border) = &table.border {
        validate_positive_length(
            border.width,
            format!("{path}.border.width"),
            "table border width",
            report,
        );
        validate_color(&border.color, format!("{path}.border.color"), report);
    }

    let mut resolved_width = 0.0_f32;
    let mut all_widths_valid = true;
    let table_width = table
        .position
        .as_ref()
        .map(|position| position.width.to_points());

    for (index, column) in table.columns.iter().enumerate() {
        let column_path = format!("{path}.columns[{index}]");

        if column.field.trim().is_empty() {
            report.error(
                "table.empty_column_field",
                format!("{column_path}.field"),
                "table column field cannot be empty",
            );
        }

        match column.width {
            TableColumnWidth::Fixed { value } => {
                let width = value.to_points();
                if !width.is_finite() || width <= 0.0 {
                    all_widths_valid = false;
                    report.error(
                        "table.invalid_column_width",
                        format!("{column_path}.width.value"),
                        "fixed table column width must be positive and finite",
                    );
                } else {
                    resolved_width += width;
                }
            }
            TableColumnWidth::Percent { value } => {
                if !value.is_finite() || value <= 0.0 || value > 100.0 {
                    all_widths_valid = false;
                    report.error(
                        "table.invalid_column_width",
                        format!("{column_path}.width.value"),
                        "percentage table column width must be greater than 0 and at most 100",
                    );
                } else if let Some(table_width) = table_width {
                    resolved_width += table_width * value / 100.0;
                }
            }
        }

        if let Some(format) = &column.format {
            match format {
                TableValueFormat::Number { decimals }
                | TableValueFormat::Currency { decimals, .. }
                    if *decimals > 12 =>
                {
                    report.error(
                        "table.invalid_format",
                        format!("{column_path}.format.decimals"),
                        "number and currency formats support at most 12 decimal places",
                    );
                }
                TableValueFormat::Currency { symbol, .. } if symbol.trim().is_empty() => {
                    report.error(
                        "table.invalid_format",
                        format!("{column_path}.format.symbol"),
                        "currency symbol cannot be empty",
                    );
                }
                _ => {}
            }
        }
    }

    if all_widths_valid
        && let Some(table_width) = table_width
        && table_width.is_finite()
        && table_width > 0.0
        && (resolved_width - table_width).abs() > 0.1
    {
        report.error(
            "table.invalid_total_width",
            format!("{path}.columns"),
            format!(
                "resolved table column widths must equal the {table_width:.2}pt table width; found {resolved_width:.2}pt"
            ),
        );
    }
}

fn validate_optional_bounds(
    bounds: Option<&Bounds>,
    path: &str,
    element_name: &str,
    canvas: Canvas,
    position_context: PositionContext,
    report: &mut ValidationReport,
) {
    match bounds {
        Some(bounds) if matches!(position_context, PositionContext::Flow(_)) => {
            validate_flow_bounds(bounds, &format!("{path}.position"), report);
        }
        Some(bounds) => validate_bounds(bounds, &format!("{path}.position"), canvas, report),
        None if matches!(
            position_context,
            PositionContext::Absolute | PositionContext::Repeating
        ) =>
        {
            report.error(
                "element.missing_position",
                format!("{path}.position"),
                format!("absolute-positioned {element_name} element requires position"),
            )
        }
        None => {}
    }
}

fn validate_required_flow_size_hint(
    bounds: Option<&Bounds>,
    path: &str,
    element_name: &str,
    position_context: PositionContext,
    report: &mut ValidationReport,
) {
    if matches!(position_context, PositionContext::Flow(_)) && bounds.is_none() {
        report.error(
            "flow.missing_size_hint",
            format!("{path}.position"),
            format!("flow {element_name} requires position width and height as size hints"),
        );
    }
}

fn validate_flow_bounds(bounds: &Bounds, path: &str, report: &mut ValidationReport) {
    validate_positive_length(
        bounds.width,
        format!("{path}.width"),
        "flow item width",
        report,
    );
    validate_positive_length(
        bounds.height,
        format!("{path}.height"),
        "flow item height",
        report,
    );
}

fn validate_bounds(bounds: &Bounds, path: &str, canvas: Canvas, report: &mut ValidationReport) {
    validate_finite_length(bounds.x, format!("{path}.x"), "x coordinate", report);
    validate_finite_length(bounds.y, format!("{path}.y"), "y coordinate", report);
    validate_positive_length(
        bounds.width,
        format!("{path}.width"),
        "element width",
        report,
    );
    validate_positive_length(
        bounds.height,
        format!("{path}.height"),
        "element height",
        report,
    );

    let values = [
        bounds.x.to_points(),
        bounds.y.to_points(),
        bounds.width.to_points(),
        bounds.height.to_points(),
    ];

    if coordinates_are_finite(&values)
        && values[2] > 0.0
        && values[3] > 0.0
        && !canvas.contains_bounds(values[0], values[1], values[2], values[3])
    {
        report.warning(
            "element.outside_bleed",
            path,
            "element extends outside the page bleed area and may be clipped",
        );
    }
}

fn validate_positive_length(
    length: Length,
    path: impl Into<String>,
    label: &str,
    report: &mut ValidationReport,
) {
    let value = length.to_points();
    if !value.is_finite() || value <= 0.0 {
        report.error(
            "value.not_positive",
            path,
            format!("{label} must be a positive finite value"),
        );
    }
}

fn validate_nonnegative_length(
    length: Length,
    path: impl Into<String>,
    label: &str,
    report: &mut ValidationReport,
) {
    let value = length.to_points();
    if !value.is_finite() || value < 0.0 {
        report.error(
            "value.negative",
            path,
            format!("{label} must be a nonnegative finite value"),
        );
    }
}

fn validate_finite_length(
    length: Length,
    path: impl Into<String>,
    label: &str,
    report: &mut ValidationReport,
) {
    if !length.to_points().is_finite() {
        report.error("value.not_finite", path, format!("{label} must be finite"));
    }
}

fn validate_row_field(
    row: &DataRow,
    row_index: usize,
    field: &Field,
    report: &mut ValidationReport,
) {
    let path = format!("rows[{row_index}].{}", field.name);
    let value = lookup_value(row, &field.name);

    let Some(value) = value else {
        if field.required {
            report.error(
                "dataset.missing_required_field",
                path,
                format!("required field '{}' is missing", field.name),
            );
        }
        return;
    };

    if value.is_null() || value.as_str().is_some_and(|value| value.trim().is_empty()) {
        if field.required {
            report.error(
                "dataset.empty_required_field",
                path,
                format!("required field '{}' cannot be empty", field.name),
            );
        }
        return;
    }

    if !value_matches_field_type(value, field.field_type) {
        report.error(
            "dataset.invalid_field_type",
            path,
            format!(
                "field '{}' does not match declared type {}",
                field.name,
                field_type_name(field.field_type)
            ),
        );
    }
}

fn lookup_value<'a>(row: &'a DataRow, path: &str) -> Option<&'a serde_json::Value> {
    let mut segments = path.split('.');
    let mut value = row.get(segments.next()?)?;

    for segment in segments {
        value = value.get(segment)?;
    }

    Some(value)
}

fn repeat_item_scope(data: &DataRow, item: &serde_json::Value, item_index: usize) -> DataRow {
    let root = data
        .get("root")
        .and_then(serde_json::Value::as_object)
        .cloned()
        .unwrap_or_else(|| data.clone());
    let mut scope = root.clone();
    if let Some(object) = item.as_object() {
        scope.extend(object.clone());
    }
    scope.insert("root".to_owned(), serde_json::Value::Object(root));
    scope.insert("item".to_owned(), item.clone());
    scope.insert(
        "index".to_owned(),
        serde_json::Value::Number(serde_json::Number::from(item_index + 1)),
    );
    scope
}

fn position_bounds(element: &Element) -> Option<&Bounds> {
    match element {
        Element::Text(element) => element.position.as_ref(),
        Element::Image(element) => element.position.as_ref(),
        Element::Rectangle(element) => element.position.as_ref(),
        Element::Svg(element) => element.position.as_ref(),
        Element::QrCode(element) => element.position.as_ref(),
        Element::Barcode(element) => element.position.as_ref(),
        Element::Group(element) => element.position.as_ref(),
        Element::Stack(element) => element.position.as_ref(),
        Element::Table(element) => element.position.as_ref(),
        Element::Line(_) | Element::Repeater(_) | Element::PageBreak => None,
    }
}

fn value_matches_field_type(value: &serde_json::Value, field_type: FieldType) -> bool {
    match field_type {
        FieldType::Text => value.is_string() || value.is_number() || value.is_boolean(),
        FieldType::Number => {
            value.is_number()
                || value.as_str().is_some_and(|value| {
                    value.parse::<f64>().is_ok_and(|number| number.is_finite())
                })
        }
        FieldType::Image => value.is_string(),
        FieldType::Boolean => {
            value.is_boolean()
                || value.as_str().is_some_and(|value| {
                    value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("false")
                })
        }
        FieldType::Collection => value.is_array(),
    }
}

const fn field_type_name(field_type: FieldType) -> &'static str {
    match field_type {
        FieldType::Text => "text",
        FieldType::Number => "number",
        FieldType::Image => "image",
        FieldType::Boolean => "boolean",
        FieldType::Collection => "collection",
    }
}

fn coordinates_are_finite(values: &[f32]) -> bool {
    values.iter().all(|value| value.is_finite())
}

#[derive(Debug, Clone, Copy)]
struct Canvas {
    width: f32,
    height: f32,
    bleed: f32,
}

impl Canvas {
    fn contains_point(self, point: (f32, f32)) -> bool {
        point.0 >= -self.bleed
            && point.1 >= -self.bleed
            && point.0 <= self.width + self.bleed
            && point.1 <= self.height + self.bleed
    }

    fn contains_bounds(self, x: f32, y: f32, width: f32, height: f32) -> bool {
        self.contains_point((x, y)) && self.contains_point((x + width, y + height))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PositionContext {
    Absolute,
    Repeating,
    Flow(StackDirection),
}
