//! Renderer-independent template schema.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

const POINTS_PER_INCH: f32 = 72.0;
const MILLIMETERS_PER_INCH: f32 = 25.4;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Points,
    Inches,
    Millimeters,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Length {
    pub value: f32,
    pub unit: Unit,
}

impl Length {
    #[must_use]
    pub const fn points(value: f32) -> Self {
        Self {
            value,
            unit: Unit::Points,
        }
    }

    #[must_use]
    pub const fn inches(value: f32) -> Self {
        Self {
            value,
            unit: Unit::Inches,
        }
    }

    #[must_use]
    pub const fn millimeters(value: f32) -> Self {
        Self {
            value,
            unit: Unit::Millimeters,
        }
    }

    #[must_use]
    pub fn to_points(self) -> f32 {
        match self.unit {
            Unit::Points => self.value,
            Unit::Inches => self.value * POINTS_PER_INCH,
            Unit::Millimeters => self.value * POINTS_PER_INCH / MILLIMETERS_PER_INCH,
        }
    }
}

/// A strictly parsed device RGB or process CMYK print color.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Color {
    Rgb {
        red: u8,
        green: u8,
        blue: u8,
    },
    Cmyk {
        cyan: f32,
        magenta: f32,
        yellow: f32,
        black: f32,
    },
}

impl Color {
    #[must_use]
    pub fn normalized(self) -> [f32; 4] {
        match self {
            Self::Rgb { red, green, blue } => [
                f32::from(red) / 255.0,
                f32::from(green) / 255.0,
                f32::from(blue) / 255.0,
                0.0,
            ],
            Self::Cmyk {
                cyan,
                magenta,
                yellow,
                black,
            } => [cyan / 100.0, magenta / 100.0, yellow / 100.0, black / 100.0],
        }
    }
}

impl FromStr for Color {
    type Err = ColorParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if let Some(hex) = value.strip_prefix('#') {
            if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(ColorParseError::new("hex colors must use exactly #RRGGBB"));
            }
            return Ok(Self::Rgb {
                red: parse_hex(&hex[0..2])?,
                green: parse_hex(&hex[2..4])?,
                blue: parse_hex(&hex[4..6])?,
            });
        }

        if let Some(body) = value
            .strip_prefix("rgb(")
            .and_then(|value| value.strip_suffix(')'))
        {
            let components = split_components(body, 3, "rgb")?;
            return Ok(Self::Rgb {
                red: parse_rgb_component(components[0])?,
                green: parse_rgb_component(components[1])?,
                blue: parse_rgb_component(components[2])?,
            });
        }

        if let Some(body) = value
            .strip_prefix("cmyk(")
            .and_then(|value| value.strip_suffix(')'))
        {
            let components = split_components(body, 4, "cmyk")?;
            return Ok(Self::Cmyk {
                cyan: parse_percentage(components[0])?,
                magenta: parse_percentage(components[1])?,
                yellow: parse_percentage(components[2])?,
                black: parse_percentage(components[3])?,
            });
        }

        Err(ColorParseError::new(
            "color must use #RRGGBB, rgb(R, G, B), or cmyk(C%, M%, Y%, K%)",
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorParseError {
    message: String,
}

impl ColorParseError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ColorParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ColorParseError {}

fn parse_hex(value: &str) -> Result<u8, ColorParseError> {
    u8::from_str_radix(value, 16)
        .map_err(|_| ColorParseError::new("hex colors must use exactly #RRGGBB"))
}

fn split_components<'a>(
    value: &'a str,
    expected: usize,
    model: &str,
) -> Result<Vec<&'a str>, ColorParseError> {
    let components = value.split(',').map(str::trim).collect::<Vec<_>>();
    if components.len() != expected || components.iter().any(|value| value.is_empty()) {
        return Err(ColorParseError::new(format!(
            "{model} colors require exactly {expected} components"
        )));
    }
    Ok(components)
}

fn parse_rgb_component(value: &str) -> Result<u8, ColorParseError> {
    value
        .parse::<u8>()
        .map_err(|_| ColorParseError::new("RGB components must be integers from 0 to 255"))
}

fn parse_percentage(value: &str) -> Result<f32, ColorParseError> {
    let number = value
        .strip_suffix('%')
        .ok_or_else(|| ColorParseError::new("CMYK components must include a % suffix"))?
        .parse::<f32>()
        .map_err(|_| ColorParseError::new("CMYK components must be percentages from 0% to 100%"))?;
    if !number.is_finite() || !(0.0..=100.0).contains(&number) {
        return Err(ColorParseError::new(
            "CMYK components must be percentages from 0% to 100%",
        ));
    }
    Ok(number)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub width: Length,
    pub height: Length,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bleed: Option<Length>,
    #[serde(default)]
    pub metadata: DocumentMetadata,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Template {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub name: String,
    pub document: Document,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<Field>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fonts: Vec<FontFamily>,
    pub pages: Vec<Page>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FontFamily {
    pub name: String,
    pub regular: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bold: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub italic: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bold_italic: Option<String>,
}

const fn default_schema_version() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Page {
    /// Absolute-positioned elements repeated on every physical page generated
    /// from this template page.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub header: Vec<Element>,
    /// Absolute-positioned elements repeated on every physical page generated
    /// from this template page.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub footer: Vec<Element>,
    #[serde(default)]
    pub elements: Vec<Element>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Field {
    pub name: String,
    pub field_type: FieldType,
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldType {
    Text,
    Number,
    Image,
    Boolean,
    Collection,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    pub x: Length,
    pub y: Length,
    pub width: Length,
    pub height: Length,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Element {
    Text(TextElement),
    Image(ImageElement),
    Rectangle(RectangleElement),
    Line(LineElement),
    Svg(SvgElement),
    QrCode(QrCodeElement),
    Group(GroupElement),
    Stack(StackElement),
    Table(TableElement),
    Repeater(RepeaterElement),
    PageBreak,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextElement {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Bounds>,
    pub value: String,
    pub font_size: Length,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    #[serde(default)]
    pub font_style: FontStyle,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_height: Option<Length>,
    #[serde(default)]
    pub align: TextAlign,
    #[serde(default)]
    pub overflow: TextOverflow,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_font_size: Option<Length>,
    #[serde(default = "default_color")]
    pub color: String,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FontStyle {
    #[default]
    Regular,
    Bold,
    Italic,
    BoldItalic,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextOverflow {
    Clip,
    Shrink,
    #[default]
    Error,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageElement {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Bounds>,
    pub source: String,
    #[serde(default)]
    pub fit: ImageFit,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageFit {
    #[default]
    Contain,
    Cover,
    Stretch,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RectangleElement {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Bounds>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<Stroke>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    pub width: Length,
    #[serde(default = "default_color")]
    pub color: String,
    #[serde(default)]
    pub dash: DashStyle,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DashStyle {
    #[default]
    Solid,
    Dashed,
    Dotted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LineElement {
    pub x1: Length,
    pub y1: Length,
    pub x2: Length,
    pub y2: Length,
    pub width: Length,
    #[serde(default = "default_color")]
    pub color: String,
    #[serde(default)]
    pub dash: DashStyle,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SvgElement {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Bounds>,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QrCodeElement {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Bounds>,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupElement {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Bounds>,
    #[serde(default)]
    pub children: Vec<Element>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StackElement {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Bounds>,
    #[serde(default)]
    pub direction: StackDirection,
    #[serde(default = "zero_length")]
    pub gap: Length,
    #[serde(default = "zero_length")]
    pub padding: Length,
    #[serde(default)]
    pub overflow: FlowOverflow,
    #[serde(default)]
    pub keep_together: bool,
    #[serde(default = "default_orphans")]
    pub orphans: usize,
    #[serde(default)]
    pub children: Vec<Element>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StackDirection {
    Horizontal,
    #[default]
    Vertical,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowOverflow {
    #[default]
    Paginate,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableElement {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Bounds>,
    pub source: String,
    #[serde(default)]
    pub header: bool,
    pub columns: Vec<TableColumn>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableColumn {
    pub field: String,
    pub header: String,
    pub width: f32,
    #[serde(default)]
    pub align: TextAlign,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepeaterElement {
    pub source: String,
    #[serde(default)]
    pub layout: RepeatLayout,
    pub template: Box<Element>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepeatLayout {
    Horizontal,
    #[default]
    Vertical,
    Grid,
}

fn default_color() -> String {
    "#000000".to_owned()
}

const fn zero_length() -> Length {
    Length::points(0.0)
}

const fn default_orphans() -> usize {
    1
}

#[cfg(test)]
mod tests {
    use super::{Color, Element, FlowOverflow, Length, Template};

    #[test]
    fn converts_supported_units_to_points() {
        assert_eq!(Length::inches(1.0).to_points(), 72.0);
        assert!((Length::millimeters(25.4).to_points() - 72.0).abs() < f32::EPSILON);
    }

    #[test]
    fn strictly_parses_rgb_and_cmyk_colors() {
        assert_eq!(
            "rgb(12, 34, 56)".parse::<Color>().unwrap(),
            Color::Rgb {
                red: 12,
                green: 34,
                blue: 56
            }
        );
        assert_eq!(
            "cmyk(0%, 25.5%, 50%, 100%)".parse::<Color>().unwrap(),
            Color::Cmyk {
                cyan: 0.0,
                magenta: 25.5,
                yellow: 50.0,
                black: 100.0
            }
        );
        assert!("#12345".parse::<Color>().is_err());
        assert!("rgb(256, 0, 0)".parse::<Color>().is_err());
        assert!("cmyk(0, 0%, 0%, 0%)".parse::<Color>().is_err());
    }

    #[test]
    fn stack_flow_options_have_safe_defaults() {
        let template: Template = serde_json::from_str(
            r#"{
              "name": "Flow defaults",
              "document": {
                "width": { "value": 100, "unit": "points" },
                "height": { "value": 100, "unit": "points" }
              },
              "pages": [{
                "elements": [{ "type": "stack", "children": [] }]
              }]
            }"#,
        )
        .unwrap();

        let Element::Stack(stack) = &template.pages[0].elements[0] else {
            panic!("expected stack");
        };
        assert_eq!(stack.gap, Length::points(0.0));
        assert_eq!(stack.padding, Length::points(0.0));
        assert_eq!(stack.overflow, FlowOverflow::Paginate);
        assert!(!stack.keep_together);
        assert_eq!(stack.orphans, 1);
    }
}
