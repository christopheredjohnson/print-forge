use print_forge_template::{
    BarcodeElement, BarcodeFormat, Bounds, DashStyle, Document, DocumentMetadata, Element, Field,
    FieldType, ImageElement, ImageFit, Length, LineElement, Page, QrCodeElement, QrErrorCorrection,
    RectangleElement, SvgElement, Template, TextAlign, TextElement, TextOverflow, Unit,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ElementKind {
    Text,
    Rectangle,
    Image,
    Line,
    Svg,
    QrCode,
    Barcode,
}

impl ElementKind {
    pub const ALL: [Self; 7] = [
        Self::Text,
        Self::Rectangle,
        Self::Image,
        Self::Line,
        Self::Svg,
        Self::QrCode,
        Self::Barcode,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Text => "Text",
            Self::Rectangle => "Rectangle",
            Self::Image => "Image",
            Self::Line => "Line",
            Self::Svg => "SVG",
            Self::QrCode => "QR code",
            Self::Barcode => "Barcode",
        }
    }
}

pub fn starter_template() -> Template {
    Template {
        schema_version: 1,
        name: "Untitled print project".to_owned(),
        document: Document {
            width: Length::inches(8.5),
            height: Length::inches(11.0),
            bleed: Some(Length::inches(0.125)),
            metadata: DocumentMetadata::default(),
        },
        fields: Vec::new(),
        fonts: Vec::new(),
        pages: vec![Page {
            header: Vec::new(),
            footer: Vec::new(),
            elements: vec![Element::Text(TextElement {
                position: Some(bounds(54.0, 702.0, 504.0, 42.0)),
                value: "Start forging your template".to_owned(),
                font_size: Length::points(24.0),
                font: None,
                font_style: Default::default(),
                line_height: None,
                align: TextAlign::Left,
                overflow: TextOverflow::Error,
                min_font_size: None,
                color: "#20252A".to_owned(),
            })],
        }],
    }
}

pub fn new_element(kind: ElementKind, offset: f32) -> Element {
    let x = 54.0 + offset;
    let y = 630.0 - offset;
    match kind {
        ElementKind::Text => Element::Text(TextElement {
            position: Some(bounds(x, y, 240.0, 36.0)),
            value: "New text".to_owned(),
            font_size: Length::points(18.0),
            font: None,
            font_style: Default::default(),
            line_height: None,
            align: TextAlign::Left,
            overflow: TextOverflow::Error,
            min_font_size: None,
            color: "#20252A".to_owned(),
        }),
        ElementKind::Rectangle => Element::Rectangle(RectangleElement {
            position: Some(bounds(x, y, 180.0, 90.0)),
            fill: Some("#F15A24".to_owned()),
            stroke: None,
        }),
        ElementKind::Image => Element::Image(ImageElement {
            position: Some(bounds(x, y, 180.0, 120.0)),
            source: "assets/images/example.png".to_owned(),
            fit: ImageFit::Contain,
        }),
        ElementKind::Line => Element::Line(LineElement {
            x1: Length::points(x),
            y1: Length::points(y),
            x2: Length::points(x + 220.0),
            y2: Length::points(y),
            width: Length::points(1.0),
            color: "#20252A".to_owned(),
            dash: DashStyle::Solid,
        }),
        ElementKind::Svg => Element::Svg(SvgElement {
            position: Some(bounds(x, y, 144.0, 144.0)),
            source: "assets/vectors/example.svg".to_owned(),
            fit: ImageFit::Contain,
        }),
        ElementKind::QrCode => Element::QrCode(QrCodeElement {
            position: Some(bounds(x, y, 108.0, 108.0)),
            value: "https://example.com".to_owned(),
            error_correction: QrErrorCorrection::Medium,
            quiet_zone: 4,
            color: "#20252A".to_owned(),
            background: "#FFFFFF".to_owned(),
        }),
        ElementKind::Barcode => Element::Barcode(BarcodeElement {
            position: Some(bounds(x, y, 252.0, 72.0)),
            value: "PRINT-FORGE-001".to_owned(),
            format: BarcodeFormat::Code128,
            quiet_zone: 10,
            color: "#20252A".to_owned(),
            background: "#FFFFFF".to_owned(),
        }),
    }
}

pub fn new_field(index: usize) -> Field {
    Field {
        name: format!("field_{}", index + 1),
        field_type: FieldType::Text,
        required: false,
    }
}

pub const fn blank_page() -> Page {
    Page {
        header: Vec::new(),
        footer: Vec::new(),
        elements: Vec::new(),
    }
}

pub fn element_label(element: &Element, index: usize) -> String {
    let kind = match element {
        Element::Text(text) => {
            let preview = text.value.lines().next().unwrap_or_default();
            return format!("Text · {}", truncate(preview, 22));
        }
        Element::Image(_) => "Image",
        Element::Rectangle(_) => "Rectangle",
        Element::Line(_) => "Line",
        Element::Svg(_) => "SVG",
        Element::QrCode(_) => "QR code",
        Element::Barcode(_) => "Barcode",
        Element::Group(_) => "Group",
        Element::Stack(_) => "Stack",
        Element::Table(_) => "Table",
        Element::Repeater(_) => "Repeater",
        Element::PageBreak => "Page break",
    };
    format!("{kind} {}", index + 1)
}

pub fn element_bounds(element: &Element) -> Option<&Bounds> {
    match element {
        Element::Text(value) => value.position.as_ref(),
        Element::Image(value) => value.position.as_ref(),
        Element::Rectangle(value) => value.position.as_ref(),
        Element::Svg(value) => value.position.as_ref(),
        Element::QrCode(value) => value.position.as_ref(),
        Element::Barcode(value) => value.position.as_ref(),
        Element::Group(value) => value.position.as_ref(),
        Element::Stack(value) => value.position.as_ref(),
        Element::Table(value) => value.position.as_ref(),
        Element::Line(_) | Element::Repeater(_) | Element::PageBreak => None,
    }
}

pub fn element_bounds_mut(element: &mut Element) -> Option<&mut Bounds> {
    match element {
        Element::Text(value) => value.position.as_mut(),
        Element::Image(value) => value.position.as_mut(),
        Element::Rectangle(value) => value.position.as_mut(),
        Element::Svg(value) => value.position.as_mut(),
        Element::QrCode(value) => value.position.as_mut(),
        Element::Barcode(value) => value.position.as_mut(),
        Element::Group(value) => value.position.as_mut(),
        Element::Stack(value) => value.position.as_mut(),
        Element::Table(value) => value.position.as_mut(),
        Element::Line(_) | Element::Repeater(_) | Element::PageBreak => None,
    }
}

pub fn bounds_points(bounds: &Bounds) -> [f32; 4] {
    [
        bounds.x.to_points(),
        bounds.y.to_points(),
        bounds.width.to_points(),
        bounds.height.to_points(),
    ]
}

pub fn translate_element(element: &mut Element, dx_points: f32, dy_points: f32) {
    if let Some(bounds) = element_bounds_mut(element) {
        add_points(&mut bounds.x, dx_points);
        add_points(&mut bounds.y, dy_points);
    } else if let Element::Line(line) = element {
        add_points(&mut line.x1, dx_points);
        add_points(&mut line.x2, dx_points);
        add_points(&mut line.y1, dy_points);
        add_points(&mut line.y2, dy_points);
    }
}

pub fn resize_element(element: &mut Element, width_points: f32, height_points: f32) {
    if let Some(bounds) = element_bounds_mut(element) {
        set_points(&mut bounds.width, width_points.max(1.0));
        set_points(&mut bounds.height, height_points.max(1.0));
    }
}

pub fn add_points(length: &mut Length, points: f32) {
    let updated = length.to_points() + points;
    set_points(length, updated);
}

pub fn set_points(length: &mut Length, points: f32) {
    length.value = match length.unit {
        Unit::Points => points,
        Unit::Inches => points / 72.0,
        Unit::Millimeters => points * 25.4 / 72.0,
    };
}

fn bounds(x: f32, y: f32, width: f32, height: f32) -> Bounds {
    Bounds {
        x: Length::points(x),
        y: Length::points(y),
        width: Length::points(width),
        height: Length::points(height),
    }
}

fn truncate(value: &str, maximum: usize) -> String {
    let mut characters = value.chars();
    let preview: String = characters.by_ref().take(maximum).collect();
    if characters.next().is_some() {
        format!("{preview}…")
    } else if preview.is_empty() {
        "Untitled".to_owned()
    } else {
        preview
    }
}

#[cfg(test)]
mod tests {
    use print_forge_validation::validate_template;

    use super::{
        ElementKind, bounds_points, element_bounds, new_element, starter_template,
        translate_element,
    };

    #[test]
    fn starter_template_is_valid_and_round_trips() {
        let template = starter_template();
        assert!(validate_template(&template).is_valid());
        let json = serde_json::to_string_pretty(&template).unwrap();
        let decoded = serde_json::from_str(&json).unwrap();
        assert_eq!(template, decoded);
    }

    #[test]
    fn palette_elements_have_editable_positions() {
        for kind in ElementKind::ALL {
            let mut element = new_element(kind, 0.0);
            translate_element(&mut element, 10.0, -5.0);
            if kind != ElementKind::Line {
                let bounds = bounds_points(element_bounds(&element).unwrap());
                assert_eq!(bounds[0], 64.0);
                assert_eq!(bounds[1], 625.0);
            }
        }
    }
}
