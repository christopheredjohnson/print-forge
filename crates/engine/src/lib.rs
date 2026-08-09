//! Layout contracts shared by template compilers and document renderers.

use print_forge_dataset::DataRow;
use print_forge_template::{DashStyle, Element, Stroke, Template, TextAlign};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedDocument {
    pub title: String,
    pub width_pt: f32,
    pub height_pt: f32,
    pub pages: Vec<ResolvedPage>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResolvedPage {
    pub commands: Vec<DrawCommand>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DrawCommand {
    Text(TextCommand),
    Image(ImageCommand),
    Rectangle(RectangleCommand),
    Line(LineCommand),
    Svg(SvgCommand),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextCommand {
    pub bounds: Rect,
    pub value: String,
    pub font_size_pt: f32,
    pub font: Option<String>,
    pub color: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImageCommand {
    pub bounds: Rect,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RectangleCommand {
    pub bounds: Rect,
    pub fill: Option<String>,
    pub stroke: Option<StrokeCommand>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LineCommand {
    pub start: Point,
    pub end: Point,
    pub width_pt: f32,
    pub color: String,
    pub dash: LineDash,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SvgCommand {
    pub bounds: Rect,
    pub source: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StrokeCommand {
    pub width_pt: f32,
    pub color: String,
    pub dash: LineDash,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum LineDash {
    #[default]
    Solid,
    Dashed,
    Dotted,
}

pub trait LayoutEngine {
    fn layout(&self, template: &Template, data: &DataRow) -> Result<ResolvedDocument, LayoutError>;
}

/// Absolute-position layout for the first renderer vertical slice.
///
/// Flow elements remain explicit errors so unsupported template content is
/// never silently omitted from output.
#[derive(Debug, Default, Clone, Copy)]
pub struct BasicLayoutEngine;

impl LayoutEngine for BasicLayoutEngine {
    fn layout(&self, template: &Template, data: &DataRow) -> Result<ResolvedDocument, LayoutError> {
        let width_pt = template.document.width.to_points();
        let height_pt = template.document.height.to_points();

        if width_pt <= 0.0 || height_pt <= 0.0 {
            return Err(LayoutError::InvalidLayout(
                "document dimensions must be positive".to_owned(),
            ));
        }

        let pages = template
            .pages
            .iter()
            .map(|page| {
                let commands = page
                    .elements
                    .iter()
                    .map(|element| layout_element(element, data))
                    .collect::<Result<Vec<_>, _>>()?;

                Ok(ResolvedPage { commands })
            })
            .collect::<Result<Vec<_>, LayoutError>>()?;

        Ok(ResolvedDocument {
            title: template.name.clone(),
            width_pt,
            height_pt,
            pages,
        })
    }
}

fn layout_element(element: &Element, data: &DataRow) -> Result<DrawCommand, LayoutError> {
    match element {
        Element::Text(text) => {
            if text.align != TextAlign::Left {
                return Err(LayoutError::UnsupportedFeature(
                    "centered, right-aligned, and justified text".to_owned(),
                ));
            }

            let bounds = text
                .position
                .as_ref()
                .ok_or_else(|| missing_position("text"))?;

            Ok(DrawCommand::Text(TextCommand {
                bounds: resolve_bounds(bounds),
                value: resolve_string(&text.value, data)?,
                font_size_pt: text.font_size.to_points(),
                font: text.font.clone(),
                color: text.color.clone(),
            }))
        }
        Element::Image(image) => {
            let bounds = image
                .position
                .as_ref()
                .ok_or_else(|| missing_position("image"))?;

            Ok(DrawCommand::Image(ImageCommand {
                bounds: resolve_bounds(bounds),
                source: resolve_string(&image.source, data)?,
            }))
        }
        Element::Rectangle(rectangle) => {
            let bounds = rectangle
                .position
                .as_ref()
                .ok_or_else(|| missing_position("rectangle"))?;

            Ok(DrawCommand::Rectangle(RectangleCommand {
                bounds: resolve_bounds(bounds),
                fill: rectangle.fill.clone(),
                stroke: rectangle.stroke.as_ref().map(resolve_stroke),
            }))
        }
        Element::Line(line) => Ok(DrawCommand::Line(LineCommand {
            start: Point {
                x: line.x1.to_points(),
                y: line.y1.to_points(),
            },
            end: Point {
                x: line.x2.to_points(),
                y: line.y2.to_points(),
            },
            width_pt: line.width.to_points(),
            color: line.color.clone(),
            dash: resolve_dash(line.dash),
        })),
        Element::Svg(svg) => {
            let bounds = svg
                .position
                .as_ref()
                .ok_or_else(|| missing_position("svg"))?;

            Ok(DrawCommand::Svg(SvgCommand {
                bounds: resolve_bounds(bounds),
                source: resolve_string(&svg.source, data)?,
            }))
        }
        Element::QrCode(_) => Err(LayoutError::UnsupportedElement("qr_code")),
        Element::Group(_) => Err(LayoutError::UnsupportedElement("group")),
        Element::Stack(_) => Err(LayoutError::UnsupportedElement("stack")),
        Element::Table(_) => Err(LayoutError::UnsupportedElement("table")),
        Element::Repeater(_) => Err(LayoutError::UnsupportedElement("repeater")),
        Element::PageBreak => Err(LayoutError::UnsupportedElement("page_break")),
    }
}

fn resolve_bounds(bounds: &print_forge_template::Bounds) -> Rect {
    Rect {
        x: bounds.x.to_points(),
        y: bounds.y.to_points(),
        width: bounds.width.to_points(),
        height: bounds.height.to_points(),
    }
}

fn resolve_stroke(stroke: &Stroke) -> StrokeCommand {
    StrokeCommand {
        width_pt: stroke.width.to_points(),
        color: stroke.color.clone(),
        dash: resolve_dash(stroke.dash),
    }
}

const fn resolve_dash(dash: DashStyle) -> LineDash {
    match dash {
        DashStyle::Solid => LineDash::Solid,
        DashStyle::Dashed => LineDash::Dashed,
        DashStyle::Dotted => LineDash::Dotted,
    }
}

fn missing_position(element: &'static str) -> LayoutError {
    LayoutError::InvalidLayout(format!(
        "absolute-positioned {element} element is missing position"
    ))
}

fn resolve_string(input: &str, data: &DataRow) -> Result<String, LayoutError> {
    let mut output = String::with_capacity(input.len());
    let mut remaining = input;

    while let Some(start) = remaining.find("{{") {
        output.push_str(&remaining[..start]);
        let expression = &remaining[start + 2..];
        let end = expression
            .find("}}")
            .ok_or_else(|| LayoutError::InvalidTemplate("unclosed template variable".to_owned()))?;
        let key = expression[..end].trim();

        if key.is_empty() {
            return Err(LayoutError::InvalidTemplate(
                "template variable name cannot be empty".to_owned(),
            ));
        }

        let value =
            lookup_value(data, key).ok_or_else(|| LayoutError::MissingVariable(key.to_owned()))?;

        if let Some(value) = value.as_str() {
            output.push_str(value);
        } else {
            output.push_str(&value.to_string());
        }

        remaining = &expression[end + 2..];
    }

    if remaining.contains("}}") {
        return Err(LayoutError::InvalidTemplate(
            "template contains an unmatched closing delimiter".to_owned(),
        ));
    }

    output.push_str(remaining);
    Ok(output)
}

fn lookup_value<'a>(data: &'a DataRow, path: &str) -> Option<&'a serde_json::Value> {
    let mut segments = path.split('.');
    let mut value = data.get(segments.next()?)?;

    for segment in segments {
        value = value.get(segment)?;
    }

    Some(value)
}

#[derive(Debug, Error)]
pub enum LayoutError {
    #[error("template element is not supported by this layout engine: {0}")]
    UnsupportedElement(&'static str),
    #[error("template variable is missing: {0}")]
    MissingVariable(String),
    #[error("template is invalid: {0}")]
    InvalidTemplate(String),
    #[error("layout failed: {0}")]
    InvalidLayout(String),
    #[error("layout feature is not implemented: {0}")]
    UnsupportedFeature(String),
}

#[cfg(test)]
mod tests {
    use print_forge_dataset::DataRow;
    use print_forge_template::Template;
    use serde_json::json;

    use super::{BasicLayoutEngine, DrawCommand, LayoutEngine, LayoutError};

    const TEMPLATE: &str = r##"
        {
          "name": "Card",
          "document": {
            "width": { "value": 3.5, "unit": "inches" },
            "height": { "value": 2.0, "unit": "inches" }
          },
          "pages": [{
            "elements": [{
              "type": "text",
              "position": {
                "x": { "value": 1.0, "unit": "inches" },
                "y": { "value": 1.0, "unit": "inches" },
                "width": { "value": 2.0, "unit": "inches" },
                "height": { "value": 0.25, "unit": "inches" }
              },
              "value": "{{person.name}} — {{title}}",
              "font_size": { "value": 12.0, "unit": "points" }
            }]
          }]
        }
    "##;

    #[test]
    fn resolves_nested_variables_into_draw_commands() {
        let template: Template = serde_json::from_str(TEMPLATE).unwrap();
        let data: DataRow = serde_json::from_value(json!({
            "person": { "name": "Ada" },
            "title": "Engineer"
        }))
        .unwrap();

        let document = BasicLayoutEngine.layout(&template, &data).unwrap();
        let DrawCommand::Text(text) = &document.pages[0].commands[0] else {
            panic!("expected text command");
        };

        assert_eq!(text.value, "Ada — Engineer");
        assert_eq!(text.bounds.x, 72.0);
    }

    #[test]
    fn reports_missing_variables() {
        let template: Template = serde_json::from_str(TEMPLATE).unwrap();
        let error = BasicLayoutEngine
            .layout(&template, &DataRow::new())
            .unwrap_err();

        assert!(matches!(error, LayoutError::MissingVariable(_)));
    }
}
