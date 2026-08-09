//! Renderer-independent template schema.

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub width: Length,
    pub height: Length,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bleed: Option<Length>,
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
    pub gap: Length,
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

#[cfg(test)]
mod tests {
    use super::Length;

    #[test]
    fn converts_supported_units_to_points() {
        assert_eq!(Length::inches(1.0).to_points(), 72.0);
        assert!((Length::millimeters(25.4).to_points() - 72.0).abs() < f32::EPSILON);
    }
}
