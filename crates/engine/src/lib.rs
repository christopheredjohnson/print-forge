//! Layout contracts shared by template compilers and document renderers.

use print_forge_dataset::DataRow;
use std::{
    fs,
    path::{Path, PathBuf},
};

use print_forge_template::{
    DashStyle, Element, FontFamily, FontStyle, ImageFit, Stroke, Template, TextAlign, TextOverflow,
};
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
    pub commands: Vec<ResolvedCommand>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedCommand {
    pub source_path: String,
    pub command: DrawCommand,
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
    pub lines: Vec<TextLine>,
    pub font_size_pt: f32,
    pub line_height_pt: f32,
    pub font: ResolvedFont,
    pub color: String,
    pub clip: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextLine {
    pub value: String,
    pub x: f32,
    pub y: f32,
    pub word_spacing_pt: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedFont {
    Builtin(String),
    External(PathBuf),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImageCommand {
    pub bounds: Rect,
    pub source: PathBuf,
    pub fit: ImageFit,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutOptions {
    pub asset_base: PathBuf,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        Self {
            asset_base: PathBuf::from("."),
        }
    }
}

/// Absolute-position layout for the first renderer vertical slice.
///
/// Flow elements remain explicit errors so unsupported template content is
/// never silently omitted from output.
#[derive(Debug, Default, Clone, Copy)]
pub struct BasicLayoutEngine;

impl LayoutEngine for BasicLayoutEngine {
    fn layout(&self, template: &Template, data: &DataRow) -> Result<ResolvedDocument, LayoutError> {
        self.layout_with_options(template, data, &LayoutOptions::default())
    }
}

impl BasicLayoutEngine {
    pub fn layout_with_options(
        &self,
        template: &Template,
        data: &DataRow,
        options: &LayoutOptions,
    ) -> Result<ResolvedDocument, LayoutError> {
        let width_pt = template.document.width.to_points();
        let height_pt = template.document.height.to_points();

        if !width_pt.is_finite() || !height_pt.is_finite() || width_pt <= 0.0 || height_pt <= 0.0 {
            return Err(LayoutError::Document {
                message: "document dimensions must be positive and finite".to_owned(),
            });
        }

        let pages = template
            .pages
            .iter()
            .enumerate()
            .map(|(page_index, page)| {
                let commands = page
                    .elements
                    .iter()
                    .enumerate()
                    .map(|(element_index, element)| {
                        let source_path = format!("pages[{page_index}].elements[{element_index}]");
                        layout_element(element, template, data, options)
                            .map(|command| ResolvedCommand {
                                source_path: source_path.clone(),
                                command,
                            })
                            .map_err(|source| LayoutError::Element {
                                page_index,
                                element_path: source_path,
                                source,
                            })
                    })
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

fn layout_element(
    element: &Element,
    template: &Template,
    data: &DataRow,
    options: &LayoutOptions,
) -> Result<DrawCommand, ElementLayoutError> {
    match element {
        Element::Text(text) => {
            let bounds = text
                .position
                .as_ref()
                .ok_or_else(|| missing_position("text"))?;
            let bounds = resolve_bounds(bounds);
            let value = resolve_string(&text.value, data)?;
            let font = resolve_font(template, text.font.as_deref(), text.font_style, options)?;
            let metrics = FontMetrics::load(&font)?;
            let font_size_pt = text.font_size.to_points();
            let line_height_pt = text
                .line_height
                .map_or(font_size_pt * 1.2, print_forge_template::Length::to_points);
            let min_font_size_pt = text
                .min_font_size
                .map_or(6.0, print_forge_template::Length::to_points);
            let laid_out = layout_text(
                &value,
                TextLayoutSpec {
                    bounds,
                    requested_font_size: font_size_pt,
                    requested_line_height: line_height_pt,
                    min_font_size: min_font_size_pt,
                    align: text.align,
                    overflow: text.overflow,
                },
                &metrics,
            )?;

            Ok(DrawCommand::Text(TextCommand {
                bounds,
                lines: laid_out.lines,
                font_size_pt: laid_out.font_size_pt,
                line_height_pt: laid_out.line_height_pt,
                font,
                color: text.color.clone(),
                clip: text.overflow == TextOverflow::Clip,
            }))
        }
        Element::Image(image) => {
            let bounds = image
                .position
                .as_ref()
                .ok_or_else(|| missing_position("image"))?;

            Ok(DrawCommand::Image(ImageCommand {
                bounds: resolve_bounds(bounds),
                source: resolve_asset_path(
                    &options.asset_base,
                    &resolve_string(&image.source, data)?,
                ),
                fit: image.fit,
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
        Element::QrCode(_) => Err(ElementLayoutError::UnsupportedElement("qr_code")),
        Element::Group(_) => Err(ElementLayoutError::UnsupportedElement("group")),
        Element::Stack(_) => Err(ElementLayoutError::UnsupportedElement("stack")),
        Element::Table(_) => Err(ElementLayoutError::UnsupportedElement("table")),
        Element::Repeater(_) => Err(ElementLayoutError::UnsupportedElement("repeater")),
        Element::PageBreak => Err(ElementLayoutError::UnsupportedElement("page_break")),
    }
}

#[derive(Debug)]
struct LaidOutText {
    lines: Vec<TextLine>,
    font_size_pt: f32,
    line_height_pt: f32,
}

#[derive(Debug)]
struct WrappedLine {
    value: String,
    width_pt: f32,
    paragraph_end: bool,
}

#[derive(Debug, Clone, Copy)]
struct TextLayoutSpec {
    bounds: Rect,
    requested_font_size: f32,
    requested_line_height: f32,
    min_font_size: f32,
    align: TextAlign,
    overflow: TextOverflow,
}

fn layout_text(
    value: &str,
    spec: TextLayoutSpec,
    metrics: &FontMetrics,
) -> Result<LaidOutText, ElementLayoutError> {
    let TextLayoutSpec {
        bounds,
        requested_font_size,
        requested_line_height,
        min_font_size,
        align,
        overflow,
    } = spec;
    if !requested_font_size.is_finite() || requested_font_size <= 0.0 {
        return Err(ElementLayoutError::InvalidLayout(
            "font size must be positive and finite".to_owned(),
        ));
    }
    if !requested_line_height.is_finite() || requested_line_height <= 0.0 {
        return Err(ElementLayoutError::InvalidLayout(
            "line height must be positive and finite".to_owned(),
        ));
    }
    if bounds.width <= 0.0 || bounds.height <= 0.0 {
        return Err(ElementLayoutError::InvalidLayout(
            "text bounds must have positive width and height".to_owned(),
        ));
    }

    let minimum = min_font_size.clamp(0.5, requested_font_size);
    let line_height_ratio = requested_line_height / requested_font_size;
    let mut font_size = requested_font_size;

    let (wrapped, line_height) = loop {
        let line_height = font_size * line_height_ratio;
        let wrapped = wrap_text(value, bounds.width, font_size, metrics);
        let fits = text_fits(&wrapped, bounds, line_height);

        if fits || overflow != TextOverflow::Shrink {
            break (wrapped, line_height);
        }
        if font_size <= minimum + f32::EPSILON {
            return Err(ElementLayoutError::InvalidLayout(format!(
                "text does not fit after shrinking to {minimum:.2}pt"
            )));
        }
        font_size = (font_size - 0.25).max(minimum);
    };

    if overflow == TextOverflow::Error && !text_fits(&wrapped, bounds, line_height) {
        return Err(ElementLayoutError::InvalidLayout(
            "text exceeds its absolute bounds; use overflow clip or shrink".to_owned(),
        ));
    }

    let top_baseline = bounds.y + bounds.height - font_size;
    let lines = wrapped
        .iter()
        .enumerate()
        .map(|(index, line)| {
            let available = (bounds.width - line.width_pt).max(0.0);
            let spaces = line
                .value
                .chars()
                .filter(|character| *character == ' ')
                .count();
            let (x, word_spacing_pt) = match align {
                TextAlign::Left => (bounds.x, 0.0),
                TextAlign::Center => (bounds.x + available / 2.0, 0.0),
                TextAlign::Right => (bounds.x + available, 0.0),
                TextAlign::Justify if !line.paragraph_end && spaces > 0 => {
                    (bounds.x, available / spaces as f32)
                }
                TextAlign::Justify => (bounds.x, 0.0),
            };

            TextLine {
                value: line.value.clone(),
                x,
                y: top_baseline - index as f32 * line_height,
                word_spacing_pt,
            }
        })
        .collect();

    Ok(LaidOutText {
        lines,
        font_size_pt: font_size,
        line_height_pt: line_height,
    })
}

fn text_fits(lines: &[WrappedLine], bounds: Rect, line_height: f32) -> bool {
    lines
        .iter()
        .all(|line| line.width_pt <= bounds.width + 0.01)
        && lines.len() as f32 * line_height <= bounds.height + 0.01
}

fn wrap_text(
    value: &str,
    max_width: f32,
    font_size: f32,
    metrics: &FontMetrics,
) -> Vec<WrappedLine> {
    let mut lines = Vec::new();
    let paragraphs = value.split('\n').collect::<Vec<_>>();

    for paragraph in paragraphs {
        let words = paragraph.split_whitespace().collect::<Vec<_>>();
        if words.is_empty() {
            lines.push(WrappedLine {
                value: String::new(),
                width_pt: 0.0,
                paragraph_end: true,
            });
            continue;
        }

        let mut current = String::new();
        for word in words {
            let candidate = if current.is_empty() {
                word.to_owned()
            } else {
                format!("{current} {word}")
            };
            let candidate_width = metrics.measure(&candidate, font_size);

            if !current.is_empty() && candidate_width > max_width {
                lines.push(WrappedLine {
                    width_pt: metrics.measure(&current, font_size),
                    value: std::mem::take(&mut current),
                    paragraph_end: false,
                });
                current.push_str(word);
            } else {
                current = candidate;
            }
        }

        lines.push(WrappedLine {
            width_pt: metrics.measure(&current, font_size),
            value: current,
            paragraph_end: true,
        });
    }

    lines
}

enum FontMetrics {
    Builtin,
    External(Vec<u8>),
}

impl FontMetrics {
    fn load(font: &ResolvedFont) -> Result<Self, ElementLayoutError> {
        match font {
            ResolvedFont::Builtin(_) => Ok(Self::Builtin),
            ResolvedFont::External(path) => fs::read(path).map(Self::External).map_err(|error| {
                ElementLayoutError::InvalidLayout(format!(
                    "cannot read font asset {}: {error}",
                    path.display()
                ))
            }),
        }
    }

    fn measure(&self, value: &str, font_size: f32) -> f32 {
        match self {
            Self::Builtin => value.chars().map(builtin_advance_em).sum::<f32>() * font_size,
            Self::External(bytes) => {
                let Ok(face) = ttf_parser::Face::parse(bytes, 0) else {
                    return value.chars().map(builtin_advance_em).sum::<f32>() * font_size;
                };
                let units_per_em = f32::from(face.units_per_em());
                value
                    .chars()
                    .map(|character| {
                        face.glyph_index(character)
                            .and_then(|glyph| face.glyph_hor_advance(glyph))
                            .map_or(units_per_em * 0.6, f32::from)
                    })
                    .sum::<f32>()
                    * font_size
                    / units_per_em
            }
        }
    }
}

fn builtin_advance_em(character: char) -> f32 {
    match character {
        ' ' => 0.278,
        'i' | 'l' | 'I' | '!' | '|' | '.' | ',' | ':' | ';' | '\'' => 0.278,
        'f' | 't' | 'r' | '(' | ')' | '[' | ']' => 0.36,
        'm' | 'w' | 'M' | 'W' | '@' => 0.84,
        character if character.is_ascii_uppercase() => 0.67,
        character if character.is_ascii_digit() => 0.56,
        _ => 0.56,
    }
}

fn resolve_font(
    template: &Template,
    requested: Option<&str>,
    style: FontStyle,
    options: &LayoutOptions,
) -> Result<ResolvedFont, ElementLayoutError> {
    let name = requested.unwrap_or("helvetica");
    if let Some(family) = template.fonts.iter().find(|family| family.name == name) {
        let source = font_variant(family, style)?;
        return Ok(ResolvedFont::External(resolve_asset_path(
            &options.asset_base,
            source,
        )));
    }

    builtin_font_variant(name, style)
        .map(|name| ResolvedFont::Builtin(name.to_owned()))
        .ok_or_else(|| {
            ElementLayoutError::InvalidLayout(format!(
                "font family {name:?} is not declared and is not a supported built-in font"
            ))
        })
}

fn font_variant(family: &FontFamily, style: FontStyle) -> Result<&str, ElementLayoutError> {
    let source = match style {
        FontStyle::Regular => Some(family.regular.as_str()),
        FontStyle::Bold => family.bold.as_deref(),
        FontStyle::Italic => family.italic.as_deref(),
        FontStyle::BoldItalic => family.bold_italic.as_deref(),
    };
    source.ok_or_else(|| {
        ElementLayoutError::InvalidLayout(format!(
            "font family {:?} does not declare its {style:?} variant",
            family.name
        ))
    })
}

fn builtin_font_variant(name: &str, style: FontStyle) -> Option<&'static str> {
    let family = match name.to_ascii_lowercase().as_str() {
        "helvetica"
        | "sans-serif"
        | "helvetica-bold"
        | "helvetica-oblique"
        | "helvetica-bold-oblique" => "helvetica",
        "times" | "times-roman" | "serif" | "times-bold" | "times-italic" | "times-bold-italic" => {
            "times"
        }
        "courier" | "monospace" | "courier-bold" | "courier-oblique" | "courier-bold-oblique" => {
            "courier"
        }
        _ => return None,
    };
    Some(match (family, style) {
        ("helvetica", FontStyle::Regular) => "helvetica",
        ("helvetica", FontStyle::Bold) => "helvetica-bold",
        ("helvetica", FontStyle::Italic) => "helvetica-oblique",
        ("helvetica", FontStyle::BoldItalic) => "helvetica-bold-oblique",
        ("times", FontStyle::Regular) => "times-roman",
        ("times", FontStyle::Bold) => "times-bold",
        ("times", FontStyle::Italic) => "times-italic",
        ("times", FontStyle::BoldItalic) => "times-bold-italic",
        ("courier", FontStyle::Regular) => "courier",
        ("courier", FontStyle::Bold) => "courier-bold",
        ("courier", FontStyle::Italic) => "courier-oblique",
        ("courier", FontStyle::BoldItalic) => "courier-bold-oblique",
        _ => unreachable!(),
    })
}

fn resolve_asset_path(asset_base: &Path, source: &str) -> PathBuf {
    let path = Path::new(source);
    if path.is_absolute() {
        path.to_owned()
    } else {
        asset_base.join(path)
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

fn missing_position(element: &'static str) -> ElementLayoutError {
    ElementLayoutError::InvalidLayout(format!(
        "absolute-positioned {element} element is missing position"
    ))
}

fn resolve_string(input: &str, data: &DataRow) -> Result<String, ElementLayoutError> {
    let mut output = String::with_capacity(input.len());
    let mut remaining = input;

    while let Some(start) = remaining.find("{{") {
        output.push_str(&remaining[..start]);
        let expression = &remaining[start + 2..];
        let end = expression.find("}}").ok_or_else(|| {
            ElementLayoutError::InvalidTemplate("unclosed template variable".to_owned())
        })?;
        let key = expression[..end].trim();

        if key.is_empty() {
            return Err(ElementLayoutError::InvalidTemplate(
                "template variable name cannot be empty".to_owned(),
            ));
        }

        let value = lookup_value(data, key)
            .ok_or_else(|| ElementLayoutError::MissingVariable(key.to_owned()))?;

        if let Some(value) = value.as_str() {
            output.push_str(value);
        } else {
            output.push_str(&value.to_string());
        }

        remaining = &expression[end + 2..];
    }

    if remaining.contains("}}") {
        return Err(ElementLayoutError::InvalidTemplate(
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
    #[error("document: {message}")]
    Document { message: String },
    #[error("page {page_index}, element {element_path}: {source}")]
    Element {
        page_index: usize,
        element_path: String,
        #[source]
        source: ElementLayoutError,
    },
}

#[derive(Debug, Error)]
pub enum ElementLayoutError {
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
    use std::path::PathBuf;

    use print_forge_dataset::DataRow;
    use print_forge_template::{Template, TextAlign, TextOverflow};
    use serde_json::json;

    use super::{
        BasicLayoutEngine, DrawCommand, ElementLayoutError, FontMetrics, LayoutEngine, LayoutError,
        LayoutOptions, Rect, TextLayoutSpec, layout_text,
    };

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
        let DrawCommand::Text(text) = &document.pages[0].commands[0].command else {
            panic!("expected text command");
        };

        assert_eq!(text.lines[0].value, "Ada — Engineer");
        assert_eq!(text.bounds.x, 72.0);
    }

    #[test]
    fn reports_missing_variables() {
        let template: Template = serde_json::from_str(TEMPLATE).unwrap();
        let error = BasicLayoutEngine
            .layout(&template, &DataRow::new())
            .unwrap_err();

        assert!(matches!(
            error,
            LayoutError::Element {
                source: ElementLayoutError::MissingVariable(_),
                ..
            }
        ));
        assert!(
            error
                .to_string()
                .contains("page 0, element pages[0].elements[0]")
        );
    }

    #[test]
    fn wraps_aligns_and_shrinks_text_inside_absolute_bounds() {
        let bounds = Rect {
            x: 10.0,
            y: 20.0,
            width: 80.0,
            height: 34.0,
        };
        let text = layout_text(
            "Measured words wrap and shrink to fit",
            TextLayoutSpec {
                bounds,
                requested_font_size: 18.0,
                requested_line_height: 21.6,
                min_font_size: 6.0,
                align: TextAlign::Center,
                overflow: TextOverflow::Shrink,
            },
            &FontMetrics::Builtin,
        )
        .unwrap();

        assert!(text.lines.len() >= 2);
        assert!(text.font_size_pt < 18.0);
        assert!(text.lines.iter().all(|line| line.x >= bounds.x));
        assert!(text.lines.len() as f32 * text.line_height_pt <= bounds.height + f32::EPSILON);
    }

    #[test]
    fn error_overflow_rejects_text_that_does_not_fit() {
        let error = layout_text(
            "this cannot fit",
            TextLayoutSpec {
                bounds: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 20.0,
                    height: 10.0,
                },
                requested_font_size: 12.0,
                requested_line_height: 14.4,
                min_font_size: 6.0,
                align: TextAlign::Left,
                overflow: TextOverflow::Error,
            },
            &FontMetrics::Builtin,
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("text exceeds its absolute bounds")
        );
    }

    #[test]
    fn resolves_fixture_assets_from_the_template_directory() {
        let template: Template =
            serde_json::from_str(include_str!("../../../examples/absolute-layout.json")).unwrap();
        let data: DataRow = serde_json::from_value(json!({
            "shrink_text": "A sentence that shrinks"
        }))
        .unwrap();
        let asset_base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        let document = BasicLayoutEngine
            .layout_with_options(&template, &data, &LayoutOptions { asset_base })
            .unwrap();

        let DrawCommand::Image(image) = &document.pages[0].commands[11].command else {
            panic!("expected image command");
        };
        assert!(
            image
                .source
                .ends_with("examples/assets/images/layout-fixture.png")
        );
    }
}
