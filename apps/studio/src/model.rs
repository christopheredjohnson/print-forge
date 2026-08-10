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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LineEndpoint {
    Start,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerMove {
    Back,
    Backward,
    Forward,
    Front,
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
                rotation: 0.0,
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
            rotation: 0.0,
        }),
        ElementKind::Rectangle => Element::Rectangle(RectangleElement {
            position: Some(bounds(x, y, 180.0, 90.0)),
            fill: Some("#F15A24".to_owned()),
            stroke: None,
            rotation: 0.0,
        }),
        ElementKind::Image => Element::Image(ImageElement {
            position: Some(bounds(x, y, 180.0, 120.0)),
            source: "assets/images/example.png".to_owned(),
            fit: ImageFit::Contain,
            rotation: 0.0,
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
            rotation: 0.0,
        }),
        ElementKind::QrCode => Element::QrCode(QrCodeElement {
            position: Some(bounds(x, y, 108.0, 108.0)),
            value: "https://example.com".to_owned(),
            error_correction: QrErrorCorrection::Medium,
            quiet_zone: 4,
            color: "#20252A".to_owned(),
            background: "#FFFFFF".to_owned(),
            rotation: 0.0,
        }),
        ElementKind::Barcode => Element::Barcode(BarcodeElement {
            position: Some(bounds(x, y, 252.0, 72.0)),
            value: "PRINT-FORGE-001".to_owned(),
            format: BarcodeFormat::Code128,
            quiet_zone: 10,
            color: "#20252A".to_owned(),
            background: "#FFFFFF".to_owned(),
            rotation: 0.0,
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

pub fn element_rotation(element: &Element) -> Option<f32> {
    match element {
        Element::Text(value) => Some(value.rotation),
        Element::Image(value) => Some(value.rotation),
        Element::Rectangle(value) => Some(value.rotation),
        Element::Svg(value) => Some(value.rotation),
        Element::QrCode(value) => Some(value.rotation),
        Element::Barcode(value) => Some(value.rotation),
        Element::Line(_)
        | Element::Group(_)
        | Element::Stack(_)
        | Element::Table(_)
        | Element::Repeater(_)
        | Element::PageBreak => None,
    }
}

pub fn set_element_rotation(element: &mut Element, rotation: f32) {
    let rotation = normalize_rotation(rotation);
    match element {
        Element::Text(value) => value.rotation = rotation,
        Element::Image(value) => value.rotation = rotation,
        Element::Rectangle(value) => value.rotation = rotation,
        Element::Svg(value) => value.rotation = rotation,
        Element::QrCode(value) => value.rotation = rotation,
        Element::Barcode(value) => value.rotation = rotation,
        Element::Line(_)
        | Element::Group(_)
        | Element::Stack(_)
        | Element::Table(_)
        | Element::Repeater(_)
        | Element::PageBreak => {}
    }
}

fn normalize_rotation(rotation: f32) -> f32 {
    (rotation + 180.0).rem_euclid(360.0) - 180.0
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

pub fn translate_line_endpoint(
    element: &mut Element,
    endpoint: LineEndpoint,
    dx_points: f32,
    dy_points: f32,
) {
    let Element::Line(line) = element else {
        return;
    };
    let (x, y) = match endpoint {
        LineEndpoint::Start => (&mut line.x1, &mut line.y1),
        LineEndpoint::End => (&mut line.x2, &mut line.y2),
    };
    add_points(x, dx_points);
    add_points(y, dy_points);
}

pub fn reorder_element(elements: &mut Vec<Element>, index: usize, movement: LayerMove) -> usize {
    if index >= elements.len() {
        return index;
    }
    let target = match movement {
        LayerMove::Back => 0,
        LayerMove::Backward => index.saturating_sub(1),
        LayerMove::Forward => (index + 1).min(elements.len() - 1),
        LayerMove::Front => elements.len() - 1,
    };
    if target == index {
        return index;
    }
    let element = elements.remove(index);
    elements.insert(target, element);
    target
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
    use print_forge_template::Element;
    use print_forge_validation::validate_template;

    use super::{
        ElementKind, LayerMove, LineEndpoint, bounds_points, element_bounds, element_rotation,
        new_element, reorder_element, set_element_rotation, starter_template, translate_element,
        translate_line_endpoint,
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

    #[test]
    fn line_endpoints_can_be_resized_independently() {
        let mut line = new_element(ElementKind::Line, 0.0);
        translate_line_endpoint(&mut line, LineEndpoint::End, 20.0, -10.0);
        let Element::Line(line) = line else {
            panic!("line palette element should remain a line");
        };

        assert_eq!(line.x1.to_points(), 54.0);
        assert_eq!(line.y1.to_points(), 630.0);
        assert_eq!(line.x2.to_points(), 294.0);
        assert_eq!(line.y2.to_points(), 620.0);
    }

    #[test]
    fn visual_elements_have_normalized_rotation() {
        let mut text = new_element(ElementKind::Text, 0.0);
        set_element_rotation(&mut text, 450.0);

        assert_eq!(element_rotation(&text), Some(90.0));
    }

    #[test]
    fn layers_can_move_through_the_canonical_paint_order() {
        let mut elements = vec![
            new_element(ElementKind::Text, 0.0),
            new_element(ElementKind::Rectangle, 0.0),
            new_element(ElementKind::Svg, 0.0),
        ];

        let selected = reorder_element(&mut elements, 0, LayerMove::Front);
        assert_eq!(selected, 2);
        assert!(matches!(elements[2], Element::Text(_)));
        let selected = reorder_element(&mut elements, selected, LayerMove::Backward);
        assert_eq!(selected, 1);
        assert!(matches!(elements[1], Element::Text(_)));
    }
}
