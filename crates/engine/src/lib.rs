//! Layout contracts shared by template compilers and document renderers.

use barcoders::sym::code128::Code128;
use print_forge_dataset::DataRow;
use qrcodegen::{QrCode, QrCodeEcc};
use std::{
    fs,
    path::{Path, PathBuf},
};

use print_forge_template::{
    BarcodeElement, BarcodeFormat, Color, DashStyle, DocumentMetadata, Element, FlowOverflow,
    FontFamily, FontStyle, GroupElement, ImageFit, QrCodeElement, QrErrorCorrection, RepeatLayout,
    RepeaterElement, StackDirection, StackElement, Stroke, TableColumnWidth, TableDateStyle,
    TableElement, TableValueFormat, Template, TextAlign, TextOverflow,
};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedDocument {
    pub title: String,
    pub width_pt: f32,
    pub height_pt: f32,
    pub bleed_pt: f32,
    pub metadata: DocumentMetadata,
    pub pages: Vec<ResolvedPage>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResolvedPage {
    pub commands: Vec<ResolvedCommand>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedCommand {
    pub source_path: String,
    /// Clockwise rotation around the command bounds center, in degrees.
    pub rotation: f32,
    pub command: DrawCommand,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DrawCommand {
    Text(TextCommand),
    Image(ImageCommand),
    Rectangle(RectangleCommand),
    Line(LineCommand),
    Svg(SvgCommand),
    QrCode(QrCodeCommand),
    Barcode(BarcodeCommand),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextCommand {
    pub bounds: Rect,
    pub lines: Vec<TextLine>,
    pub font_size_pt: f32,
    pub line_height_pt: f32,
    pub font: ResolvedFont,
    pub color: Color,
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
    pub fill: Option<Color>,
    pub stroke: Option<StrokeCommand>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LineCommand {
    pub start: Point,
    pub end: Point,
    pub width_pt: f32,
    pub color: Color,
    pub dash: LineDash,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SvgCommand {
    pub bounds: Rect,
    pub source: PathBuf,
    pub fit: ImageFit,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QrCodeCommand {
    pub bounds: Rect,
    pub size: usize,
    pub modules: Vec<bool>,
    pub quiet_zone: u8,
    pub color: Color,
    pub background: Color,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BarcodeCommand {
    pub bounds: Rect,
    pub modules: Vec<bool>,
    pub quiet_zone: u8,
    pub color: Color,
    pub background: Color,
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

/// The available space supplied to an element during flow measurement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeasureConstraints {
    pub max_width: f32,
    pub max_height: f32,
}

/// The space requested by an element before final coordinates are assigned.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeasuredSize {
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StrokeCommand {
    pub width_pt: f32,
    pub color: Color,
    pub dash: LineDash,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum LineDash {
    #[default]
    Solid,
    Dashed,
    Dotted,
}

/// Renderer-neutral layout boundary for applications that provide a custom engine.
pub trait LayoutEngine {
    fn layout(&self, template: &Template, data: &DataRow) -> Result<ResolvedDocument, LayoutError>;

    fn layout_with_options(
        &self,
        template: &Template,
        data: &DataRow,
        _options: &LayoutOptions,
    ) -> Result<ResolvedDocument, LayoutError> {
        self.layout(template, data)
    }
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

/// Renderer-independent absolute and flow layout engine.
#[derive(Debug, Default, Clone, Copy)]
pub struct BasicLayoutEngine;

impl LayoutEngine for BasicLayoutEngine {
    fn layout(&self, template: &Template, data: &DataRow) -> Result<ResolvedDocument, LayoutError> {
        self.layout_with_options(template, data, &LayoutOptions::default())
    }

    fn layout_with_options(
        &self,
        template: &Template,
        data: &DataRow,
        options: &LayoutOptions,
    ) -> Result<ResolvedDocument, LayoutError> {
        BasicLayoutEngine::layout_with_options(self, template, data, options)
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

        let mut pages = Vec::new();
        for (page_index, page) in template.pages.iter().enumerate() {
            pages.extend(layout_template_page(
                page_index, page, template, data, options,
            )?);
        }

        Ok(ResolvedDocument {
            title: template.name.clone(),
            width_pt,
            height_pt,
            bleed_pt: template
                .document
                .bleed
                .map_or(0.0, |bleed| bleed.to_points()),
            metadata: template.document.metadata.clone(),
            pages,
        })
    }
}

fn layout_template_page(
    page_index: usize,
    page: &print_forge_template::Page,
    template: &Template,
    data: &DataRow,
    options: &LayoutOptions,
) -> Result<Vec<ResolvedPage>, LayoutError> {
    let repeating = layout_repeating_elements(page_index, page, template, data, options)?;
    let mut pages = vec![ResolvedPage {
        commands: repeating.clone(),
    }];

    for (element_index, element) in page.elements.iter().enumerate() {
        let source_path = format!("pages[{page_index}].elements[{element_index}]");
        match element {
            Element::PageBreak => pages.push(ResolvedPage {
                commands: repeating.clone(),
            }),
            Element::Stack(stack) => {
                let flow_pages = layout_flow_stack(stack, &source_path, template, data, options)
                    .map_err(|source| layout_error(page_index, &source_path, source))?;
                for (continuation, commands) in flow_pages.into_iter().enumerate() {
                    if continuation > 0 {
                        pages.push(ResolvedPage {
                            commands: repeating.clone(),
                        });
                    }
                    pages
                        .last_mut()
                        .expect("page exists")
                        .commands
                        .extend(commands);
                }
            }
            Element::Table(table) => {
                let table_pages = layout_table(table, &source_path, template, data, options)
                    .map_err(|source| layout_error(page_index, &source_path, source))?;
                for (continuation, commands) in table_pages.into_iter().enumerate() {
                    if continuation > 0 {
                        pages.push(ResolvedPage {
                            commands: repeating.clone(),
                        });
                    }
                    pages
                        .last_mut()
                        .expect("page exists")
                        .commands
                        .extend(commands);
                }
            }
            Element::Group(group) => {
                let commands = layout_group(group, None, &source_path, template, data, options)
                    .map_err(|source| layout_error(page_index, &source_path, source))?;
                pages
                    .last_mut()
                    .expect("page exists")
                    .commands
                    .extend(commands);
            }
            Element::Repeater(repeater) => {
                let repeat_pages = layout_repeater(
                    repeater,
                    &source_path,
                    template,
                    data,
                    options,
                    template.document.width.to_points(),
                    template.document.height.to_points(),
                )
                .map_err(|source| layout_error(page_index, &source_path, source))?;
                for (continuation, commands) in repeat_pages.into_iter().enumerate() {
                    if continuation > 0 {
                        pages.push(ResolvedPage {
                            commands: repeating.clone(),
                        });
                    }
                    pages
                        .last_mut()
                        .expect("page exists")
                        .commands
                        .extend(commands);
                }
            }
            _ => {
                let command = layout_element_at(element, None, template, data, options)
                    .map_err(|source| layout_error(page_index, &source_path, source))?;
                pages
                    .last_mut()
                    .expect("page exists")
                    .commands
                    .push(ResolvedCommand {
                        source_path,
                        rotation: element_rotation(element),
                        command,
                    });
            }
        }
    }

    Ok(pages)
}

fn layout_repeating_elements(
    page_index: usize,
    page: &print_forge_template::Page,
    template: &Template,
    data: &DataRow,
    options: &LayoutOptions,
) -> Result<Vec<ResolvedCommand>, LayoutError> {
    let mut commands = Vec::new();
    for (section, elements) in [("header", &page.header), ("footer", &page.footer)] {
        for (element_index, element) in elements.iter().enumerate() {
            let source_path = format!("pages[{page_index}].{section}[{element_index}]");
            match element {
                Element::PageBreak => {
                    return Err(layout_error(
                        page_index,
                        &source_path,
                        ElementLayoutError::InvalidLayout(
                            "page breaks are not allowed in repeating headers or footers"
                                .to_owned(),
                        ),
                    ));
                }
                Element::Stack(stack) => {
                    let mut flow_pages =
                        layout_flow_stack(stack, &source_path, template, data, options)
                            .map_err(|source| layout_error(page_index, &source_path, source))?;
                    if flow_pages.len() != 1 {
                        return Err(layout_error(
                            page_index,
                            &source_path,
                            ElementLayoutError::InvalidLayout(
                                "repeating headers and footers cannot paginate".to_owned(),
                            ),
                        ));
                    }
                    commands.append(&mut flow_pages[0]);
                }
                Element::Group(group) => {
                    commands.extend(
                        layout_group(group, None, &source_path, template, data, options)
                            .map_err(|source| layout_error(page_index, &source_path, source))?,
                    );
                }
                Element::Repeater(repeater) => {
                    let mut repeat_pages = layout_repeater(
                        repeater,
                        &source_path,
                        template,
                        data,
                        options,
                        template.document.width.to_points(),
                        template.document.height.to_points(),
                    )
                    .map_err(|source| layout_error(page_index, &source_path, source))?;
                    if repeat_pages.len() != 1 {
                        return Err(layout_error(
                            page_index,
                            &source_path,
                            ElementLayoutError::InvalidLayout(
                                "repeaters in headers and footers cannot paginate".to_owned(),
                            ),
                        ));
                    }
                    commands.append(&mut repeat_pages[0]);
                }
                _ => {
                    let command = layout_element_at(element, None, template, data, options)
                        .map_err(|source| layout_error(page_index, &source_path, source))?;
                    commands.push(ResolvedCommand {
                        source_path,
                        rotation: element_rotation(element),
                        command,
                    });
                }
            }
        }
    }
    Ok(commands)
}

fn layout_error(page_index: usize, element_path: &str, source: ElementLayoutError) -> LayoutError {
    LayoutError::Element {
        page_index,
        element_path: element_path.to_owned(),
        source,
    }
}

fn layout_group(
    group: &GroupElement,
    assigned_bounds: Option<Rect>,
    source_path: &str,
    template: &Template,
    data: &DataRow,
    options: &LayoutOptions,
) -> Result<Vec<ResolvedCommand>, ElementLayoutError> {
    let group_bounds = assigned_bounds
        .or_else(|| group.position.as_ref().map(resolve_bounds))
        .unwrap_or(Rect {
            x: 0.0,
            y: 0.0,
            width: template.document.width.to_points(),
            height: template.document.height.to_points(),
        });
    validate_flow_rect(group_bounds, "group bounds")?;

    let mut commands = Vec::new();
    for (index, child) in group.children.iter().enumerate() {
        let child_path = format!("{source_path}.children[{index}]");
        let child_commands = match child {
            Element::Group(nested) => {
                let bounds = nested
                    .position
                    .as_ref()
                    .map(|position| translate_bounds(resolve_bounds(position), group_bounds))
                    .unwrap_or(group_bounds);
                layout_group(
                    nested,
                    Some(bounds),
                    &child_path,
                    template,
                    data,
                    options,
                )
            }
            Element::Stack(stack) => {
                let bounds = stack
                    .position
                    .as_ref()
                    .map(|position| translate_bounds(resolve_bounds(position), group_bounds))
                    .ok_or_else(|| missing_position("stack"))?;
                layout_stack_in_bounds(
                    stack,
                    bounds,
                    &child_path,
                    template,
                    data,
                    options,
                )
            }
            Element::Line(line) => Ok(vec![ResolvedCommand {
                source_path: child_path.clone(),
                rotation: 0.0,
                command: DrawCommand::Line(LineCommand {
                    start: Point {
                        x: group_bounds.x + line.x1.to_points(),
                        y: group_bounds.y + line.y1.to_points(),
                    },
                    end: Point {
                        x: group_bounds.x + line.x2.to_points(),
                        y: group_bounds.y + line.y2.to_points(),
                    },
                    width_pt: line.width.to_points(),
                    color: resolve_color(&line.color)?,
                    dash: resolve_dash(line.dash),
                }),
            }]),
            Element::Table(_) => Err(ElementLayoutError::InvalidLayout(
                "MVP tables must be positioned top-level page elements".to_owned(),
            )),
            Element::Repeater(_) => Err(ElementLayoutError::InvalidLayout(
                "nested repeaters are not supported; use a dotted source path for nested JSON arrays"
                    .to_owned(),
            )),
            Element::PageBreak => Err(ElementLayoutError::InvalidLayout(
                "page breaks are not allowed inside groups".to_owned(),
            )),
            _ => {
                let bounds = element_bounds(child)
                    .map(|position| translate_bounds(resolve_bounds(position), group_bounds))
                    .ok_or_else(|| {
                        ElementLayoutError::InvalidLayout(
                            "group children require a position".to_owned(),
                        )
                    })?;
                Ok(vec![ResolvedCommand {
                    source_path: child_path.clone(),
                    rotation: element_rotation(child),
                    command: layout_element_at(child, Some(bounds), template, data, options)?,
                }])
            }
        }
        .map_err(|source| nested_layout_error(&child_path, source))?;
        commands.extend(child_commands);
    }

    Ok(commands)
}

#[allow(clippy::too_many_arguments)]
fn layout_repeater(
    repeater: &RepeaterElement,
    source_path: &str,
    template: &Template,
    data: &DataRow,
    options: &LayoutOptions,
    page_width: f32,
    page_height: f32,
) -> Result<Vec<Vec<ResolvedCommand>>, ElementLayoutError> {
    let value = lookup_value(data, &repeater.source)
        .ok_or_else(|| ElementLayoutError::MissingVariable(repeater.source.clone()))?;
    let items = value.as_array().ok_or_else(|| {
        ElementLayoutError::InvalidLayout(format!(
            "repeater source {:?} must resolve to an array",
            repeater.source
        ))
    })?;

    let start = element_bounds(&repeater.template)
        .map(resolve_bounds)
        .ok_or_else(|| {
            ElementLayoutError::InvalidLayout(
                "repeater template requires position bounds that define the first item slot"
                    .to_owned(),
            )
        })?;
    validate_flow_rect(start, "repeater item bounds")?;
    if start.x < 0.0
        || start.y < 0.0
        || start.x + start.width > page_width + 0.01
        || start.y + start.height > page_height + 0.01
    {
        return Err(ElementLayoutError::InvalidLayout(
            "repeater template's first item slot must fit inside the page".to_owned(),
        ));
    }

    let columns = ((page_width - start.x) / start.width).floor() as usize;
    let rows = ((start.y + start.height) / start.height).floor() as usize;
    let (columns, rows) = match repeater.layout {
        RepeatLayout::Vertical => (1, rows),
        RepeatLayout::Horizontal => (columns, 1),
        RepeatLayout::Grid => (columns, rows),
    };
    if columns == 0 || rows == 0 {
        return Err(ElementLayoutError::InvalidLayout(
            "repeater item bounds leave no usable slots on the page".to_owned(),
        ));
    }
    let capacity = columns.checked_mul(rows).ok_or_else(|| {
        ElementLayoutError::InvalidLayout("repeater page capacity is too large".to_owned())
    })?;

    let mut pages = vec![Vec::new()];
    for (item_index, item) in items.iter().enumerate() {
        let page_index = item_index / capacity;
        while pages.len() <= page_index {
            pages.push(Vec::new());
        }
        let slot = item_index % capacity;
        let column = slot % columns;
        let row = slot / columns;
        let bounds = Rect {
            x: start.x + column as f32 * start.width,
            y: start.y - row as f32 * start.height,
            width: start.width,
            height: start.height,
        };
        let item_path = format!("{source_path}.items[{item_index}].template");
        let scope = repeat_item_scope(data, item, item_index);
        pages[page_index].extend(
            layout_repeated_element(
                &repeater.template,
                bounds,
                &item_path,
                template,
                &scope,
                options,
            )
            .map_err(|source| nested_layout_error(&item_path, source))?,
        );
    }

    Ok(pages)
}

fn layout_repeated_element(
    element: &Element,
    bounds: Rect,
    source_path: &str,
    template: &Template,
    data: &DataRow,
    options: &LayoutOptions,
) -> Result<Vec<ResolvedCommand>, ElementLayoutError> {
    match element {
        Element::Group(group) => {
            layout_group(group, Some(bounds), source_path, template, data, options)
        }
        Element::Stack(stack) => {
            layout_stack_in_bounds(stack, bounds, source_path, template, data, options)
        }
        Element::Line(_) | Element::Table(_) | Element::Repeater(_) | Element::PageBreak => {
            Err(ElementLayoutError::InvalidLayout(
                "repeater templates must be positioned text, image, rectangle, SVG, QR code, group, or stack elements"
                    .to_owned(),
            ))
        }
        _ => Ok(vec![ResolvedCommand {
            source_path: source_path.to_owned(),
            rotation: element_rotation(element),
            command: layout_element_at(element, Some(bounds), template, data, options)?,
        }]),
    }
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

fn element_bounds(element: &Element) -> Option<&print_forge_template::Bounds> {
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

fn element_rotation(element: &Element) -> f32 {
    match element {
        Element::Text(element) => element.rotation,
        Element::Image(element) => element.rotation,
        Element::Rectangle(element) => element.rotation,
        Element::Svg(element) => element.rotation,
        Element::QrCode(element) => element.rotation,
        Element::Barcode(element) => element.rotation,
        Element::Line(_)
        | Element::Group(_)
        | Element::Stack(_)
        | Element::Table(_)
        | Element::Repeater(_)
        | Element::PageBreak => 0.0,
    }
}

fn translate_bounds(bounds: Rect, parent: Rect) -> Rect {
    Rect {
        x: parent.x + bounds.x,
        y: parent.y + bounds.y,
        ..bounds
    }
}

fn nested_layout_error(element_path: &str, source: ElementLayoutError) -> ElementLayoutError {
    ElementLayoutError::Nested {
        element_path: element_path.to_owned(),
        source: Box::new(source),
    }
}

fn layout_flow_stack(
    stack: &StackElement,
    source_path: &str,
    template: &Template,
    data: &DataRow,
    options: &LayoutOptions,
) -> Result<Vec<Vec<ResolvedCommand>>, ElementLayoutError> {
    validate_stack_options(stack)?;
    let bounds = stack
        .position
        .as_ref()
        .map(resolve_bounds)
        .ok_or_else(|| missing_position("stack"))?;
    validate_flow_rect(bounds, "stack bounds")?;

    if stack.keep_together {
        if stack
            .children
            .iter()
            .any(|element| matches!(element, Element::PageBreak))
        {
            return Err(ElementLayoutError::InvalidLayout(
                "a keep-together stack cannot contain an explicit page break".to_owned(),
            ));
        }
        return layout_stack_in_bounds(stack, bounds, source_path, template, data, options)
            .map(|commands| vec![commands]);
    }

    match stack.direction {
        StackDirection::Vertical => {
            paginate_vertical_stack(stack, bounds, source_path, template, data, options)
        }
        StackDirection::Horizontal => {
            layout_stack_in_bounds(stack, bounds, source_path, template, data, options)
                .map(|commands| vec![commands])
        }
    }
}

fn paginate_vertical_stack(
    stack: &StackElement,
    bounds: Rect,
    source_path: &str,
    template: &Template,
    data: &DataRow,
    options: &LayoutOptions,
) -> Result<Vec<Vec<ResolvedCommand>>, ElementLayoutError> {
    let content = inset_rect(bounds, stack.padding.to_points())?;
    let constraints = MeasureConstraints {
        max_width: content.width,
        max_height: content.height,
    };
    let gap = stack.gap.to_points();
    let mut pages = vec![Vec::new()];
    let mut cursor_top = content.y + content.height;
    let mut items_on_page = 0_usize;

    for (index, child) in stack.children.iter().enumerate() {
        let child_path = format!("{source_path}.children[{index}]");
        if matches!(child, Element::PageBreak) {
            pages.push(Vec::new());
            cursor_top = content.y + content.height;
            items_on_page = 0;
            continue;
        }

        let size = measure_element(
            child,
            constraints,
            StackDirection::Vertical,
            template,
            data,
            options,
        )?;
        ensure_measured_size_fits(size, constraints, &child_path)?;
        let required = size.height + if items_on_page == 0 { 0.0 } else { gap };
        let remaining = cursor_top - content.y;

        if required > remaining + 0.01 {
            if stack.overflow == FlowOverflow::Error {
                return Err(ElementLayoutError::InvalidLayout(format!(
                    "{child_path} exceeds the stack region and overflow is set to error"
                )));
            }
            if items_on_page < stack.orphans {
                return Err(ElementLayoutError::InvalidLayout(format!(
                    "automatic page break would leave {items_on_page} flow item(s) before the break; stack requires at least {} orphan item(s)",
                    stack.orphans
                )));
            }
            pages.push(Vec::new());
            cursor_top = content.y + content.height;
            items_on_page = 0;
        }

        if items_on_page > 0 {
            cursor_top -= gap;
        }
        let child_bounds = Rect {
            x: content.x,
            y: cursor_top - size.height,
            width: size.width,
            height: size.height,
        };
        let mut commands =
            layout_flow_element(child, child_bounds, &child_path, template, data, options)?;
        pages.last_mut().expect("page exists").append(&mut commands);
        cursor_top -= size.height;
        items_on_page += 1;
    }

    Ok(pages)
}

fn layout_stack_in_bounds(
    stack: &StackElement,
    bounds: Rect,
    source_path: &str,
    template: &Template,
    data: &DataRow,
    options: &LayoutOptions,
) -> Result<Vec<ResolvedCommand>, ElementLayoutError> {
    validate_stack_options(stack)?;
    let content = inset_rect(bounds, stack.padding.to_points())?;
    let gap = stack.gap.to_points();
    let mut commands = Vec::new();

    match stack.direction {
        StackDirection::Vertical => {
            let constraints = MeasureConstraints {
                max_width: content.width,
                max_height: content.height,
            };
            let mut cursor_top = content.y + content.height;
            for (index, child) in stack.children.iter().enumerate() {
                let child_path = format!("{source_path}.children[{index}]");
                if matches!(child, Element::PageBreak) {
                    return Err(ElementLayoutError::InvalidLayout(
                        "page breaks are only supported by paginating vertical stacks".to_owned(),
                    ));
                }
                let size = measure_element(
                    child,
                    constraints,
                    StackDirection::Vertical,
                    template,
                    data,
                    options,
                )?;
                ensure_measured_size_fits(size, constraints, &child_path)?;
                if index > 0 {
                    cursor_top -= gap;
                }
                if cursor_top - size.height < content.y - 0.01 {
                    return Err(ElementLayoutError::InvalidLayout(format!(
                        "{child_path} exceeds its non-paginating stack bounds"
                    )));
                }
                let child_bounds = Rect {
                    x: content.x,
                    y: cursor_top - size.height,
                    width: size.width,
                    height: size.height,
                };
                commands.extend(layout_flow_element(
                    child,
                    child_bounds,
                    &child_path,
                    template,
                    data,
                    options,
                )?);
                cursor_top -= size.height;
            }
        }
        StackDirection::Horizontal => {
            let mut cursor_x = content.x;
            for (index, child) in stack.children.iter().enumerate() {
                let child_path = format!("{source_path}.children[{index}]");
                if matches!(child, Element::PageBreak) {
                    return Err(ElementLayoutError::InvalidLayout(
                        "page breaks are not valid in horizontal stacks".to_owned(),
                    ));
                }
                if index > 0 {
                    cursor_x += gap;
                }
                let constraints = MeasureConstraints {
                    max_width: (content.x + content.width - cursor_x).max(0.0),
                    max_height: content.height,
                };
                let size = measure_element(
                    child,
                    constraints,
                    StackDirection::Horizontal,
                    template,
                    data,
                    options,
                )?;
                ensure_measured_size_fits(size, constraints, &child_path)?;
                let child_bounds = Rect {
                    x: cursor_x,
                    y: content.y + content.height - size.height,
                    width: size.width,
                    height: size.height,
                };
                commands.extend(layout_flow_element(
                    child,
                    child_bounds,
                    &child_path,
                    template,
                    data,
                    options,
                )?);
                cursor_x += size.width;
            }
        }
    }

    Ok(commands)
}

fn layout_table(
    table: &TableElement,
    source_path: &str,
    template: &Template,
    data: &DataRow,
    options: &LayoutOptions,
) -> Result<Vec<Vec<ResolvedCommand>>, ElementLayoutError> {
    let bounds = table
        .position
        .as_ref()
        .map(resolve_bounds)
        .ok_or_else(|| missing_position("table"))?;
    validate_flow_rect(bounds, "table bounds")?;
    if table.columns.is_empty() {
        return Err(ElementLayoutError::InvalidLayout(
            "table must define at least one column".to_owned(),
        ));
    }

    let source = lookup_value(data, &table.source)
        .ok_or_else(|| ElementLayoutError::MissingVariable(table.source.clone()))?;
    let rows = source.as_array().ok_or_else(|| {
        ElementLayoutError::InvalidLayout(format!(
            "table source {:?} must resolve to an array",
            table.source
        ))
    })?;
    let column_widths = resolve_table_column_widths(table, bounds.width)?;
    let padding = table.cell_padding.to_points();
    if !padding.is_finite() || padding < 0.0 {
        return Err(ElementLayoutError::InvalidLayout(
            "table cell padding must be nonnegative and finite".to_owned(),
        ));
    }
    for (index, width) in column_widths.iter().enumerate() {
        if width - padding * 2.0 <= 0.0 {
            return Err(ElementLayoutError::InvalidLayout(format!(
                "table column {index} is too narrow for {padding:.2}pt cell padding"
            )));
        }
    }

    let font_size = table.font_size.to_points();
    let line_height = table
        .line_height
        .map_or(font_size * 1.2, |value| value.to_points());
    if !font_size.is_finite() || font_size <= 0.0 || !line_height.is_finite() || line_height <= 0.0
    {
        return Err(ElementLayoutError::InvalidLayout(
            "table font size and line height must be positive and finite".to_owned(),
        ));
    }
    let body_font = resolve_font(template, table.font.as_deref(), FontStyle::Regular, options)?;
    let header_font = resolve_font(
        template,
        table.font.as_deref(),
        table.header_font_style,
        options,
    )?;
    let body_metrics = FontMetrics::load(&body_font)?;
    let header_metrics = FontMetrics::load(&header_font)?;
    let text_color = resolve_color(&table.color)?;
    let header_background = table
        .header_background
        .as_deref()
        .map(resolve_color)
        .transpose()?;
    let row_background = table
        .row_background
        .as_deref()
        .map(resolve_color)
        .transpose()?;
    let alternate_background = table
        .alternate_row_background
        .as_deref()
        .map(resolve_color)
        .transpose()?;
    let border = table.border.as_ref().map(resolve_stroke).transpose()?;

    let formatted_rows = rows
        .iter()
        .enumerate()
        .map(|(row_index, value)| {
            let object = value.as_object().ok_or_else(|| {
                ElementLayoutError::InvalidLayout(format!(
                    "table source {:?} row {row_index} must be an object",
                    table.source
                ))
            })?;
            table
                .columns
                .iter()
                .enumerate()
                .map(|(column_index, column)| {
                    let value = lookup_table_cell(object, &column.field).ok_or_else(|| {
                        ElementLayoutError::InvalidLayout(format!(
                            "table row {row_index}, column {column_index} is missing field {:?}",
                            column.field
                        ))
                    })?;
                    format_table_value(value, column.format.as_ref()).map_err(|message| {
                        ElementLayoutError::InvalidLayout(format!(
                            "table row {row_index}, column {column_index}: {message}"
                        ))
                    })
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .collect::<Result<Vec<_>, _>>()?;

    let header_values = table
        .columns
        .iter()
        .map(|column| column.header.clone())
        .collect::<Vec<_>>();
    let header_height = table.header.then(|| {
        measure_table_row(
            &header_values,
            &column_widths,
            padding,
            font_size,
            line_height,
            &header_metrics,
        )
    });
    let row_heights = formatted_rows
        .iter()
        .map(|values| {
            measure_table_row(
                values,
                &column_widths,
                padding,
                font_size,
                line_height,
                &body_metrics,
            )
        })
        .collect::<Vec<_>>();

    let mut pages = Vec::new();
    let mut row_index = 0_usize;
    let mut first_page = true;
    while first_page || row_index < formatted_rows.len() {
        first_page = false;
        let page_index = pages.len();
        let page_top = bounds.y + bounds.height;
        let mut cursor_top = page_top;
        let mut boundaries = vec![page_top];
        let mut commands = Vec::new();

        if let Some(header_height) = header_height {
            if header_height > bounds.height + 0.01 {
                return Err(ElementLayoutError::InvalidLayout(format!(
                    "table header measures {header_height:.2}pt but the table region is only {:.2}pt high",
                    bounds.height
                )));
            }
            let row_bottom = cursor_top - header_height;
            append_table_row(
                &mut commands,
                &header_values,
                table,
                &column_widths,
                bounds.x,
                row_bottom,
                header_height,
                padding,
                font_size,
                line_height,
                &header_metrics,
                &header_font,
                text_color,
                header_background,
                &format!("{source_path}.pages[{page_index}].header"),
            )?;
            cursor_top = row_bottom;
            boundaries.push(cursor_top);
        }

        let first_row_on_page = row_index;
        while row_index < formatted_rows.len() {
            let row_height = row_heights[row_index];
            if cursor_top - row_height < bounds.y - 0.01 {
                break;
            }
            let row_bottom = cursor_top - row_height;
            let background = if row_index % 2 == 1 {
                alternate_background.or(row_background)
            } else {
                row_background
            };
            append_table_row(
                &mut commands,
                &formatted_rows[row_index],
                table,
                &column_widths,
                bounds.x,
                row_bottom,
                row_height,
                padding,
                font_size,
                line_height,
                &body_metrics,
                &body_font,
                text_color,
                background,
                &format!("{source_path}.rows[{row_index}]"),
            )?;
            cursor_top = row_bottom;
            boundaries.push(cursor_top);
            row_index += 1;
        }

        if row_index == first_row_on_page && row_index < formatted_rows.len() {
            return Err(ElementLayoutError::InvalidLayout(format!(
                "table row {row_index} measures {:.2}pt and cannot fit with the repeated header in the {:.2}pt table region",
                row_heights[row_index], bounds.height
            )));
        }

        if let Some(border) = &border
            && boundaries.len() > 1
        {
            append_table_grid(
                &mut commands,
                bounds.x,
                page_top,
                cursor_top,
                &boundaries,
                &column_widths,
                border,
                &format!("{source_path}.pages[{page_index}].grid"),
            );
        }
        pages.push(commands);
    }

    Ok(pages)
}

#[allow(clippy::too_many_arguments)]
fn append_table_row(
    commands: &mut Vec<ResolvedCommand>,
    values: &[String],
    table: &TableElement,
    column_widths: &[f32],
    table_x: f32,
    row_bottom: f32,
    row_height: f32,
    padding: f32,
    font_size: f32,
    line_height: f32,
    metrics: &FontMetrics,
    font: &ResolvedFont,
    text_color: Color,
    background: Option<Color>,
    source_path: &str,
) -> Result<(), ElementLayoutError> {
    let table_width = column_widths.iter().sum();
    if let Some(fill) = background {
        commands.push(ResolvedCommand {
            source_path: format!("{source_path}.background"),
            rotation: 0.0,
            command: DrawCommand::Rectangle(RectangleCommand {
                bounds: Rect {
                    x: table_x,
                    y: row_bottom,
                    width: table_width,
                    height: row_height,
                },
                fill: Some(fill),
                stroke: None,
            }),
        });
    }

    let mut cell_x = table_x;
    for (column_index, ((value, column), width)) in values
        .iter()
        .zip(&table.columns)
        .zip(column_widths)
        .enumerate()
    {
        let bounds = Rect {
            x: cell_x + padding,
            y: row_bottom + padding,
            width: width - padding * 2.0,
            height: row_height - padding * 2.0,
        };
        let laid_out = layout_text(
            value,
            TextLayoutSpec {
                bounds,
                requested_font_size: font_size,
                requested_line_height: line_height,
                min_font_size: font_size,
                align: column.align,
                overflow: TextOverflow::Error,
            },
            metrics,
        )?;
        commands.push(ResolvedCommand {
            source_path: format!("{source_path}.cells[{column_index}]"),
            rotation: 0.0,
            command: DrawCommand::Text(TextCommand {
                bounds,
                lines: laid_out.lines,
                font_size_pt: laid_out.font_size_pt,
                line_height_pt: laid_out.line_height_pt,
                font: font.clone(),
                color: text_color,
                clip: false,
            }),
        });
        cell_x += width;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn append_table_grid(
    commands: &mut Vec<ResolvedCommand>,
    table_x: f32,
    top: f32,
    bottom: f32,
    horizontal_boundaries: &[f32],
    column_widths: &[f32],
    border: &StrokeCommand,
    source_path: &str,
) {
    let table_width: f32 = column_widths.iter().sum();
    for (index, y) in horizontal_boundaries.iter().enumerate() {
        commands.push(ResolvedCommand {
            source_path: format!("{source_path}.horizontal[{index}]"),
            rotation: 0.0,
            command: DrawCommand::Line(LineCommand {
                start: Point { x: table_x, y: *y },
                end: Point {
                    x: table_x + table_width,
                    y: *y,
                },
                width_pt: border.width_pt,
                color: border.color,
                dash: border.dash,
            }),
        });
    }

    let mut x = table_x;
    for index in 0..=column_widths.len() {
        commands.push(ResolvedCommand {
            source_path: format!("{source_path}.vertical[{index}]"),
            rotation: 0.0,
            command: DrawCommand::Line(LineCommand {
                start: Point { x, y: top },
                end: Point { x, y: bottom },
                width_pt: border.width_pt,
                color: border.color,
                dash: border.dash,
            }),
        });
        if let Some(width) = column_widths.get(index) {
            x += width;
        }
    }
}

fn measure_table_row(
    values: &[String],
    column_widths: &[f32],
    padding: f32,
    font_size: f32,
    line_height: f32,
    metrics: &FontMetrics,
) -> f32 {
    values
        .iter()
        .zip(column_widths)
        .map(|(value, width)| {
            let lines = wrap_text(value, width - padding * 2.0, font_size, metrics);
            lines.len() as f32 * line_height + padding * 2.0
        })
        .fold(line_height + padding * 2.0, f32::max)
}

fn resolve_table_column_widths(
    table: &TableElement,
    table_width: f32,
) -> Result<Vec<f32>, ElementLayoutError> {
    let widths = table
        .columns
        .iter()
        .enumerate()
        .map(|(index, column)| {
            let width = match column.width {
                TableColumnWidth::Fixed { value } => value.to_points(),
                TableColumnWidth::Percent { value } => table_width * value / 100.0,
            };
            if !width.is_finite() || width <= 0.0 {
                return Err(ElementLayoutError::InvalidLayout(format!(
                    "table column {index} resolves to an invalid width"
                )));
            }
            Ok(width)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let resolved: f32 = widths.iter().sum();
    if (resolved - table_width).abs() > 0.1 {
        return Err(ElementLayoutError::InvalidLayout(format!(
            "resolved table columns total {resolved:.2}pt but the table is {table_width:.2}pt wide"
        )));
    }
    Ok(widths)
}

fn lookup_table_cell<'a>(
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

fn format_table_value(
    value: &serde_json::Value,
    format: Option<&TableValueFormat>,
) -> Result<String, String> {
    if value.is_array() || value.is_object() {
        return Err(
            "table cells must be scalar values; nested tables and arbitrary cell layouts are not supported"
                .to_owned(),
        );
    }
    let Some(format) = format else {
        return Ok(match value {
            serde_json::Value::Null => String::new(),
            serde_json::Value::String(value) => value.clone(),
            _ => value.to_string(),
        });
    };

    match format {
        TableValueFormat::Number { decimals } => {
            if *decimals > 12 {
                return Err("number formats support at most 12 decimal places".to_owned());
            }
            let number = table_number(value)?;
            Ok(format_number(number, *decimals))
        }
        TableValueFormat::Currency { symbol, decimals } => {
            if symbol.trim().is_empty() {
                return Err("currency symbol cannot be empty".to_owned());
            }
            if *decimals > 12 {
                return Err("currency formats support at most 12 decimal places".to_owned());
            }
            let number = table_number(value)?;
            let formatted = format_number(number, *decimals);
            Ok(if let Some(unsigned) = formatted.strip_prefix('-') {
                format!("-{symbol}{unsigned}")
            } else {
                format!("{symbol}{formatted}")
            })
        }
        TableValueFormat::Date { style } => {
            let value = value
                .as_str()
                .ok_or_else(|| "date format requires a YYYY-MM-DD string".to_owned())?;
            let (year, month, day) = parse_table_date(value)
                .ok_or_else(|| "date format requires a valid YYYY-MM-DD date".to_owned())?;
            Ok(match style {
                TableDateStyle::Iso => format!("{year:04}-{month:02}-{day:02}"),
                TableDateStyle::Us => format!("{month:02}/{day:02}/{year:04}"),
                TableDateStyle::European => format!("{day:02}/{month:02}/{year:04}"),
                TableDateStyle::Long => format!(
                    "{} {day}, {year:04}",
                    month_name(month).expect("validated month")
                ),
            })
        }
    }
}

fn table_number(value: &serde_json::Value) -> Result<f64, String> {
    let number = value
        .as_f64()
        .or_else(|| value.as_str().and_then(|value| value.parse::<f64>().ok()));
    number
        .filter(|number| number.is_finite())
        .ok_or_else(|| "number and currency formats require a finite numeric value".to_owned())
}

fn format_number(value: f64, decimals: u8) -> String {
    let precision = usize::from(decimals);
    let unsigned = format!("{:.*}", precision, value.abs());
    let (integer, fraction) = unsigned
        .split_once('.')
        .map_or((unsigned.as_str(), None), |(integer, fraction)| {
            (integer, Some(fraction))
        });
    let reversed = integer.chars().rev().collect::<Vec<_>>();
    let grouped = reversed
        .chunks(3)
        .map(|chunk| chunk.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join(",")
        .chars()
        .rev()
        .collect::<String>();
    let formatted = fraction.map_or(grouped.clone(), |fraction| format!("{grouped}.{fraction}"));
    if value.is_sign_negative() {
        format!("-{formatted}")
    } else {
        formatted
    }
}

fn parse_table_date(value: &str) -> Option<(i32, u32, u32)> {
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
        2 if is_table_leap_year(year) => 29,
        2 => 28,
        _ => return None,
    };
    (1..=max_day).contains(&day).then_some((year, month, day))
}

const fn is_table_leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

const fn month_name(month: u32) -> Option<&'static str> {
    match month {
        1 => Some("January"),
        2 => Some("February"),
        3 => Some("March"),
        4 => Some("April"),
        5 => Some("May"),
        6 => Some("June"),
        7 => Some("July"),
        8 => Some("August"),
        9 => Some("September"),
        10 => Some("October"),
        11 => Some("November"),
        12 => Some("December"),
        _ => None,
    }
}

fn measure_element(
    element: &Element,
    constraints: MeasureConstraints,
    parent_direction: StackDirection,
    template: &Template,
    data: &DataRow,
    options: &LayoutOptions,
) -> Result<MeasuredSize, ElementLayoutError> {
    validate_constraints(constraints)?;
    match element {
        Element::Text(text) => {
            if let Some(position) = &text.position {
                return Ok(measured_bounds(position));
            }
            if parent_direction == StackDirection::Horizontal {
                return Err(ElementLayoutError::InvalidLayout(
                    "horizontal stack children require position width and height size hints"
                        .to_owned(),
                ));
            }
            let value = resolve_string(&text.value, data)?;
            let font = resolve_font(template, text.font.as_deref(), text.font_style, options)?;
            let metrics = FontMetrics::load(&font)?;
            let font_size = text.font_size.to_points();
            let line_height = text
                .line_height
                .map_or(font_size * 1.2, |value| value.to_points());
            let lines = wrap_text(&value, constraints.max_width, font_size, &metrics);
            Ok(MeasuredSize {
                width: constraints.max_width,
                height: lines.len() as f32 * line_height,
            })
        }
        Element::Image(image) => image
            .position
            .as_ref()
            .map(measured_bounds)
            .ok_or_else(|| flow_size_hint_missing("image")),
        Element::Rectangle(rectangle) => rectangle
            .position
            .as_ref()
            .map(measured_bounds)
            .ok_or_else(|| flow_size_hint_missing("rectangle")),
        Element::Svg(svg) => svg
            .position
            .as_ref()
            .map(measured_bounds)
            .ok_or_else(|| flow_size_hint_missing("svg")),
        Element::QrCode(qr_code) => qr_code
            .position
            .as_ref()
            .map(measured_bounds)
            .ok_or_else(|| flow_size_hint_missing("qr_code")),
        Element::Barcode(barcode) => barcode
            .position
            .as_ref()
            .map(measured_bounds)
            .ok_or_else(|| flow_size_hint_missing("barcode")),
        Element::Stack(stack) => {
            if let Some(position) = &stack.position {
                Ok(measured_bounds(position))
            } else {
                measure_stack(stack, constraints, template, data, options)
            }
        }
        Element::Line(_) => Err(ElementLayoutError::InvalidLayout(
            "line elements use absolute coordinates and cannot be flow children".to_owned(),
        )),
        Element::Group(group) => group
            .position
            .as_ref()
            .map(measured_bounds)
            .ok_or_else(|| flow_size_hint_missing("group")),
        Element::Table(_) => Err(ElementLayoutError::InvalidLayout(
            "MVP tables must be positioned top-level page elements".to_owned(),
        )),
        Element::Repeater(_) => Err(ElementLayoutError::UnsupportedElement("repeater")),
        Element::PageBreak => Err(ElementLayoutError::InvalidLayout(
            "page breaks cannot be measured as flow items".to_owned(),
        )),
    }
}

fn measure_stack(
    stack: &StackElement,
    constraints: MeasureConstraints,
    template: &Template,
    data: &DataRow,
    options: &LayoutOptions,
) -> Result<MeasuredSize, ElementLayoutError> {
    validate_stack_options(stack)?;
    let padding = stack.padding.to_points();
    let inner = MeasureConstraints {
        max_width: constraints.max_width - padding * 2.0,
        max_height: constraints.max_height - padding * 2.0,
    };
    validate_constraints(inner)?;
    let gap = stack.gap.to_points();
    let mut width = 0.0_f32;
    let mut height = 0.0_f32;
    let mut count = 0_usize;

    for child in &stack.children {
        if matches!(child, Element::PageBreak) {
            return Err(ElementLayoutError::InvalidLayout(
                "nested stacks cannot contain page breaks".to_owned(),
            ));
        }
        let child_constraints = match stack.direction {
            StackDirection::Vertical => inner,
            StackDirection::Horizontal => MeasureConstraints {
                max_width: (inner.max_width - width - gap * count as f32).max(0.0),
                max_height: inner.max_height,
            },
        };
        let child_size = measure_element(
            child,
            child_constraints,
            stack.direction,
            template,
            data,
            options,
        )?;
        match stack.direction {
            StackDirection::Vertical => {
                width = width.max(child_size.width);
                height += child_size.height;
            }
            StackDirection::Horizontal => {
                width += child_size.width;
                height = height.max(child_size.height);
            }
        }
        count += 1;
    }
    if count > 1 {
        match stack.direction {
            StackDirection::Vertical => height += gap * (count - 1) as f32,
            StackDirection::Horizontal => width += gap * (count - 1) as f32,
        }
    }

    Ok(MeasuredSize {
        width: width + padding * 2.0,
        height: height + padding * 2.0,
    })
}

fn layout_flow_element(
    element: &Element,
    bounds: Rect,
    source_path: &str,
    template: &Template,
    data: &DataRow,
    options: &LayoutOptions,
) -> Result<Vec<ResolvedCommand>, ElementLayoutError> {
    match element {
        Element::Stack(stack) => {
            return layout_stack_in_bounds(stack, bounds, source_path, template, data, options);
        }
        Element::Group(group) => {
            return layout_group(group, Some(bounds), source_path, template, data, options);
        }
        _ => {}
    }
    let command = layout_element_at(element, Some(bounds), template, data, options)?;
    Ok(vec![ResolvedCommand {
        source_path: source_path.to_owned(),
        rotation: element_rotation(element),
        command,
    }])
}

fn measured_bounds(bounds: &print_forge_template::Bounds) -> MeasuredSize {
    MeasuredSize {
        width: bounds.width.to_points(),
        height: bounds.height.to_points(),
    }
}

fn inset_rect(bounds: Rect, padding: f32) -> Result<Rect, ElementLayoutError> {
    if !padding.is_finite() || padding < 0.0 {
        return Err(ElementLayoutError::InvalidLayout(
            "stack padding must be nonnegative and finite".to_owned(),
        ));
    }
    let content = Rect {
        x: bounds.x + padding,
        y: bounds.y + padding,
        width: bounds.width - padding * 2.0,
        height: bounds.height - padding * 2.0,
    };
    validate_flow_rect(content, "stack content bounds")?;
    Ok(content)
}

fn validate_flow_rect(bounds: Rect, label: &str) -> Result<(), ElementLayoutError> {
    if !bounds.x.is_finite()
        || !bounds.y.is_finite()
        || !bounds.width.is_finite()
        || !bounds.height.is_finite()
        || bounds.width <= 0.0
        || bounds.height <= 0.0
    {
        return Err(ElementLayoutError::InvalidLayout(format!(
            "{label} must have finite coordinates and positive dimensions"
        )));
    }
    Ok(())
}

fn validate_constraints(constraints: MeasureConstraints) -> Result<(), ElementLayoutError> {
    if !constraints.max_width.is_finite()
        || !constraints.max_height.is_finite()
        || constraints.max_width <= 0.0
        || constraints.max_height <= 0.0
    {
        return Err(ElementLayoutError::InvalidLayout(
            "flow measurement requires positive finite available width and height".to_owned(),
        ));
    }
    Ok(())
}

fn ensure_measured_size_fits(
    size: MeasuredSize,
    constraints: MeasureConstraints,
    source_path: &str,
) -> Result<(), ElementLayoutError> {
    if !size.width.is_finite()
        || !size.height.is_finite()
        || size.width <= 0.0
        || size.height <= 0.0
    {
        return Err(ElementLayoutError::InvalidLayout(format!(
            "{source_path} measured to invalid dimensions"
        )));
    }
    if size.width > constraints.max_width + 0.01 || size.height > constraints.max_height + 0.01 {
        return Err(ElementLayoutError::InvalidLayout(format!(
            "{source_path} measures {:.2}pt by {:.2}pt but only {:.2}pt by {:.2}pt is available",
            size.width, size.height, constraints.max_width, constraints.max_height
        )));
    }
    Ok(())
}

fn flow_size_hint_missing(element: &str) -> ElementLayoutError {
    ElementLayoutError::InvalidLayout(format!(
        "flow {element} elements require position width and height as size hints"
    ))
}

fn validate_stack_options(stack: &StackElement) -> Result<(), ElementLayoutError> {
    let gap = stack.gap.to_points();
    if !gap.is_finite() || gap < 0.0 {
        return Err(ElementLayoutError::InvalidLayout(
            "stack gap must be nonnegative and finite".to_owned(),
        ));
    }
    if stack.orphans == 0 {
        return Err(ElementLayoutError::InvalidLayout(
            "stack orphans must be at least 1".to_owned(),
        ));
    }
    Ok(())
}

fn layout_element_at(
    element: &Element,
    assigned_bounds: Option<Rect>,
    template: &Template,
    data: &DataRow,
    options: &LayoutOptions,
) -> Result<DrawCommand, ElementLayoutError> {
    match element {
        Element::Text(text) => {
            let bounds = assigned_bounds
                .or_else(|| text.position.as_ref().map(resolve_bounds))
                .ok_or_else(|| missing_position("text"))?;
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
                color: resolve_color(&text.color)?,
                clip: text.overflow == TextOverflow::Clip,
            }))
        }
        Element::Image(image) => {
            let bounds = assigned_bounds
                .or_else(|| image.position.as_ref().map(resolve_bounds))
                .ok_or_else(|| missing_position("image"))?;

            Ok(DrawCommand::Image(ImageCommand {
                bounds,
                source: resolve_asset_path(
                    &options.asset_base,
                    &resolve_string(&image.source, data)?,
                ),
                fit: image.fit,
            }))
        }
        Element::Rectangle(rectangle) => {
            let bounds = assigned_bounds
                .or_else(|| rectangle.position.as_ref().map(resolve_bounds))
                .ok_or_else(|| missing_position("rectangle"))?;

            Ok(DrawCommand::Rectangle(RectangleCommand {
                bounds,
                fill: rectangle.fill.as_deref().map(resolve_color).transpose()?,
                stroke: rectangle.stroke.as_ref().map(resolve_stroke).transpose()?,
            }))
        }
        Element::Line(line) => {
            if assigned_bounds.is_some() {
                return Err(ElementLayoutError::InvalidLayout(
                    "line elements use absolute coordinates and cannot be flow children".to_owned(),
                ));
            }
            Ok(DrawCommand::Line(LineCommand {
                start: Point {
                    x: line.x1.to_points(),
                    y: line.y1.to_points(),
                },
                end: Point {
                    x: line.x2.to_points(),
                    y: line.y2.to_points(),
                },
                width_pt: line.width.to_points(),
                color: resolve_color(&line.color)?,
                dash: resolve_dash(line.dash),
            }))
        }
        Element::Svg(svg) => {
            let bounds = assigned_bounds
                .or_else(|| svg.position.as_ref().map(resolve_bounds))
                .ok_or_else(|| missing_position("svg"))?;

            Ok(DrawCommand::Svg(SvgCommand {
                bounds,
                source: resolve_asset_path(
                    &options.asset_base,
                    &resolve_string(&svg.source, data)?,
                ),
                fit: svg.fit,
            }))
        }
        Element::QrCode(qr_code) => {
            let bounds = assigned_bounds
                .or_else(|| qr_code.position.as_ref().map(resolve_bounds))
                .ok_or_else(|| missing_position("qr_code"))?;
            build_qr_code_command(qr_code, bounds, data).map(DrawCommand::QrCode)
        }
        Element::Barcode(barcode) => {
            let bounds = assigned_bounds
                .or_else(|| barcode.position.as_ref().map(resolve_bounds))
                .ok_or_else(|| missing_position("barcode"))?;
            build_barcode_command(barcode, bounds, data).map(DrawCommand::Barcode)
        }
        Element::Group(_) => Err(ElementLayoutError::UnsupportedElement("group")),
        Element::Stack(_) => Err(ElementLayoutError::InvalidLayout(
            "stack elements must be laid out through the flow layout contract".to_owned(),
        )),
        Element::Table(_) => Err(ElementLayoutError::InvalidLayout(
            "MVP tables must be positioned top-level page elements".to_owned(),
        )),
        Element::Repeater(_) => Err(ElementLayoutError::UnsupportedElement("repeater")),
        Element::PageBreak => Err(ElementLayoutError::InvalidLayout(
            "page breaks are only valid between flow items".to_owned(),
        )),
    }
}

const MIN_SCANNABLE_MODULE_PT: f32 = 0.5;
const MIN_BARCODE_HEIGHT_PT: f32 = 14.4;

fn build_qr_code_command(
    element: &QrCodeElement,
    bounds: Rect,
    data: &DataRow,
) -> Result<QrCodeCommand, ElementLayoutError> {
    validate_flow_rect(bounds, "QR code bounds")?;
    if (bounds.width - bounds.height).abs() > 0.01 {
        return Err(ElementLayoutError::InvalidLayout(
            "QR code bounds must be square".to_owned(),
        ));
    }
    if element.quiet_zone < 4 {
        return Err(ElementLayoutError::InvalidLayout(
            "QR code quiet zone must be at least 4 modules".to_owned(),
        ));
    }
    let value = resolve_string(&element.value, data)?;
    if value.is_empty() {
        return Err(ElementLayoutError::InvalidLayout(
            "QR code value cannot be empty".to_owned(),
        ));
    }
    let error_correction = match element.error_correction {
        QrErrorCorrection::Low => QrCodeEcc::Low,
        QrErrorCorrection::Medium => QrCodeEcc::Medium,
        QrErrorCorrection::Quartile => QrCodeEcc::Quartile,
        QrErrorCorrection::High => QrCodeEcc::High,
    };
    let qr = QrCode::encode_text(&value, error_correction).map_err(|error| {
        ElementLayoutError::InvalidLayout(format!("QR code data cannot be encoded: {error}"))
    })?;
    let size = qr.size() as usize;
    let total_modules = size + usize::from(element.quiet_zone) * 2;
    let module_size = bounds.width / total_modules as f32;
    if module_size + 0.001 < MIN_SCANNABLE_MODULE_PT {
        return Err(ElementLayoutError::InvalidLayout(format!(
            "QR code module size is {module_size:.2}pt; increase its bounds to provide at least {MIN_SCANNABLE_MODULE_PT:.2}pt per module"
        )));
    }
    let mut modules = Vec::with_capacity(size * size);
    for y in 0..size {
        for x in 0..size {
            modules.push(qr.get_module(x as i32, y as i32));
        }
    }

    Ok(QrCodeCommand {
        bounds,
        size,
        modules,
        quiet_zone: element.quiet_zone,
        color: resolve_color(&element.color)?,
        background: resolve_color(&element.background)?,
    })
}

fn build_barcode_command(
    element: &BarcodeElement,
    bounds: Rect,
    data: &DataRow,
) -> Result<BarcodeCommand, ElementLayoutError> {
    validate_flow_rect(bounds, "barcode bounds")?;
    if element.quiet_zone < 10 {
        return Err(ElementLayoutError::InvalidLayout(
            "Code 128 quiet zone must be at least 10 modules".to_owned(),
        ));
    }
    if bounds.height + 0.01 < MIN_BARCODE_HEIGHT_PT {
        return Err(ElementLayoutError::InvalidLayout(format!(
            "Code 128 bar height is {:.2}pt; increase it to at least {MIN_BARCODE_HEIGHT_PT:.2}pt",
            bounds.height
        )));
    }
    let value = resolve_string(&element.value, data)?;
    if value.is_empty() {
        return Err(ElementLayoutError::InvalidLayout(
            "Code 128 value cannot be empty".to_owned(),
        ));
    }
    let modules = match element.format {
        BarcodeFormat::Code128 => Code128::new(format!("Ɓ{value}"))
            .map_err(|error| {
                ElementLayoutError::InvalidLayout(format!(
                    "Code 128 value cannot be encoded: {error}"
                ))
            })?
            .encode()
            .into_iter()
            .map(|module| module == 1)
            .collect::<Vec<_>>(),
    };
    let total_modules = modules.len() + usize::from(element.quiet_zone) * 2;
    let module_size = bounds.width / total_modules as f32;
    if module_size + 0.001 < MIN_SCANNABLE_MODULE_PT {
        return Err(ElementLayoutError::InvalidLayout(format!(
            "Code 128 module size is {module_size:.2}pt; increase its width to provide at least {MIN_SCANNABLE_MODULE_PT:.2}pt per module"
        )));
    }

    Ok(BarcodeCommand {
        bounds,
        modules,
        quiet_zone: element.quiet_zone,
        color: resolve_color(&element.color)?,
        background: resolve_color(&element.background)?,
    })
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
    Builtin(&'static [f32; 256]),
    External(Vec<u8>),
}

impl FontMetrics {
    fn load(font: &ResolvedFont) -> Result<Self, ElementLayoutError> {
        match font {
            ResolvedFont::Builtin(name) => {
                builtin_font_widths(name).map(Self::Builtin).ok_or_else(|| {
                    ElementLayoutError::InvalidLayout(format!(
                        "built-in font {name:?} does not have width metrics"
                    ))
                })
            }
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
            Self::Builtin(widths) => {
                value
                    .chars()
                    .map(|character| {
                        if character.is_ascii() {
                            widths[character as usize] / 1_000.0
                        } else {
                            approximate_advance_em(character)
                        }
                    })
                    .sum::<f32>()
                    * font_size
            }
            Self::External(bytes) => {
                let Ok(face) = ttf_parser::Face::parse(bytes, 0) else {
                    return value.chars().map(approximate_advance_em).sum::<f32>() * font_size;
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

fn builtin_font_widths(name: &str) -> Option<&'static [f32; 256]> {
    use std::sync::OnceLock;

    static HELVETICA: OnceLock<[f32; 256]> = OnceLock::new();
    static HELVETICA_BOLD: OnceLock<[f32; 256]> = OnceLock::new();
    static HELVETICA_OBLIQUE: OnceLock<[f32; 256]> = OnceLock::new();
    static HELVETICA_BOLD_OBLIQUE: OnceLock<[f32; 256]> = OnceLock::new();
    static TIMES_ROMAN: OnceLock<[f32; 256]> = OnceLock::new();
    static TIMES_BOLD: OnceLock<[f32; 256]> = OnceLock::new();
    static TIMES_ITALIC: OnceLock<[f32; 256]> = OnceLock::new();
    static TIMES_BOLD_ITALIC: OnceLock<[f32; 256]> = OnceLock::new();
    static COURIER: OnceLock<[f32; 256]> = OnceLock::new();
    static COURIER_BOLD: OnceLock<[f32; 256]> = OnceLock::new();
    static COURIER_OBLIQUE: OnceLock<[f32; 256]> = OnceLock::new();
    static COURIER_BOLD_OBLIQUE: OnceLock<[f32; 256]> = OnceLock::new();

    let (cache, afm) = match name {
        "helvetica" => (&HELVETICA, pdf_core_14_font_afms::HELVETICA),
        "helvetica-bold" => (&HELVETICA_BOLD, pdf_core_14_font_afms::HELVETICA_BOLD),
        "helvetica-oblique" => (&HELVETICA_OBLIQUE, pdf_core_14_font_afms::HELVETICA_OBLIQUE),
        "helvetica-bold-oblique" => (
            &HELVETICA_BOLD_OBLIQUE,
            pdf_core_14_font_afms::HELVETICA_BOLD_OBLIQUE,
        ),
        "times-roman" => (&TIMES_ROMAN, pdf_core_14_font_afms::TIMES_ROMAN),
        "times-bold" => (&TIMES_BOLD, pdf_core_14_font_afms::TIMES_BOLD),
        "times-italic" => (&TIMES_ITALIC, pdf_core_14_font_afms::TIMES_ITALIC),
        "times-bold-italic" => (&TIMES_BOLD_ITALIC, pdf_core_14_font_afms::TIMES_BOLD_ITALIC),
        "courier" => (&COURIER, pdf_core_14_font_afms::COURIER),
        "courier-bold" => (&COURIER_BOLD, pdf_core_14_font_afms::COURIER_BOLD),
        "courier-oblique" => (&COURIER_OBLIQUE, pdf_core_14_font_afms::COURIER_OBLIQUE),
        "courier-bold-oblique" => (
            &COURIER_BOLD_OBLIQUE,
            pdf_core_14_font_afms::COURIER_BOLD_OBLIQUE,
        ),
        _ => return None,
    };

    Some(cache.get_or_init(|| parse_afm_widths(afm)))
}

fn parse_afm_widths(afm: &str) -> [f32; 256] {
    let mut widths = [600.0; 256];
    for line in afm.lines().filter(|line| line.starts_with("C ")) {
        let mut code = None;
        let mut width = None;
        for field in line.split(';') {
            let mut parts = field.split_whitespace();
            match parts.next() {
                Some("C") => code = parts.next().and_then(|value| value.parse::<usize>().ok()),
                Some("WX") => width = parts.next().and_then(|value| value.parse::<f32>().ok()),
                _ => {}
            }
        }
        if let (Some(code @ 0..=255), Some(width)) = (code, width) {
            widths[code] = width;
        }
    }
    widths
}

fn approximate_advance_em(character: char) -> f32 {
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

fn resolve_stroke(stroke: &Stroke) -> Result<StrokeCommand, ElementLayoutError> {
    Ok(StrokeCommand {
        width_pt: stroke.width.to_points(),
        color: resolve_color(&stroke.color)?,
        dash: resolve_dash(stroke.dash),
    })
}

fn resolve_color(value: &str) -> Result<Color, ElementLayoutError> {
    value.parse::<Color>().map_err(|error| {
        ElementLayoutError::InvalidLayout(format!("invalid color {value:?}: {error}"))
    })
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
    #[error("element {element_path}: {source}")]
    Nested {
        element_path: String,
        #[source]
        source: Box<ElementLayoutError>,
    },
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use print_forge_dataset::DataRow;
    use print_forge_template::{
        Element, FlowOverflow, TableDateStyle, TableValueFormat, Template, TextAlign, TextOverflow,
    };
    use serde_json::json;

    use super::{
        BasicLayoutEngine, DrawCommand, ElementLayoutError, FontMetrics, LayoutEngine, LayoutError,
        LayoutOptions, Rect, ResolvedFont, TextLayoutSpec, format_table_value, layout_text,
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
              "font_size": { "value": 12.0, "unit": "points" },
              "rotation": 30
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
        assert_eq!(document.pages[0].commands[0].rotation, 30.0);
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
        let metrics = FontMetrics::load(&ResolvedFont::Builtin("helvetica".to_owned())).unwrap();
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
            &metrics,
        )
        .unwrap();

        assert!(text.lines.len() >= 2);
        assert!(text.font_size_pt < 18.0);
        assert!(text.lines.iter().all(|line| line.x >= bounds.x));
        assert!(text.lines.len() as f32 * text.line_height_pt <= bounds.height + f32::EPSILON);
    }

    #[test]
    fn error_overflow_rejects_text_that_does_not_fit() {
        let metrics = FontMetrics::load(&ResolvedFont::Builtin("helvetica".to_owned())).unwrap();
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
            &metrics,
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("text exceeds its absolute bounds")
        );
    }

    #[test]
    fn right_alignment_uses_exact_builtin_font_widths() {
        let metrics = FontMetrics::load(&ResolvedFont::Builtin("helvetica".to_owned())).unwrap();
        let bounds = Rect {
            x: 18.0,
            y: 18.0,
            width: 216.0,
            height: 24.0,
        };

        for (value, expected_width) in [
            ("Chris Johnson", 115.038_f32),
            ("Developer", 82.026_f32),
            ("111-111-1111", 112.068_f32),
        ] {
            let text = layout_text(
                value,
                TextLayoutSpec {
                    bounds,
                    requested_font_size: 18.0,
                    requested_line_height: 21.6,
                    min_font_size: 6.0,
                    align: TextAlign::Right,
                    overflow: TextOverflow::Error,
                },
                &metrics,
            )
            .unwrap();

            assert!((metrics.measure(value, 18.0) - expected_width).abs() < 0.001);
            assert!((text.lines[0].x + expected_width - 234.0).abs() < 0.001);
        }
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

    #[test]
    fn paginates_vertical_flow_and_repeats_header_and_footer() {
        let template: Template = serde_json::from_str(
            r##"{
              "name": "Paginated flow",
              "document": {
                "width": { "value": 200, "unit": "points" },
                "height": { "value": 200, "unit": "points" }
              },
              "pages": [{
                "header": [{
                  "type": "text",
                  "position": {
                    "x": { "value": 10, "unit": "points" },
                    "y": { "value": 180, "unit": "points" },
                    "width": { "value": 180, "unit": "points" },
                    "height": { "value": 12, "unit": "points" }
                  },
                  "value": "Repeated header",
                  "font_size": { "value": 10, "unit": "points" }
                }],
                "footer": [{
                  "type": "text",
                  "position": {
                    "x": { "value": 10, "unit": "points" },
                    "y": { "value": 5, "unit": "points" },
                    "width": { "value": 180, "unit": "points" },
                    "height": { "value": 12, "unit": "points" }
                  },
                  "value": "Repeated footer",
                  "font_size": { "value": 10, "unit": "points" }
                }],
                "elements": [
                  {
                    "type": "rectangle",
                    "position": {
                      "x": { "value": 2, "unit": "points" },
                      "y": { "value": 2, "unit": "points" },
                      "width": { "value": 4, "unit": "points" },
                      "height": { "value": 4, "unit": "points" }
                    },
                    "fill": "#000000"
                  },
                  {
                    "type": "stack",
                    "position": {
                      "x": { "value": 10, "unit": "points" },
                      "y": { "value": 30, "unit": "points" },
                      "width": { "value": 180, "unit": "points" },
                      "height": { "value": 140, "unit": "points" }
                    },
                    "gap": { "value": 5, "unit": "points" },
                    "padding": { "value": 5, "unit": "points" },
                    "orphans": 1,
                    "children": [
                      { "type": "rectangle", "position": { "x": { "value": 0, "unit": "points" }, "y": { "value": 0, "unit": "points" }, "width": { "value": 170, "unit": "points" }, "height": { "value": 50, "unit": "points" } }, "fill": "#111111" },
                      { "type": "rectangle", "position": { "x": { "value": 0, "unit": "points" }, "y": { "value": 0, "unit": "points" }, "width": { "value": 170, "unit": "points" }, "height": { "value": 50, "unit": "points" } }, "fill": "#222222" },
                      { "type": "rectangle", "position": { "x": { "value": 0, "unit": "points" }, "y": { "value": 0, "unit": "points" }, "width": { "value": 170, "unit": "points" }, "height": { "value": 50, "unit": "points" } }, "fill": "#333333" },
                      { "type": "rectangle", "position": { "x": { "value": 0, "unit": "points" }, "y": { "value": 0, "unit": "points" }, "width": { "value": 170, "unit": "points" }, "height": { "value": 50, "unit": "points" } }, "fill": "#444444" },
                      { "type": "rectangle", "position": { "x": { "value": 0, "unit": "points" }, "y": { "value": 0, "unit": "points" }, "width": { "value": 170, "unit": "points" }, "height": { "value": 50, "unit": "points" } }, "fill": "#555555" }
                    ]
                  }
                ]
              }]
            }"##,
        )
        .unwrap();

        let document = BasicLayoutEngine
            .layout(&template, &DataRow::new())
            .unwrap();

        assert_eq!(document.pages.len(), 3);
        assert!(document.pages.iter().all(|page| {
            page.commands
                .iter()
                .any(|command| command.source_path.contains(".header[0]"))
                && page
                    .commands
                    .iter()
                    .any(|command| command.source_path.contains(".footer[0]"))
        }));
        assert!(
            document.pages[1]
                .commands
                .iter()
                .all(|command| command.source_path != "pages[0].elements[0]")
        );

        let flow_rectangles = document.pages[0]
            .commands
            .iter()
            .filter(|command| command.source_path.contains(".children["))
            .filter_map(|command| match &command.command {
                DrawCommand::Rectangle(rectangle) => Some(rectangle.bounds),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(flow_rectangles.len(), 2);
        assert_eq!(
            flow_rectangles[0],
            Rect {
                x: 15.0,
                y: 115.0,
                width: 170.0,
                height: 50.0
            }
        );
        assert_eq!(flow_rectangles[1].y, 60.0);
    }

    #[test]
    fn lays_out_horizontal_stacks_with_gap_and_padding() {
        let template: Template = serde_json::from_str(
            r##"{
              "name": "Horizontal flow",
              "document": {
                "width": { "value": 200, "unit": "points" },
                "height": { "value": 150, "unit": "points" }
              },
              "pages": [{ "elements": [{
                "type": "stack",
                "direction": "horizontal",
                "position": {
                  "x": { "value": 10, "unit": "points" },
                  "y": { "value": 40, "unit": "points" },
                  "width": { "value": 180, "unit": "points" },
                  "height": { "value": 80, "unit": "points" }
                },
                "gap": { "value": 10, "unit": "points" },
                "padding": { "value": 10, "unit": "points" },
                "children": [
                  { "type": "rectangle", "position": { "x": { "value": 99, "unit": "points" }, "y": { "value": 99, "unit": "points" }, "width": { "value": 40, "unit": "points" }, "height": { "value": 20, "unit": "points" } }, "fill": "#111111" },
                  { "type": "rectangle", "position": { "x": { "value": 99, "unit": "points" }, "y": { "value": 99, "unit": "points" }, "width": { "value": 60, "unit": "points" }, "height": { "value": 30, "unit": "points" } }, "fill": "#222222" }
                ]
              }] }]
            }"##,
        )
        .unwrap();

        let document = BasicLayoutEngine
            .layout(&template, &DataRow::new())
            .unwrap();
        let rectangles = document.pages[0]
            .commands
            .iter()
            .filter_map(|command| match &command.command {
                DrawCommand::Rectangle(rectangle) => Some(rectangle.bounds),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(
            rectangles[0],
            Rect {
                x: 20.0,
                y: 90.0,
                width: 40.0,
                height: 20.0
            }
        );
        assert_eq!(
            rectangles[1],
            Rect {
                x: 70.0,
                y: 80.0,
                width: 60.0,
                height: 30.0
            }
        );
    }

    #[test]
    fn measures_unpositioned_text_in_a_vertical_flow() {
        let template: Template = serde_json::from_str(
            r##"{
              "name": "Measured text",
              "document": {
                "width": { "value": 120, "unit": "points" },
                "height": { "value": 120, "unit": "points" }
              },
              "pages": [{ "elements": [{
                "type": "stack",
                "position": {
                  "x": { "value": 10, "unit": "points" },
                  "y": { "value": 10, "unit": "points" },
                  "width": { "value": 100, "unit": "points" },
                  "height": { "value": 100, "unit": "points" }
                },
                "padding": { "value": 10, "unit": "points" },
                "children": [{
                  "type": "text",
                  "value": "Measured flow text wraps to the available width",
                  "font_size": { "value": 10, "unit": "points" },
                  "line_height": { "value": 12, "unit": "points" }
                }]
              }] }]
            }"##,
        )
        .unwrap();

        let document = BasicLayoutEngine
            .layout(&template, &DataRow::new())
            .unwrap();
        let DrawCommand::Text(text) = &document.pages[0].commands[0].command else {
            panic!("expected text command");
        };

        assert_eq!(text.bounds.x, 20.0);
        assert_eq!(text.bounds.width, 80.0);
        assert_eq!(text.bounds.height, text.lines.len() as f32 * 12.0);
        assert!(text.lines.len() > 1);
    }

    #[test]
    fn honors_explicit_breaks_and_rejects_unsafe_break_policies() {
        let explicit: Template = serde_json::from_str(
            r##"{
              "name": "Explicit break",
              "document": {
                "width": { "value": 100, "unit": "points" },
                "height": { "value": 100, "unit": "points" }
              },
              "pages": [{ "elements": [{
                "type": "stack",
                "position": {
                  "x": { "value": 10, "unit": "points" },
                  "y": { "value": 10, "unit": "points" },
                  "width": { "value": 80, "unit": "points" },
                  "height": { "value": 80, "unit": "points" }
                },
                "children": [
                  { "type": "rectangle", "position": { "x": { "value": 0, "unit": "points" }, "y": { "value": 0, "unit": "points" }, "width": { "value": 80, "unit": "points" }, "height": { "value": 20, "unit": "points" } }, "fill": "#111111" },
                  { "type": "page_break" },
                  { "type": "rectangle", "position": { "x": { "value": 0, "unit": "points" }, "y": { "value": 0, "unit": "points" }, "width": { "value": 80, "unit": "points" }, "height": { "value": 20, "unit": "points" } }, "fill": "#222222" }
                ]
              }] }]
            }"##,
        )
        .unwrap();
        assert_eq!(
            BasicLayoutEngine
                .layout(&explicit, &DataRow::new())
                .unwrap()
                .pages
                .len(),
            2
        );

        let mut keep_together = explicit.clone();
        let Element::Stack(stack) = &mut keep_together.pages[0].elements[0] else {
            unreachable!();
        };
        stack.keep_together = true;
        assert!(
            BasicLayoutEngine
                .layout(&keep_together, &DataRow::new())
                .unwrap_err()
                .to_string()
                .contains("keep-together")
        );

        let mut orphaned = explicit;
        let Element::Stack(stack) = &mut orphaned.pages[0].elements[0] else {
            unreachable!();
        };
        stack.children.remove(1);
        for child in &mut stack.children {
            let Element::Rectangle(rectangle) = child else {
                unreachable!()
            };
            rectangle.position.as_mut().unwrap().height.value = 60.0;
        }
        stack.orphans = 2;
        assert!(
            BasicLayoutEngine
                .layout(&orphaned, &DataRow::new())
                .unwrap_err()
                .to_string()
                .contains("orphan item")
        );

        let Element::Stack(stack) = &mut orphaned.pages[0].elements[0] else {
            unreachable!();
        };
        stack.orphans = 1;
        stack.overflow = FlowOverflow::Error;
        assert!(
            BasicLayoutEngine
                .layout(&orphaned, &DataRow::new())
                .unwrap_err()
                .to_string()
                .contains("overflow is set to error")
        );
    }

    #[test]
    fn encodes_vector_qr_and_code128_commands_with_preflight() {
        let template: Template = serde_json::from_str(
            r##"{
              "name": "Specialty codes",
              "document": {
                "width": { "value": 240, "unit": "points" },
                "height": { "value": 200, "unit": "points" }
              },
              "pages": [{ "elements": [
                {
                  "type": "qr_code",
                  "position": {
                    "x": { "value": 10, "unit": "points" },
                    "y": { "value": 100, "unit": "points" },
                    "width": { "value": 80, "unit": "points" },
                    "height": { "value": 80, "unit": "points" }
                  },
                  "value": "{{url}}",
                  "error_correction": "high",
                  "quiet_zone": 6
                },
                {
                  "type": "barcode",
                  "position": {
                    "x": { "value": 10, "unit": "points" },
                    "y": { "value": 30, "unit": "points" },
                    "width": { "value": 220, "unit": "points" },
                    "height": { "value": 40, "unit": "points" }
                  },
                  "value": "{{sku}}"
                },
                {
                  "type": "svg",
                  "position": {
                    "x": { "value": 110, "unit": "points" },
                    "y": { "value": 100, "unit": "points" },
                    "width": { "value": 80, "unit": "points" },
                    "height": { "value": 80, "unit": "points" }
                  },
                  "source": "{{icon}}"
                }
              ] }]
            }"##,
        )
        .unwrap();
        let data: DataRow = serde_json::from_value(json!({
            "url": "https://example.com/forge",
            "sku": "PF-100",
            "icon": "assets/icon.svg"
        }))
        .unwrap();
        let document = BasicLayoutEngine
            .layout_with_options(
                &template,
                &data,
                &LayoutOptions {
                    asset_base: PathBuf::from("examples"),
                },
            )
            .unwrap();

        let DrawCommand::QrCode(qr_code) = &document.pages[0].commands[0].command else {
            panic!("expected QR code");
        };
        assert!(qr_code.size >= 21);
        assert_eq!(qr_code.modules.len(), qr_code.size * qr_code.size);
        assert_eq!(qr_code.quiet_zone, 6);

        let DrawCommand::Barcode(barcode) = &document.pages[0].commands[1].command else {
            panic!("expected barcode");
        };
        assert!(barcode.modules.len() > 80);
        assert!(barcode.modules.iter().any(|module| *module));
        assert_eq!(barcode.quiet_zone, 10);

        let DrawCommand::Svg(svg) = &document.pages[0].commands[2].command else {
            panic!("expected SVG");
        };
        assert_eq!(svg.source, PathBuf::from("examples/assets/icon.svg"));

        let mut too_small = template;
        let Element::QrCode(qr_code) = &mut too_small.pages[0].elements[0] else {
            unreachable!();
        };
        qr_code.position.as_mut().unwrap().width.value = 10.0;
        qr_code.position.as_mut().unwrap().height.value = 10.0;
        let error = BasicLayoutEngine.layout(&too_small, &data).unwrap_err();
        assert!(error.to_string().contains("module size"));
    }

    #[test]
    fn translates_group_children_and_nested_groups() {
        let template: Template = serde_json::from_str(
            r##"{
              "name": "Translated groups",
              "document": {
                "width": { "value": 200, "unit": "points" },
                "height": { "value": 200, "unit": "points" }
              },
              "pages": [{ "elements": [{
                "type": "group",
                "position": {
                  "x": { "value": 20, "unit": "points" },
                  "y": { "value": 30, "unit": "points" },
                  "width": { "value": 100, "unit": "points" },
                  "height": { "value": 100, "unit": "points" }
                },
                "children": [
                  {
                    "type": "rectangle",
                    "position": {
                      "x": { "value": 5, "unit": "points" },
                      "y": { "value": 7, "unit": "points" },
                      "width": { "value": 20, "unit": "points" },
                      "height": { "value": 10, "unit": "points" }
                    },
                    "fill": "#112233"
                  },
                  {
                    "type": "group",
                    "position": {
                      "x": { "value": 40, "unit": "points" },
                      "y": { "value": 50, "unit": "points" },
                      "width": { "value": 40, "unit": "points" },
                      "height": { "value": 40, "unit": "points" }
                    },
                    "children": [{
                      "type": "line",
                      "x1": { "value": 1, "unit": "points" },
                      "y1": { "value": 2, "unit": "points" },
                      "x2": { "value": 11, "unit": "points" },
                      "y2": { "value": 12, "unit": "points" },
                      "width": { "value": 1, "unit": "points" }
                    }]
                  }
                ]
              }] }]
            }"##,
        )
        .unwrap();

        let document = BasicLayoutEngine
            .layout(&template, &DataRow::new())
            .unwrap();
        let DrawCommand::Rectangle(rectangle) = &document.pages[0].commands[0].command else {
            panic!("expected rectangle");
        };
        assert_eq!(rectangle.bounds.x, 25.0);
        assert_eq!(rectangle.bounds.y, 37.0);
        let DrawCommand::Line(line) = &document.pages[0].commands[1].command else {
            panic!("expected line");
        };
        assert_eq!(line.start, super::Point { x: 61.0, y: 82.0 });
        assert_eq!(line.end, super::Point { x: 71.0, y: 92.0 });
    }

    #[test]
    fn repeats_grid_items_with_item_and_root_scope_across_pages() {
        let template: Template = serde_json::from_str(
            r##"{
              "name": "Grid repeater",
              "document": {
                "width": { "value": 100, "unit": "points" },
                "height": { "value": 100, "unit": "points" }
              },
              "pages": [{ "elements": [{
                "type": "repeater",
                "source": "catalog.products",
                "layout": "grid",
                "template": {
                  "type": "group",
                  "position": {
                    "x": { "value": 10, "unit": "points" },
                    "y": { "value": 60, "unit": "points" },
                    "width": { "value": 30, "unit": "points" },
                    "height": { "value": 30, "unit": "points" }
                  },
                  "children": [
                    {
                      "type": "rectangle",
                      "position": {
                        "x": { "value": 2, "unit": "points" },
                        "y": { "value": 3, "unit": "points" },
                        "width": { "value": 26, "unit": "points" },
                        "height": { "value": 24, "unit": "points" }
                      },
                      "fill": "#EEEEEE"
                    },
                    {
                      "type": "text",
                      "position": {
                        "x": { "value": 2, "unit": "points" },
                        "y": { "value": 8, "unit": "points" },
                        "width": { "value": 26, "unit": "points" },
                        "height": { "value": 10, "unit": "points" }
                      },
                      "value": "{{name}}-{{root.batch}}-{{index}}",
                      "font_size": { "value": 5, "unit": "points" },
                      "overflow": "shrink"
                    }
                  ]
                }
              }] }]
            }"##,
        )
        .unwrap();
        let products = (0..10)
            .map(|index| json!({ "name": format!("P{index}") }))
            .collect::<Vec<_>>();
        let data: DataRow = serde_json::from_value(json!({
            "batch": "R",
            "catalog": { "products": products }
        }))
        .unwrap();

        let document = BasicLayoutEngine.layout(&template, &data).unwrap();
        assert_eq!(document.pages.len(), 2);
        let rectangles = document.pages[0]
            .commands
            .iter()
            .filter_map(|command| match &command.command {
                DrawCommand::Rectangle(rectangle) => Some(rectangle.bounds),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(rectangles.len(), 9);
        assert_eq!((rectangles[0].x, rectangles[0].y), (12.0, 63.0));
        assert_eq!((rectangles[1].x, rectangles[1].y), (42.0, 63.0));
        assert_eq!((rectangles[3].x, rectangles[3].y), (12.0, 33.0));

        let DrawCommand::Text(first_text) = &document.pages[0].commands[1].command else {
            panic!("expected text");
        };
        assert_eq!(first_text.lines[0].value, "P0-R-1");
        let DrawCommand::Text(last_text) = &document.pages[1].commands[1].command else {
            panic!("expected text");
        };
        assert_eq!(last_text.lines[0].value, "P9-R-10");
    }

    #[test]
    fn supports_vertical_and_horizontal_repeater_layouts() {
        for (layout, expected_second, expected_pages) in [
            ("vertical", (10.0, 30.0), 2),
            ("horizontal", (40.0, 60.0), 2),
        ] {
            let source = format!(
                r##"{{
                  "name": "{layout} repeater",
                  "document": {{
                    "width": {{ "value": 70, "unit": "points" }},
                    "height": {{ "value": 90, "unit": "points" }}
                  }},
                  "pages": [{{ "elements": [{{
                    "type": "repeater",
                    "source": "items",
                    "layout": "{layout}",
                    "template": {{
                      "type": "rectangle",
                      "position": {{
                        "x": {{ "value": 10, "unit": "points" }},
                        "y": {{ "value": 60, "unit": "points" }},
                        "width": {{ "value": 30, "unit": "points" }},
                        "height": {{ "value": 30, "unit": "points" }}
                      }},
                      "fill": "#000000"
                    }}
                  }}] }}]
                }}"##
            );
            let template: Template = serde_json::from_str(&source).unwrap();
            let data: DataRow = serde_json::from_value(json!({ "items": [1, 2, 3, 4] })).unwrap();
            let document = BasicLayoutEngine.layout(&template, &data).unwrap();
            assert_eq!(document.pages.len(), expected_pages);
            let DrawCommand::Rectangle(second) = &document.pages[0].commands[1].command else {
                panic!("expected rectangle");
            };
            assert_eq!((second.bounds.x, second.bounds.y), expected_second);
        }
    }

    #[test]
    fn product_catalog_continuation_retains_composition_and_page_furniture() {
        let template: Template =
            serde_json::from_str(include_str!("../../../examples/product-catalog.json")).unwrap();
        let rows = serde_json::from_str::<Vec<serde_json::Value>>(include_str!(
            "../../../examples/product-catalog-data.json"
        ))
        .unwrap();
        let data = rows[0].as_object().unwrap().clone();
        let asset_base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples");

        let document = BasicLayoutEngine
            .layout_with_options(&template, &data, &LayoutOptions { asset_base })
            .unwrap();

        assert_eq!(document.pages.len(), 2);
        assert_eq!(document.pages[0].commands.len(), 20);
        assert_eq!(document.pages[1].commands.len(), 14);
        assert!(document.pages[1].commands.iter().any(|command| {
            command.source_path == "pages[0].header[0]"
                && matches!(command.command, DrawCommand::Text(_))
        }));
        assert!(document.pages[1].commands.iter().any(|command| {
            command
                .source_path
                .ends_with("items[3].template.children[0]")
                && matches!(command.command, DrawCommand::Rectangle(_))
        }));
        assert!(document.pages[1].commands.iter().any(|command| {
            command.source_path == "pages[0].footer[0]"
                && matches!(command.command, DrawCommand::Text(_))
        }));
    }

    #[test]
    fn formats_table_numbers_currency_and_dates() {
        assert_eq!(
            format_table_value(
                &json!(-1234.5),
                Some(&TableValueFormat::Currency {
                    symbol: "$".to_owned(),
                    decimals: 2,
                })
            )
            .unwrap(),
            "-$1,234.50"
        );
        assert_eq!(
            format_table_value(
                &json!(12345.678),
                Some(&TableValueFormat::Number { decimals: 1 })
            )
            .unwrap(),
            "12,345.7"
        );
        assert_eq!(
            format_table_value(
                &json!("2024-02-29"),
                Some(&TableValueFormat::Date {
                    style: TableDateStyle::Long,
                })
            )
            .unwrap(),
            "February 29, 2024"
        );
        assert!(format_table_value(&json!([1, 2]), None).is_err());
        assert!(
            format_table_value(&json!(1), Some(&TableValueFormat::Number { decimals: 13 }))
                .is_err()
        );
    }

    #[test]
    fn measures_and_paginates_table_rows_with_repeated_headers() {
        let template: Template = serde_json::from_str(
            r##"{
              "name": "Paginated table",
              "document": {
                "width": { "value": 240, "unit": "points" },
                "height": { "value": 200, "unit": "points" }
              },
              "pages": [{
                "header": [{
                  "type": "text",
                  "position": {
                    "x": { "value": 20, "unit": "points" },
                    "y": { "value": 180, "unit": "points" },
                    "width": { "value": 200, "unit": "points" },
                    "height": { "value": 12, "unit": "points" }
                  },
                  "value": "Repeated page header",
                  "font_size": { "value": 9, "unit": "points" }
                }],
                "elements": [{
                  "type": "table",
                  "position": {
                    "x": { "value": 20, "unit": "points" },
                    "y": { "value": 30, "unit": "points" },
                    "width": { "value": 200, "unit": "points" },
                    "height": { "value": 130, "unit": "points" }
                  },
                  "source": "items",
                  "header": true,
                  "font_size": { "value": 9, "unit": "points" },
                  "line_height": { "value": 11, "unit": "points" },
                  "cell_padding": { "value": 4, "unit": "points" },
                  "header_background": "#DDDDDD",
                  "alternate_row_background": "#F5F5F5",
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
                        "value": { "value": 60, "unit": "points" }
                      }
                    },
                    {
                      "field": "amount",
                      "header": "Amount",
                      "width": { "type": "percent", "value": 35 },
                      "align": "right",
                      "format": { "type": "currency", "symbol": "$", "decimals": 2 }
                    },
                    {
                      "field": "date",
                      "header": "Date",
                      "width": { "type": "percent", "value": 35 },
                      "align": "center",
                      "format": { "type": "date", "style": "us" }
                    }
                  ]
                }]
              }]
            }"##,
        )
        .unwrap();
        let data: DataRow = serde_json::from_value(json!({
            "items": [
                {"description":"A long description that wraps", "amount":1234.5, "date":"2026-01-01"},
                {"description":"Second", "amount":2, "date":"2026-01-02"},
                {"description":"Third", "amount":3, "date":"2026-01-03"},
                {"description":"Fourth", "amount":4, "date":"2026-01-04"},
                {"description":"Fifth", "amount":5, "date":"2026-01-05"},
                {"description":"Sixth", "amount":6, "date":"2026-01-06"},
                {"description":"Seventh", "amount":7, "date":"2026-01-07"}
            ]
        }))
        .unwrap();

        let document = BasicLayoutEngine.layout(&template, &data).unwrap();
        assert!(document.pages.len() >= 2);
        assert!(document.pages.iter().all(|page| {
            page.commands
                .iter()
                .any(|command| command.source_path.contains(".header.cells[0]"))
                && page
                    .commands
                    .iter()
                    .any(|command| command.source_path.contains(".grid.vertical[0]"))
                && page
                    .commands
                    .iter()
                    .any(|command| command.source_path == "pages[0].header[0]")
        }));

        let body_cells = document
            .pages
            .iter()
            .flat_map(|page| &page.commands)
            .filter(|command| {
                command.source_path.contains(".rows[") && command.source_path.contains(".cells[")
            })
            .collect::<Vec<_>>();
        assert_eq!(body_cells.len(), 21);
        let first_description = body_cells
            .iter()
            .find(|command| command.source_path.contains(".rows[0].cells[0]"))
            .unwrap();
        let DrawCommand::Text(first_description) = &first_description.command else {
            panic!("expected table text");
        };
        assert!(first_description.lines.len() > 1);
        let first_amount = body_cells
            .iter()
            .find(|command| command.source_path.contains(".rows[0].cells[1]"))
            .unwrap();
        let DrawCommand::Text(first_amount) = &first_amount.command else {
            panic!("expected formatted currency text");
        };
        assert_eq!(first_amount.lines[0].value, "$1,234.50");
    }
}
