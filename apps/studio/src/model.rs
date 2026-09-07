use print_forge_template::{
    BarcodeElement, BarcodeFormat, Bounds, DashStyle, Document, DocumentMetadata, Element, Field,
    FieldType, FlowOverflow, FontStyle, GroupElement, ImageElement, ImageFit, Length, LineElement,
    Page, QrCodeElement, QrErrorCorrection, RectangleElement, RepeatLayout, RepeaterElement,
    StackDirection, StackElement, Stroke, SvgElement, TableColumn, TableColumnWidth, TableElement,
    Template, TextAlign, TextElement, TextOverflow, Unit,
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
    Group,
    Stack,
    Table,
    Repeater,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignMode {
    Left,
    HorizontalCenter,
    Right,
    Bottom,
    VerticalCenter,
    Top,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistributionAxis {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SnapResult {
    pub delta: [f32; 2],
    pub x: Option<f32>,
    pub y: Option<f32>,
}

impl ElementKind {
    pub const BASIC: [Self; 7] = [
        Self::Text,
        Self::Rectangle,
        Self::Image,
        Self::Line,
        Self::Svg,
        Self::QrCode,
        Self::Barcode,
    ];

    pub const ADVANCED: [Self; 4] = [Self::Group, Self::Stack, Self::Table, Self::Repeater];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Text => "Text",
            Self::Rectangle => "Rectangle",
            Self::Image => "Image",
            Self::Line => "Line",
            Self::Svg => "SVG",
            Self::QrCode => "QR code",
            Self::Barcode => "Barcode",
            Self::Group => "Group",
            Self::Stack => "Flow stack",
            Self::Table => "Table",
            Self::Repeater => "Repeater",
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
                name: None,
                visible: true,
                locked: false,
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
            name: None,
            visible: true,
            locked: false,
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
            name: None,
            visible: true,
            locked: false,
            position: Some(bounds(x, y, 180.0, 90.0)),
            fill: Some("#F15A24".to_owned()),
            stroke: None,
            rotation: 0.0,
        }),
        ElementKind::Image => Element::Image(ImageElement {
            name: None,
            visible: true,
            locked: false,
            position: Some(bounds(x, y, 180.0, 120.0)),
            source: "assets/images/example.png".to_owned(),
            fit: ImageFit::Contain,
            rotation: 0.0,
        }),
        ElementKind::Line => Element::Line(LineElement {
            name: None,
            visible: true,
            locked: false,
            x1: Length::points(x),
            y1: Length::points(y),
            x2: Length::points(x + 220.0),
            y2: Length::points(y),
            width: Length::points(1.0),
            color: "#20252A".to_owned(),
            dash: DashStyle::Solid,
        }),
        ElementKind::Svg => Element::Svg(SvgElement {
            name: None,
            visible: true,
            locked: false,
            position: Some(bounds(x, y, 144.0, 144.0)),
            source: "assets/images/example.svg".to_owned(),
            fit: ImageFit::Contain,
            rotation: 0.0,
        }),
        ElementKind::QrCode => Element::QrCode(QrCodeElement {
            name: None,
            visible: true,
            locked: false,
            position: Some(bounds(x, y, 108.0, 108.0)),
            value: "https://example.com".to_owned(),
            error_correction: QrErrorCorrection::Medium,
            quiet_zone: 4,
            color: "#20252A".to_owned(),
            background: "#FFFFFF".to_owned(),
            rotation: 0.0,
        }),
        ElementKind::Barcode => Element::Barcode(BarcodeElement {
            name: None,
            visible: true,
            locked: false,
            position: Some(bounds(x, y, 252.0, 72.0)),
            value: "PRINT-FORGE-001".to_owned(),
            format: BarcodeFormat::Code128,
            quiet_zone: 10,
            color: "#20252A".to_owned(),
            background: "#FFFFFF".to_owned(),
            rotation: 0.0,
        }),
        ElementKind::Group => Element::Group(GroupElement {
            name: None,
            visible: true,
            locked: false,
            position: Some(bounds(x, y, 252.0, 108.0)),
            children: vec![
                Element::Rectangle(RectangleElement {
                    name: Some("Group background".to_owned()),
                    visible: true,
                    locked: false,
                    position: Some(bounds(0.0, 0.0, 252.0, 108.0)),
                    fill: Some("#FFF2EB".to_owned()),
                    stroke: Some(Stroke {
                        width: Length::points(1.0),
                        color: "#F45B20".to_owned(),
                        dash: DashStyle::Solid,
                    }),
                    rotation: 0.0,
                }),
                Element::Text(TextElement {
                    name: Some("Group text".to_owned()),
                    visible: true,
                    locked: false,
                    position: Some(bounds(16.0, 42.0, 220.0, 24.0)),
                    value: "Reusable group".to_owned(),
                    font_size: Length::points(16.0),
                    font: None,
                    font_style: FontStyle::Bold,
                    line_height: None,
                    align: TextAlign::Center,
                    overflow: TextOverflow::Shrink,
                    min_font_size: Some(Length::points(9.0)),
                    color: "#20252A".to_owned(),
                    rotation: 0.0,
                }),
            ],
        }),
        ElementKind::Stack => Element::Stack(StackElement {
            name: None,
            visible: true,
            locked: false,
            position: Some(bounds(x, y - 108.0, 300.0, 216.0)),
            direction: StackDirection::Vertical,
            gap: Length::points(8.0),
            padding: Length::points(12.0),
            overflow: FlowOverflow::Paginate,
            keep_together: false,
            orphans: 1,
            children: vec![
                flow_text("First flow item", 276.0),
                flow_text("Second flow item", 276.0),
            ],
        }),
        ElementKind::Table => Element::Table(TableElement {
            name: None,
            visible: true,
            locked: false,
            position: Some(bounds(x, y - 108.0, 360.0, 216.0)),
            source: "items".to_owned(),
            header: true,
            font: None,
            font_size: Length::points(9.0),
            line_height: Some(Length::points(11.0)),
            color: "#20252A".to_owned(),
            header_font_style: FontStyle::Bold,
            cell_padding: Length::points(4.0),
            border: Some(Stroke {
                width: Length::points(0.5),
                color: "#9FB7C9".to_owned(),
                dash: DashStyle::Solid,
            }),
            header_background: Some("#E8EEF2".to_owned()),
            row_background: Some("#FFFFFF".to_owned()),
            alternate_row_background: Some("#F5F7F9".to_owned()),
            columns: vec![
                TableColumn {
                    field: "description".to_owned(),
                    header: "Description".to_owned(),
                    width: TableColumnWidth::Percent { value: 70.0 },
                    align: TextAlign::Left,
                    format: None,
                },
                TableColumn {
                    field: "value".to_owned(),
                    header: "Value".to_owned(),
                    width: TableColumnWidth::Percent { value: 30.0 },
                    align: TextAlign::Right,
                    format: None,
                },
            ],
        }),
        ElementKind::Repeater => Element::Repeater(RepeaterElement {
            name: None,
            visible: true,
            locked: false,
            source: "items".to_owned(),
            layout: RepeatLayout::Grid,
            template: Box::new(Element::Group(GroupElement {
                name: Some("Repeated item".to_owned()),
                visible: true,
                locked: false,
                position: Some(bounds(x, y, 144.0, 72.0)),
                children: vec![Element::Text(TextElement {
                    name: None,
                    visible: true,
                    locked: false,
                    position: Some(bounds(8.0, 24.0, 128.0, 20.0)),
                    value: "{{name}}".to_owned(),
                    font_size: Length::points(11.0),
                    font: None,
                    font_style: FontStyle::Bold,
                    line_height: None,
                    align: TextAlign::Center,
                    overflow: TextOverflow::Shrink,
                    min_font_size: Some(Length::points(7.0)),
                    color: "#20252A".to_owned(),
                    rotation: 0.0,
                })],
            })),
        }),
    }
}

fn flow_text(value: &str, width: f32) -> Element {
    Element::Text(TextElement {
        name: None,
        visible: true,
        locked: false,
        position: Some(bounds(0.0, 0.0, width, 36.0)),
        value: value.to_owned(),
        font_size: Length::points(11.0),
        font: None,
        font_style: FontStyle::Regular,
        line_height: Some(Length::points(14.0)),
        align: TextAlign::Left,
        overflow: TextOverflow::Shrink,
        min_font_size: Some(Length::points(7.0)),
        color: "#20252A".to_owned(),
        rotation: 0.0,
    })
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
    if let Some(name) = element.layer_name().filter(|name| !name.trim().is_empty()) {
        return name.to_owned();
    }
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
        Element::Repeater(value) => element_bounds(&value.template),
        Element::Line(_) | Element::PageBreak => None,
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
        Element::Repeater(value) => element_bounds_mut(&mut value.template),
        Element::Line(_) | Element::PageBreak => None,
    }
}

pub fn element_alignment_bounds(element: &Element) -> Option<[f32; 4]> {
    if let Element::Line(line) = element {
        let x1 = line.x1.to_points();
        let y1 = line.y1.to_points();
        let x2 = line.x2.to_points();
        let y2 = line.y2.to_points();
        return Some([x1.min(x2), y1.min(y2), (x2 - x1).abs(), (y2 - y1).abs()]);
    }
    let bounds = bounds_points(element_bounds(element)?);
    let rotation = element_rotation(element).unwrap_or(0.0).to_radians();
    if rotation.abs() < f32::EPSILON {
        return Some(bounds);
    }
    let half_width = bounds[2] / 2.0;
    let half_height = bounds[3] / 2.0;
    let rotated_half_width = rotation.cos().abs() * half_width + rotation.sin().abs() * half_height;
    let rotated_half_height =
        rotation.sin().abs() * half_width + rotation.cos().abs() * half_height;
    let center_x = bounds[0] + half_width;
    let center_y = bounds[1] + half_height;
    Some([
        center_x - rotated_half_width,
        center_y - rotated_half_height,
        rotated_half_width * 2.0,
        rotated_half_height * 2.0,
    ])
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

pub fn move_element(elements: &mut Vec<Element>, from: usize, to: usize) -> bool {
    if from >= elements.len() || to >= elements.len() || from == to {
        return false;
    }
    let element = elements.remove(from);
    elements.insert(to, element);
    true
}

pub fn align_elements(elements: &mut [Element], indices: &[usize], mode: AlignMode) -> bool {
    let items = selected_alignment_bounds(elements, indices);
    if items.len() < 2 {
        return false;
    }
    let left = items
        .iter()
        .map(|(_, bounds)| bounds[0])
        .fold(f32::INFINITY, f32::min);
    let right = items
        .iter()
        .map(|(_, bounds)| bounds[0] + bounds[2])
        .fold(f32::NEG_INFINITY, f32::max);
    let bottom = items
        .iter()
        .map(|(_, bounds)| bounds[1])
        .fold(f32::INFINITY, f32::min);
    let top = items
        .iter()
        .map(|(_, bounds)| bounds[1] + bounds[3])
        .fold(f32::NEG_INFINITY, f32::max);
    let horizontal_center = (left + right) / 2.0;
    let vertical_center = (bottom + top) / 2.0;

    for (index, bounds) in items {
        let (dx, dy) = match mode {
            AlignMode::Left => (left - bounds[0], 0.0),
            AlignMode::HorizontalCenter => (horizontal_center - (bounds[0] + bounds[2] / 2.0), 0.0),
            AlignMode::Right => (right - (bounds[0] + bounds[2]), 0.0),
            AlignMode::Bottom => (0.0, bottom - bounds[1]),
            AlignMode::VerticalCenter => (0.0, vertical_center - (bounds[1] + bounds[3] / 2.0)),
            AlignMode::Top => (0.0, top - (bounds[1] + bounds[3])),
        };
        translate_element(&mut elements[index], dx, dy);
    }
    true
}

pub fn distribute_elements(
    elements: &mut [Element],
    indices: &[usize],
    axis: DistributionAxis,
) -> bool {
    let mut items = selected_alignment_bounds(elements, indices);
    if items.len() < 3 {
        return false;
    }
    match axis {
        DistributionAxis::Horizontal => {
            items.sort_by(|left, right| left.1[0].total_cmp(&right.1[0]));
            let outer_start = items.first().unwrap().1[0];
            let outer_end = items.last().unwrap().1[0] + items.last().unwrap().1[2];
            let occupied = items.iter().map(|(_, bounds)| bounds[2]).sum::<f32>();
            let gap = (outer_end - outer_start - occupied) / (items.len() - 1) as f32;
            let mut cursor = outer_start;
            for (index, bounds) in items {
                translate_element(&mut elements[index], cursor - bounds[0], 0.0);
                cursor += bounds[2] + gap;
            }
        }
        DistributionAxis::Vertical => {
            items.sort_by(|left, right| left.1[1].total_cmp(&right.1[1]));
            let outer_start = items.first().unwrap().1[1];
            let outer_end = items.last().unwrap().1[1] + items.last().unwrap().1[3];
            let occupied = items.iter().map(|(_, bounds)| bounds[3]).sum::<f32>();
            let gap = (outer_end - outer_start - occupied) / (items.len() - 1) as f32;
            let mut cursor = outer_start;
            for (index, bounds) in items {
                translate_element(&mut elements[index], 0.0, cursor - bounds[1]);
                cursor += bounds[3] + gap;
            }
        }
    }
    true
}

fn selected_alignment_bounds(elements: &[Element], indices: &[usize]) -> Vec<(usize, [f32; 4])> {
    indices
        .iter()
        .copied()
        .filter_map(|index| {
            elements
                .get(index)
                .and_then(element_alignment_bounds)
                .map(|bounds| (index, bounds))
        })
        .collect()
}

pub fn snap_translation(
    bounds: [f32; 4],
    delta: [f32; 2],
    x_targets: &[f32],
    y_targets: &[f32],
    threshold: f32,
) -> SnapResult {
    let x_points = [
        bounds[0] + delta[0],
        bounds[0] + bounds[2] / 2.0 + delta[0],
        bounds[0] + bounds[2] + delta[0],
    ];
    let y_points = [
        bounds[1] + delta[1],
        bounds[1] + bounds[3] / 2.0 + delta[1],
        bounds[1] + bounds[3] + delta[1],
    ];
    let x_snap = closest_snap(&x_points, x_targets, threshold);
    let y_snap = closest_snap(&y_points, y_targets, threshold);
    SnapResult {
        delta: [
            delta[0] + x_snap.map_or(0.0, |snap| snap.0),
            delta[1] + y_snap.map_or(0.0, |snap| snap.0),
        ],
        x: x_snap.map(|snap| snap.1),
        y: y_snap.map(|snap| snap.1),
    }
}

pub fn snap_point(
    point: [f32; 2],
    delta: [f32; 2],
    x_targets: &[f32],
    y_targets: &[f32],
    threshold: f32,
) -> SnapResult {
    snap_translation(
        [point[0], point[1], 0.0, 0.0],
        delta,
        x_targets,
        y_targets,
        threshold,
    )
}

pub fn snap_size(
    bounds: [f32; 4],
    size_delta: [f32; 2],
    x_targets: &[f32],
    y_targets: &[f32],
    threshold: f32,
) -> SnapResult {
    let right = bounds[0] + bounds[2] + size_delta[0];
    let top = bounds[1] + bounds[3] + size_delta[1];
    let valid_x_targets = x_targets
        .iter()
        .copied()
        .filter(|target| *target >= bounds[0] + 1.0)
        .collect::<Vec<_>>();
    let valid_y_targets = y_targets
        .iter()
        .copied()
        .filter(|target| *target >= bounds[1] + 1.0)
        .collect::<Vec<_>>();
    let x_snap = closest_snap(&[right], &valid_x_targets, threshold);
    let y_snap = closest_snap(&[top], &valid_y_targets, threshold);
    SnapResult {
        delta: [
            size_delta[0] + x_snap.map_or(0.0, |snap| snap.0),
            size_delta[1] + y_snap.map_or(0.0, |snap| snap.0),
        ],
        x: x_snap.map(|snap| snap.1),
        y: y_snap.map(|snap| snap.1),
    }
}

fn closest_snap(points: &[f32], targets: &[f32], threshold: f32) -> Option<(f32, f32)> {
    points
        .iter()
        .flat_map(|point| {
            targets
                .iter()
                .map(move |target| (*target - *point, *target))
        })
        .filter(|(correction, _)| correction.abs() <= threshold)
        .min_by(|left, right| left.0.abs().total_cmp(&right.0.abs()))
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
        AlignMode, DistributionAxis, ElementKind, LayerMove, LineEndpoint, align_elements,
        bounds_points, distribute_elements, element_alignment_bounds, element_bounds,
        element_rotation, move_element, new_element, reorder_element, set_element_rotation,
        snap_point, snap_size, snap_translation, starter_template, translate_element,
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
        for kind in ElementKind::BASIC.into_iter().chain(ElementKind::ADVANCED) {
            let mut element = new_element(kind, 0.0);
            let original = element_bounds(&element).map(bounds_points);
            translate_element(&mut element, 10.0, -5.0);
            if let Some(original) = original {
                let bounds = bounds_points(element_bounds(&element).unwrap());
                assert_eq!(bounds[0], original[0] + 10.0);
                assert_eq!(bounds[1], original[1] - 5.0);
            }
        }
    }

    #[test]
    fn advanced_palette_elements_round_trip() {
        for kind in ElementKind::ADVANCED {
            let element = new_element(kind, 0.0);
            let json = serde_json::to_string_pretty(&element).unwrap();
            let decoded = serde_json::from_str(&json).unwrap();
            assert_eq!(element, decoded, "{} should round trip", kind.label());
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

    #[test]
    fn layers_can_drag_to_an_exact_paint_position() {
        let mut elements = vec![
            new_element(ElementKind::Text, 0.0),
            new_element(ElementKind::Rectangle, 0.0),
            new_element(ElementKind::Svg, 0.0),
        ];

        assert!(move_element(&mut elements, 0, 2));
        assert!(matches!(elements[2], Element::Text(_)));
        assert!(!move_element(&mut elements, 2, 2));
    }

    #[test]
    fn selected_elements_align_by_their_visible_bounds() {
        let mut elements = vec![
            new_element(ElementKind::Rectangle, 0.0),
            new_element(ElementKind::Rectangle, 20.0),
        ];

        assert!(align_elements(&mut elements, &[0, 1], AlignMode::Left));
        let first = element_alignment_bounds(&elements[0]).unwrap();
        let second = element_alignment_bounds(&elements[1]).unwrap();
        assert!((first[0] - second[0]).abs() < 0.001);

        assert!(align_elements(&mut elements, &[0, 1], AlignMode::Top));
        let first = element_alignment_bounds(&elements[0]).unwrap();
        let second = element_alignment_bounds(&elements[1]).unwrap();
        assert!((first[1] + first[3] - second[1] - second[3]).abs() < 0.001);
    }

    #[test]
    fn selected_elements_distribute_with_equal_gaps() {
        let mut elements = vec![
            new_element(ElementKind::Rectangle, 0.0),
            new_element(ElementKind::Rectangle, 40.0),
            new_element(ElementKind::Rectangle, 100.0),
        ];

        assert!(distribute_elements(
            &mut elements,
            &[0, 1, 2],
            DistributionAxis::Horizontal
        ));
        let bounds = elements
            .iter()
            .map(|element| element_alignment_bounds(element).unwrap())
            .collect::<Vec<_>>();
        let first_gap = bounds[1][0] - (bounds[0][0] + bounds[0][2]);
        let second_gap = bounds[2][0] - (bounds[1][0] + bounds[1][2]);
        assert!((first_gap - second_gap).abs() < 0.001);
    }

    #[test]
    fn rotated_elements_snap_using_their_visible_alignment_bounds() {
        let mut rectangle = new_element(ElementKind::Rectangle, 0.0);
        set_element_rotation(&mut rectangle, 90.0);

        let bounds = element_alignment_bounds(&rectangle).unwrap();
        assert!((bounds[2] - 90.0).abs() < 0.001);
        assert!((bounds[3] - 180.0).abs() < 0.001);
    }

    #[test]
    fn translation_snaps_edges_and_centers_to_the_closest_target() {
        let snapped = snap_translation(
            [10.0, 20.0, 30.0, 40.0],
            [7.0, 8.0],
            &[0.0, 50.0, 100.0],
            &[0.0, 50.0, 100.0],
            4.0,
        );

        assert_eq!(snapped.delta, [10.0, 10.0]);
        assert_eq!(snapped.x, Some(50.0));
        assert_eq!(snapped.y, Some(50.0));
    }

    #[test]
    fn points_and_resize_handles_snap_independently() {
        let point = snap_point([10.0, 10.0], [7.0, 38.0], &[20.0], &[50.0], 3.0);
        assert_eq!(point.delta, [10.0, 40.0]);

        let size = snap_size(
            [10.0, 10.0, 20.0, 20.0],
            [18.0, 17.0],
            &[50.0],
            &[50.0],
            3.0,
        );
        assert_eq!(size.delta, [20.0, 20.0]);
    }
}
