use std::{fs, path::PathBuf};

use eframe::egui::{
    self, Align, Align2, Color32, ComboBox, CornerRadius, FontId, Frame, Id, Key, Layout, Margin,
    Pos2, Rect, RichText, ScrollArea, Sense, Stroke, StrokeKind, Vec2,
};
use print_forge_dataset::DataRow;
use print_forge_engine::{BasicLayoutEngine, LayoutOptions};
use print_forge_pdf::{PdfRenderOptions, PdfRenderer};
use print_forge_template::{
    DashStyle, Element, FieldType, FontStyle, ImageFit, Length, QrErrorCorrection, Template,
    TextAlign, TextOverflow, Unit,
};
use print_forge_validation::{ValidationReport, validate_template};
use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
use serde::{Deserialize, Serialize};

use crate::model::{
    ElementKind, blank_page, bounds_points, element_bounds, element_bounds_mut, element_label,
    new_element, new_field, resize_element, starter_template, translate_element,
};

const APP_STATE_KEY: &str = "print-forge-studio-state";
const ORANGE: Color32 = Color32::from_rgb(244, 91, 32);
const CHARCOAL: Color32 = Color32::from_rgb(31, 36, 41);
const PANEL: Color32 = Color32::from_rgb(38, 44, 50);
const PANEL_DEEP: Color32 = Color32::from_rgb(26, 31, 36);
const CREAM: Color32 = Color32::from_rgb(250, 247, 239);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Selection {
    Document,
    Page,
    Field(usize),
    Element(usize),
}

#[derive(Serialize, Deserialize)]
struct PersistedState {
    template: Template,
    current_path: Option<PathBuf>,
    current_page: usize,
    preview_data: String,
}

#[derive(Debug, Clone, Copy)]
enum NoticeKind {
    Info,
    Success,
    Error,
}

struct Notice {
    kind: NoticeKind,
    message: String,
}

pub struct StudioApp {
    template: Template,
    current_path: Option<PathBuf>,
    current_page: usize,
    selection: Selection,
    zoom: f32,
    dirty: bool,
    notice: Notice,
    show_json: bool,
    json_buffer: String,
    show_preview_data: bool,
    preview_data: String,
    print_ready_preview: bool,
}

impl StudioApp {
    pub fn new(creation: &eframe::CreationContext<'_>) -> Self {
        configure_style(&creation.egui_ctx);
        let persisted = creation
            .storage
            .and_then(|storage| eframe::get_value::<PersistedState>(storage, APP_STATE_KEY));
        let (template, current_path, current_page, preview_data) = persisted.map_or_else(
            || (starter_template(), None, 0, "{}".to_owned()),
            |state| {
                (
                    state.template,
                    state.current_path,
                    state.current_page,
                    state.preview_data,
                )
            },
        );

        Self {
            template,
            current_path,
            current_page,
            selection: Selection::Document,
            zoom: 1.0,
            dirty: false,
            notice: Notice {
                kind: NoticeKind::Info,
                message: "Ready to forge".to_owned(),
            },
            show_json: false,
            json_buffer: String::new(),
            show_preview_data: false,
            preview_data,
            print_ready_preview: false,
        }
    }

    fn new_project(&mut self) {
        if !self.confirm_discard() {
            return;
        }
        self.template = starter_template();
        self.current_path = None;
        self.current_page = 0;
        self.selection = Selection::Document;
        self.dirty = false;
        self.set_notice(NoticeKind::Success, "Created a new template");
    }

    fn open_project(&mut self) {
        if !self.confirm_discard() {
            return;
        }
        let Some(path) = FileDialog::new()
            .add_filter("Print Forge template", &["json"])
            .set_title("Open Print Forge template")
            .pick_file()
        else {
            return;
        };
        match fs::read_to_string(&path)
            .map_err(|error| error.to_string())
            .and_then(|json| {
                serde_json::from_str::<Template>(&json).map_err(|error| error.to_string())
            }) {
            Ok(template) => {
                self.template = template;
                self.current_path = Some(path.clone());
                self.current_page = 0;
                self.selection = Selection::Document;
                self.dirty = false;
                self.set_notice(
                    NoticeKind::Success,
                    format!("Opened {}", display_name(&path)),
                );
            }
            Err(error) => self.set_notice(NoticeKind::Error, format!("Open failed: {error}")),
        }
    }

    fn save_project(&mut self, save_as: bool) {
        let path = if !save_as {
            self.current_path.clone()
        } else {
            None
        };
        let path = path.or_else(|| {
            FileDialog::new()
                .add_filter("Print Forge template", &["json"])
                .set_file_name(safe_template_name(&self.template.name))
                .set_title("Save Print Forge template")
                .save_file()
        });
        let Some(mut path) = path else {
            return;
        };
        if path.extension().is_none() {
            path.set_extension("json");
        }
        match serialize_template(&self.template)
            .and_then(|json| fs::write(&path, json).map_err(|error| error.to_string()))
        {
            Ok(()) => {
                self.current_path = Some(path.clone());
                self.dirty = false;
                self.set_notice(
                    NoticeKind::Success,
                    format!("Saved {}", display_name(&path)),
                );
            }
            Err(error) => self.set_notice(NoticeKind::Error, format!("Save failed: {error}")),
        }
    }

    fn render_pdf(&mut self) {
        let report = validate_template(&self.template);
        if !report.is_valid() {
            self.set_notice(
                NoticeKind::Error,
                format!(
                    "Fix {} validation error(s) before rendering",
                    report.error_count()
                ),
            );
            return;
        }
        let data = match serde_json::from_str::<DataRow>(&self.preview_data) {
            Ok(data) => data,
            Err(error) => {
                self.set_notice(
                    NoticeKind::Error,
                    format!("Preview data must be one JSON object: {error}"),
                );
                self.show_preview_data = true;
                return;
            }
        };
        let asset_base = self
            .current_path
            .as_deref()
            .and_then(|path| path.parent())
            .unwrap_or_else(|| std::path::Path::new("."))
            .to_owned();
        let document = match BasicLayoutEngine.layout_with_options(
            &self.template,
            &data,
            &LayoutOptions { asset_base },
        ) {
            Ok(document) => document,
            Err(error) => {
                self.set_notice(NoticeKind::Error, format!("Layout failed: {error}"));
                return;
            }
        };
        let options = if self.print_ready_preview {
            PdfRenderOptions::print_ready()
        } else {
            PdfRenderOptions::default()
        };
        let bytes = match PdfRenderer.render_with_options(&document, &options) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.set_notice(NoticeKind::Error, format!("PDF render failed: {error}"));
                return;
            }
        };
        let Some(mut path) = FileDialog::new()
            .add_filter("PDF document", &["pdf"])
            .set_file_name(format!("{}.pdf", safe_stem(&self.template.name)))
            .set_title("Render preview PDF")
            .save_file()
        else {
            return;
        };
        if path.extension().is_none() {
            path.set_extension("pdf");
        }
        match fs::write(&path, bytes) {
            Ok(()) => self.set_notice(
                NoticeKind::Success,
                format!("Rendered {}", display_name(&path)),
            ),
            Err(error) => self.set_notice(NoticeKind::Error, format!("Write failed: {error}")),
        }
    }

    fn confirm_discard(&self) -> bool {
        !self.dirty
            || matches!(
                MessageDialog::new()
                    .set_level(MessageLevel::Warning)
                    .set_title("Unsaved template")
                    .set_description("Discard the unsaved changes to this template?")
                    .set_buttons(MessageButtons::YesNo)
                    .show(),
                MessageDialogResult::Yes
            )
    }

    fn set_notice(&mut self, kind: NoticeKind, message: impl Into<String>) {
        self.notice = Notice {
            kind,
            message: message.into(),
        };
    }

    fn show_toolbar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("studio-toolbar")
            .exact_height(58.0)
            .frame(
                Frame::new()
                    .fill(CHARCOAL)
                    .inner_margin(Margin::symmetric(16, 10)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("◆").color(ORANGE).size(25.0).strong());
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("PRINT FORGE")
                                .color(CREAM)
                                .size(15.0)
                                .strong(),
                        );
                        ui.label(RichText::new("STUDIO").color(ORANGE).size(10.0).strong());
                    });
                    ui.add_space(18.0);
                    let path = self
                        .current_path
                        .as_deref()
                        .map(display_name)
                        .unwrap_or_else(|| "Untitled template".to_owned());
                    ui.label(
                        RichText::new(format!("{path}{}", if self.dirty { "  •" } else { "" }))
                            .color(Color32::from_gray(190)),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if accent_button(ui, "Render PDF").clicked() {
                            self.render_pdf();
                        }
                        if ui.button("Preview data").clicked() {
                            self.show_preview_data = true;
                        }
                        if ui.button("JSON").clicked() {
                            self.json_buffer =
                                serialize_template(&self.template).unwrap_or_default();
                            self.show_json = true;
                        }
                        if ui.button("Save as").clicked() {
                            self.save_project(true);
                        }
                        if ui.button("Save").clicked() {
                            self.save_project(false);
                        }
                        if ui.button("Open").clicked() {
                            self.open_project();
                        }
                        if ui.button("New").clicked() {
                            self.new_project();
                        }
                    });
                });
            });
    }

    fn show_left_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("studio-layers")
            .default_width(244.0)
            .width_range(210.0..=310.0)
            .frame(Frame::new().fill(PANEL).inner_margin(Margin::same(12)))
            .show(ctx, |ui| {
                ScrollArea::vertical().show(ui, |ui| {
                    section_label(ui, "PROJECT");
                    if selectable_row(ui, self.selection == Selection::Document, "Document")
                        .clicked()
                    {
                        self.selection = Selection::Document;
                    }
                    ui.add_space(14.0);

                    ui.horizontal(|ui| {
                        section_label(ui, "PAGES");
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if small_button(ui, "+").clicked() {
                                self.template.pages.push(blank_page());
                                self.current_page = self.template.pages.len() - 1;
                                self.selection = Selection::Page;
                                self.dirty = true;
                            }
                        });
                    });
                    for page_index in 0..self.template.pages.len() {
                        let selected = page_index == self.current_page;
                        if selectable_row(ui, selected, &format!("Page {}", page_index + 1))
                            .clicked()
                        {
                            self.current_page = page_index;
                            self.selection = Selection::Page;
                        }
                    }

                    ui.add_space(16.0);
                    section_label(ui, "ADD ELEMENT");
                    egui::Grid::new("element-palette")
                        .num_columns(2)
                        .spacing([6.0, 6.0])
                        .show(ui, |ui| {
                            for (index, kind) in ElementKind::ALL.into_iter().enumerate() {
                                if ui
                                    .add_sized([101.0, 30.0], egui::Button::new(kind.label()))
                                    .clicked()
                                {
                                    let offset =
                                        (self.template.pages[self.current_page].elements.len() % 8)
                                            as f32
                                            * 9.0;
                                    self.template.pages[self.current_page]
                                        .elements
                                        .push(new_element(kind, offset));
                                    let element_index =
                                        self.template.pages[self.current_page].elements.len() - 1;
                                    self.selection = Selection::Element(element_index);
                                    self.dirty = true;
                                }
                                if index % 2 == 1 {
                                    ui.end_row();
                                }
                            }
                        });

                    ui.add_space(16.0);
                    section_label(ui, "LAYERS");
                    let labels: Vec<String> = self.template.pages[self.current_page]
                        .elements
                        .iter()
                        .enumerate()
                        .map(|(index, element)| element_label(element, index))
                        .collect();
                    for (index, label) in labels.iter().enumerate().rev() {
                        if selectable_row(ui, self.selection == Selection::Element(index), label)
                            .clicked()
                        {
                            self.selection = Selection::Element(index);
                        }
                    }

                    ui.add_space(16.0);
                    ui.horizontal(|ui| {
                        section_label(ui, "DATA FIELDS");
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if small_button(ui, "+").clicked() {
                                self.template
                                    .fields
                                    .push(new_field(self.template.fields.len()));
                                self.selection = Selection::Field(self.template.fields.len() - 1);
                                self.dirty = true;
                            }
                        });
                    });
                    if self.template.fields.is_empty() {
                        ui.label(
                            RichText::new("No variable fields yet")
                                .color(Color32::from_gray(135))
                                .italics(),
                        );
                    }
                    for (index, field) in self.template.fields.iter().enumerate() {
                        if selectable_row(
                            ui,
                            self.selection == Selection::Field(index),
                            &format!("{{{{{}}}}}", field.name),
                        )
                        .clicked()
                        {
                            self.selection = Selection::Field(index);
                        }
                    }
                });
            });
    }

    fn show_right_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::right("studio-inspector")
            .default_width(300.0)
            .width_range(270.0..=360.0)
            .frame(Frame::new().fill(PANEL).inner_margin(Margin::same(14)))
            .show(ctx, |ui| {
                section_label(ui, "INSPECTOR");
                ui.add_space(4.0);
                ScrollArea::vertical().show(ui, |ui| match self.selection {
                    Selection::Document => {
                        if document_inspector(ui, &mut self.template) {
                            self.dirty = true;
                        }
                    }
                    Selection::Page => self.page_inspector(ui),
                    Selection::Field(index) => self.field_inspector(ui, index),
                    Selection::Element(index) => self.element_inspector(ui, index),
                });
            });
    }

    fn page_inspector(&mut self, ui: &mut egui::Ui) {
        ui.heading(format!("Page {}", self.current_page + 1));
        ui.label(
            RichText::new(format!(
                "{} layer(s)",
                self.template.pages[self.current_page].elements.len()
            ))
            .color(Color32::from_gray(155)),
        );
        ui.add_space(12.0);
        if ui.button("Duplicate page").clicked() {
            let duplicate = self.template.pages[self.current_page].clone();
            self.template.pages.insert(self.current_page + 1, duplicate);
            self.current_page += 1;
            self.dirty = true;
        }
        ui.add_enabled_ui(self.template.pages.len() > 1, |ui| {
            if danger_button(ui, "Delete page").clicked() {
                self.template.pages.remove(self.current_page);
                self.current_page = self.current_page.min(self.template.pages.len() - 1);
                self.selection = Selection::Page;
                self.dirty = true;
            }
        });
        ui.add_space(16.0);
        ui.label(
            RichText::new("Headers, footers, flow regions, tables, groups, and repeaters can be imported and inspected on the canvas. Dedicated visual controls are planned as Studio grows.")
                .color(Color32::from_gray(155))
                .small(),
        );
    }

    fn field_inspector(&mut self, ui: &mut egui::Ui, index: usize) {
        if index >= self.template.fields.len() {
            self.selection = Selection::Document;
            return;
        }
        ui.heading("Data field");
        let field = &mut self.template.fields[index];
        let mut changed = false;
        ui.label("Name");
        changed |= ui.text_edit_singleline(&mut field.name).changed();
        ui.add_space(8.0);
        changed |= enum_combo(
            ui,
            "Field type",
            &mut field.field_type,
            &[
                (FieldType::Text, "Text"),
                (FieldType::Number, "Number"),
                (FieldType::Image, "Image"),
                (FieldType::Boolean, "Boolean"),
                (FieldType::Collection, "Collection"),
            ],
        );
        changed |= ui.checkbox(&mut field.required, "Required").changed();
        ui.add_space(10.0);
        ui.label(
            RichText::new(format!("Use as {{{{{}}}}}", field.name))
                .color(ORANGE)
                .monospace(),
        );
        ui.add_space(18.0);
        if danger_button(ui, "Delete field").clicked() {
            self.template.fields.remove(index);
            self.selection = Selection::Document;
            self.dirty = true;
            return;
        }
        self.dirty |= changed;
    }

    fn element_inspector(&mut self, ui: &mut egui::Ui, index: usize) {
        if index >= self.template.pages[self.current_page].elements.len() {
            self.selection = Selection::Page;
            return;
        }
        let label = element_label(
            &self.template.pages[self.current_page].elements[index],
            index,
        );
        ui.heading(label);
        let changed = element_properties(
            ui,
            &mut self.template.pages[self.current_page].elements[index],
        );
        self.dirty |= changed;
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            if ui.button("Duplicate").clicked() {
                let duplicate = self.template.pages[self.current_page].elements[index].clone();
                self.template.pages[self.current_page]
                    .elements
                    .insert(index + 1, duplicate);
                self.selection = Selection::Element(index + 1);
                self.dirty = true;
            }
            if danger_button(ui, "Delete").clicked() {
                self.template.pages[self.current_page]
                    .elements
                    .remove(index);
                self.selection = Selection::Page;
                self.dirty = true;
            }
        });
    }

    fn show_canvas(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(Frame::new().fill(PANEL_DEEP).inner_margin(Margin::same(0)))
            .show(ctx, |ui| {
                Frame::new()
                    .fill(Color32::from_rgb(32, 37, 42))
                    .inner_margin(Margin::symmetric(16, 8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("PAGE {}", self.current_page + 1))
                                    .strong()
                                    .color(Color32::from_gray(175)),
                            );
                            ui.separator();
                            ui.label(RichText::new("Zoom").color(Color32::from_gray(150)));
                            ui.add(
                                egui::Slider::new(&mut self.zoom, 0.4..=2.0)
                                    .show_value(false)
                                    .custom_formatter(|value, _| format!("{:.0}%", value * 100.0)),
                            );
                            ui.label(format!("{:.0}%", self.zoom * 100.0));
                            if ui.small_button("Fit").clicked() {
                                self.zoom = 1.0;
                            }
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.checkbox(&mut self.print_ready_preview, "Print-ready preview");
                            });
                        });
                    });

                let available = ui.available_size();
                let (workspace, background_response) =
                    ui.allocate_exact_size(available, Sense::click());
                if background_response.clicked() {
                    self.selection = Selection::Page;
                }
                let page_width = self.template.document.width.to_points().max(1.0);
                let page_height = self.template.document.height.to_points().max(1.0);
                let fit = ((workspace.width() - 100.0) / page_width)
                    .min((workspace.height() - 80.0) / page_height)
                    .max(0.05);
                let scale = fit * self.zoom;
                let page_size = Vec2::new(page_width * scale, page_height * scale);
                let page_rect = Rect::from_center_size(workspace.center(), page_size);
                let painter = ui.painter().with_clip_rect(workspace);

                painter.rect_filled(
                    page_rect.translate(Vec2::new(8.0, 10.0)),
                    CornerRadius::same(3),
                    Color32::from_black_alpha(80),
                );
                painter.rect_filled(page_rect, CornerRadius::same(2), CREAM);
                if let Some(bleed) = self.template.document.bleed {
                    let bleed = bleed.to_points() * scale;
                    painter.rect_stroke(
                        page_rect.expand(bleed),
                        CornerRadius::same(2),
                        Stroke::new(1.0_f32, Color32::from_rgb(151, 77, 54)),
                        StrokeKind::Outside,
                    );
                }
                paint_grid(&painter, page_rect, page_width, page_height, scale);

                let elements = self.template.pages[self.current_page].elements.clone();
                for (index, element) in elements.iter().enumerate() {
                    let selected = self.selection == Selection::Element(index);
                    let interaction =
                        paint_element(ui, &painter, page_rect, scale, element, index, selected);
                    if interaction.clicked {
                        self.selection = Selection::Element(index);
                    }
                    if let Some(delta) = interaction.translate {
                        translate_element(
                            &mut self.template.pages[self.current_page].elements[index],
                            delta.x / scale,
                            -delta.y / scale,
                        );
                        self.dirty = true;
                    }
                    if let Some(size) = interaction.resize {
                        resize_element(
                            &mut self.template.pages[self.current_page].elements[index],
                            size.x / scale,
                            size.y / scale,
                        );
                        self.dirty = true;
                    }
                }
            });
    }

    fn show_status(&mut self, ctx: &egui::Context, report: &ValidationReport) {
        egui::TopBottomPanel::bottom("studio-status")
            .exact_height(34.0)
            .frame(
                Frame::new()
                    .fill(CHARCOAL)
                    .inner_margin(Margin::symmetric(14, 7)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let notice_color = match self.notice.kind {
                        NoticeKind::Info => Color32::from_gray(165),
                        NoticeKind::Success => Color32::from_rgb(83, 194, 131),
                        NoticeKind::Error => Color32::from_rgb(242, 111, 111),
                    };
                    ui.label(RichText::new(&self.notice.message).color(notice_color));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let valid = report.is_valid();
                        ui.label(
                            RichText::new(if valid { "READY" } else { "NEEDS ATTENTION" })
                                .color(if valid {
                                    Color32::from_rgb(83, 194, 131)
                                } else {
                                    Color32::from_rgb(242, 111, 111)
                                })
                                .strong(),
                        );
                        ui.separator();
                        ui.label(format!("{} warning(s)", report.warning_count()));
                        ui.label(format!("{} error(s)", report.error_count()));
                        if let Some(diagnostic) = report.diagnostics().first() {
                            ui.separator();
                            ui.label(
                                RichText::new(format!(
                                    "{}: {}",
                                    diagnostic.path, diagnostic.message
                                ))
                                .color(Color32::from_gray(165)),
                            );
                        }
                    });
                });
            });
    }

    fn show_json_window(&mut self, ctx: &egui::Context) {
        if !self.show_json {
            return;
        }
        let mut open = self.show_json;
        let mut apply = false;
        egui::Window::new("Template JSON")
            .open(&mut open)
            .default_width(760.0)
            .default_height(620.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(
                        "Advanced source editor · changes apply only after parsing succeeds",
                    )
                    .color(Color32::from_gray(155)),
                );
                ui.add_space(6.0);
                ScrollArea::vertical().show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.json_buffer)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY)
                            .desired_rows(30),
                    );
                });
                ui.add_space(8.0);
                apply = accent_button(ui, "Apply JSON").clicked();
            });
        self.show_json = open;
        if apply {
            match serde_json::from_str::<Template>(&self.json_buffer) {
                Ok(template) => {
                    self.template = template;
                    self.current_page = self
                        .current_page
                        .min(self.template.pages.len().saturating_sub(1));
                    self.selection = Selection::Document;
                    self.dirty = true;
                    self.show_json = false;
                    self.set_notice(NoticeKind::Success, "Applied template JSON");
                }
                Err(error) => {
                    self.set_notice(NoticeKind::Error, format!("JSON parse failed: {error}"));
                }
            }
        }
    }

    fn show_preview_data_window(&mut self, ctx: &egui::Context) {
        if !self.show_preview_data {
            return;
        }
        let mut open = self.show_preview_data;
        egui::Window::new("Preview data")
            .open(&mut open)
            .default_width(520.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.label(
                    "Enter one JSON object. Its values resolve template variables when rendering a preview PDF.",
                );
                ui.add_space(8.0);
                ui.add(
                    egui::TextEdit::multiline(&mut self.preview_data)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(f32::INFINITY)
                        .desired_rows(18),
                );
                let parse = serde_json::from_str::<DataRow>(&self.preview_data);
                ui.label(
                    RichText::new(match parse {
                        Ok(_) => "Valid preview object".to_owned(),
                        Err(ref error) => format!("Invalid: {error}"),
                    })
                    .color(if parse.is_ok() {
                        Color32::from_rgb(83, 194, 131)
                    } else {
                        Color32::from_rgb(242, 111, 111)
                    }),
                );
            });
        self.show_preview_data = open;
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let (save, save_as, open, duplicate, delete) = ctx.input(|input| {
            (
                input.modifiers.command && input.key_pressed(Key::S) && !input.modifiers.shift,
                input.modifiers.command && input.modifiers.shift && input.key_pressed(Key::S),
                input.modifiers.command && input.key_pressed(Key::O),
                input.modifiers.command && input.key_pressed(Key::D),
                input.key_pressed(Key::Delete) || input.key_pressed(Key::Backspace),
            )
        });
        if save {
            self.save_project(false);
        } else if save_as {
            self.save_project(true);
        } else if open {
            self.open_project();
        }
        if duplicate && !ctx.wants_keyboard_input() {
            if let Selection::Element(index) = self.selection {
                if let Some(element) = self.template.pages[self.current_page]
                    .elements
                    .get(index)
                    .cloned()
                {
                    self.template.pages[self.current_page]
                        .elements
                        .insert(index + 1, element);
                    self.selection = Selection::Element(index + 1);
                    self.dirty = true;
                }
            }
        }
        if delete && !ctx.wants_keyboard_input() {
            if let Selection::Element(index) = self.selection {
                if index < self.template.pages[self.current_page].elements.len() {
                    self.template.pages[self.current_page]
                        .elements
                        .remove(index);
                    self.selection = Selection::Page;
                    self.dirty = true;
                }
            }
        }
    }
}

impl eframe::App for StudioApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.current_page = self
            .current_page
            .min(self.template.pages.len().saturating_sub(1));
        if self.template.pages.is_empty() {
            self.template.pages.push(blank_page());
            self.current_page = 0;
        }
        self.handle_shortcuts(ctx);
        self.show_toolbar(ctx);
        self.show_left_panel(ctx);
        self.show_right_panel(ctx);
        self.show_canvas(ctx);
        let report = validate_template(&self.template);
        self.show_status(ctx, &report);
        self.show_json_window(ctx);
        self.show_preview_data_window(ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(
            storage,
            APP_STATE_KEY,
            &PersistedState {
                template: self.template.clone(),
                current_path: self.current_path.clone(),
                current_page: self.current_page,
                preview_data: self.preview_data.clone(),
            },
        );
    }
}

struct ElementInteraction {
    clicked: bool,
    translate: Option<Vec2>,
    resize: Option<Vec2>,
}

fn paint_element(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    page: Rect,
    scale: f32,
    element: &Element,
    index: usize,
    selected: bool,
) -> ElementInteraction {
    let mut interaction = ElementInteraction {
        clicked: false,
        translate: None,
        resize: None,
    };
    if let Element::Line(line) = element {
        let start = page_point(page, scale, line.x1.to_points(), line.y1.to_points());
        let end = page_point(page, scale, line.x2.to_points(), line.y2.to_points());
        painter.line_segment(
            [start, end],
            Stroke::new(line.width.to_points().max(1.0), parse_color(&line.color)),
        );
        let rect = Rect::from_two_pos(start, end).expand(7.0);
        let response = ui.interact(
            rect,
            Id::new(("canvas-line", index)),
            Sense::click_and_drag(),
        );
        interaction.clicked = response.clicked();
        if response.dragged() {
            interaction.translate = Some(ui.input(|input| input.pointer.delta()));
        }
        if selected {
            painter.rect_stroke(
                rect,
                CornerRadius::same(1),
                Stroke::new(1.5_f32, ORANGE),
                StrokeKind::Outside,
            );
        }
        return interaction;
    }

    let Some(bounds) = element_bounds(element) else {
        return interaction;
    };
    let points = bounds_points(bounds);
    let rect = page_bounds(page, scale, points);
    paint_element_content(painter, rect, scale, element);
    let response = ui.interact(
        rect,
        Id::new(("canvas-element", index)),
        Sense::click_and_drag(),
    );
    interaction.clicked = response.clicked();
    if response.dragged() {
        interaction.translate = Some(ui.input(|input| input.pointer.delta()));
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    if selected {
        painter.rect_stroke(
            rect,
            CornerRadius::same(2),
            Stroke::new(2.0_f32, ORANGE),
            StrokeKind::Outside,
        );
        let handle = Rect::from_center_size(rect.right_top(), Vec2::splat(12.0));
        painter.rect_filled(handle, CornerRadius::same(2), ORANGE);
        let resize = ui.interact(handle, Id::new(("canvas-resize", index)), Sense::drag());
        if resize.dragged() {
            let delta = ui.input(|input| input.pointer.delta());
            interaction.translate = None;
            interaction.resize = Some(Vec2::new(rect.width() + delta.x, rect.height() - delta.y));
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNeSw);
        }
    }
    interaction
}

fn paint_element_content(painter: &egui::Painter, rect: Rect, scale: f32, element: &Element) {
    match element {
        Element::Text(text) => {
            let font_size = (text.font_size.to_points() * scale).clamp(8.0, 34.0);
            painter.text(
                rect.left_top() + Vec2::new(3.0, 2.0),
                Align2::LEFT_TOP,
                &text.value,
                FontId::proportional(font_size),
                parse_color(&text.color),
            );
        }
        Element::Rectangle(rectangle) => {
            painter.rect_filled(
                rect,
                CornerRadius::ZERO,
                rectangle
                    .fill
                    .as_deref()
                    .map(parse_color)
                    .unwrap_or(Color32::TRANSPARENT),
            );
            if let Some(stroke) = &rectangle.stroke {
                painter.rect_stroke(
                    rect,
                    CornerRadius::ZERO,
                    Stroke::new(
                        stroke.width.to_points().max(1.0),
                        parse_color(&stroke.color),
                    ),
                    StrokeKind::Inside,
                );
            }
        }
        Element::Image(image) => paint_placeholder(painter, rect, "IMAGE", &image.source),
        Element::Svg(svg) => paint_placeholder(painter, rect, "VECTOR SVG", &svg.source),
        Element::QrCode(_) => paint_qr_placeholder(painter, rect),
        Element::Barcode(barcode) => paint_barcode_placeholder(painter, rect, &barcode.value),
        Element::Group(_) => paint_placeholder(painter, rect, "GROUP", "composed elements"),
        Element::Stack(_) => paint_placeholder(painter, rect, "FLOW STACK", "paginating region"),
        Element::Table(table) => paint_placeholder(painter, rect, "TABLE", &table.source),
        Element::Line(_) | Element::Repeater(_) | Element::PageBreak => {}
    }
}

fn paint_placeholder(painter: &egui::Painter, rect: Rect, title: &str, detail: &str) {
    painter.rect_filled(
        rect,
        CornerRadius::same(2),
        Color32::from_rgb(226, 228, 225),
    );
    painter.rect_stroke(
        rect,
        CornerRadius::same(2),
        Stroke::new(1.0_f32, Color32::from_rgb(165, 170, 168)),
        StrokeKind::Inside,
    );
    painter.line_segment(
        [rect.left_top(), rect.right_bottom()],
        Stroke::new(1.0_f32, Color32::from_rgb(190, 194, 191)),
    );
    painter.line_segment(
        [rect.right_top(), rect.left_bottom()],
        Stroke::new(1.0_f32, Color32::from_rgb(190, 194, 191)),
    );
    painter.text(
        rect.center() - Vec2::new(0.0, 8.0),
        Align2::CENTER_CENTER,
        title,
        FontId::proportional(12.0),
        CHARCOAL,
    );
    painter.text(
        rect.center() + Vec2::new(0.0, 10.0),
        Align2::CENTER_CENTER,
        detail,
        FontId::proportional(9.0),
        Color32::from_gray(95),
    );
}

fn paint_qr_placeholder(painter: &egui::Painter, rect: Rect) {
    painter.rect_filled(rect, CornerRadius::ZERO, Color32::WHITE);
    let cells = 11;
    let size = rect.width().min(rect.height()) / cells as f32;
    let origin = rect.center() - Vec2::splat(size * cells as f32 / 2.0);
    for y in 0..cells {
        for x in 0..cells {
            let finder = ((x < 3 || x >= cells - 3) && y < 3) || (x < 3 && y >= cells - 3);
            if finder || (x * 3 + y * 5 + x * y) % 4 == 0 {
                painter.rect_filled(
                    Rect::from_min_size(
                        origin + Vec2::new(x as f32 * size, y as f32 * size),
                        Vec2::splat(size + 0.2),
                    ),
                    CornerRadius::ZERO,
                    CHARCOAL,
                );
            }
        }
    }
}

fn paint_barcode_placeholder(painter: &egui::Painter, rect: Rect, value: &str) {
    painter.rect_filled(rect, CornerRadius::ZERO, Color32::WHITE);
    let bars = 43;
    let bar_width = rect.width() / bars as f32;
    for index in 0..bars {
        if (index * 7 + 3) % 5 < 2 {
            painter.rect_filled(
                Rect::from_min_max(
                    Pos2::new(rect.left() + index as f32 * bar_width, rect.top() + 5.0),
                    Pos2::new(
                        rect.left() + (index + 1) as f32 * bar_width,
                        rect.bottom() - 16.0,
                    ),
                ),
                CornerRadius::ZERO,
                CHARCOAL,
            );
        }
    }
    painter.text(
        Pos2::new(rect.center().x, rect.bottom() - 8.0),
        Align2::CENTER_CENTER,
        value,
        FontId::monospace(9.0),
        CHARCOAL,
    );
}

fn paint_grid(painter: &egui::Painter, page: Rect, width: f32, height: f32, scale: f32) {
    if 18.0 * scale < 7.0 {
        return;
    }
    let color = Color32::from_rgba_unmultiplied(80, 87, 91, 20);
    let mut x = 18.0;
    while x < width {
        let screen_x = page.left() + x * scale;
        painter.line_segment(
            [
                Pos2::new(screen_x, page.top()),
                Pos2::new(screen_x, page.bottom()),
            ],
            Stroke::new(0.5_f32, color),
        );
        x += 18.0;
    }
    let mut y = 18.0;
    while y < height {
        let screen_y = page.bottom() - y * scale;
        painter.line_segment(
            [
                Pos2::new(page.left(), screen_y),
                Pos2::new(page.right(), screen_y),
            ],
            Stroke::new(0.5_f32, color),
        );
        y += 18.0;
    }
}

fn page_bounds(page: Rect, scale: f32, bounds: [f32; 4]) -> Rect {
    Rect::from_min_size(
        Pos2::new(
            page.left() + bounds[0] * scale,
            page.bottom() - (bounds[1] + bounds[3]) * scale,
        ),
        Vec2::new(bounds[2] * scale, bounds[3] * scale),
    )
}

fn page_point(page: Rect, scale: f32, x: f32, y: f32) -> Pos2 {
    Pos2::new(page.left() + x * scale, page.bottom() - y * scale)
}

fn document_inspector(ui: &mut egui::Ui, template: &mut Template) -> bool {
    let mut changed = false;
    ui.heading("Document");
    ui.label("Template name");
    changed |= ui.text_edit_singleline(&mut template.name).changed();
    ui.add_space(10.0);
    section_label(ui, "PAGE SIZE");
    ui.horizontal_wrapped(|ui| {
        if ui.small_button("US Letter").clicked() {
            template.document.width = Length::inches(8.5);
            template.document.height = Length::inches(11.0);
            changed = true;
        }
        if ui.small_button("A4").clicked() {
            template.document.width = Length::millimeters(210.0);
            template.document.height = Length::millimeters(297.0);
            changed = true;
        }
        if ui.small_button("Business card").clicked() {
            template.document.width = Length::inches(3.5);
            template.document.height = Length::inches(2.0);
            changed = true;
        }
    });
    changed |= length_editor(ui, "Width", &mut template.document.width, "doc-width");
    changed |= length_editor(ui, "Height", &mut template.document.height, "doc-height");
    let mut has_bleed = template.document.bleed.is_some();
    if ui.checkbox(&mut has_bleed, "Include bleed").changed() {
        template.document.bleed = has_bleed.then(|| Length::inches(0.125));
        changed = true;
    }
    if let Some(bleed) = &mut template.document.bleed {
        changed |= length_editor(ui, "Bleed", bleed, "doc-bleed");
    }
    ui.add_space(12.0);
    section_label(ui, "METADATA");
    changed |= optional_text(ui, "Title", &mut template.document.metadata.title);
    changed |= optional_text(ui, "Author", &mut template.document.metadata.author);
    changed |= optional_text(ui, "Subject", &mut template.document.metadata.subject);
    changed
}

fn element_properties(ui: &mut egui::Ui, element: &mut Element) -> bool {
    let mut changed = false;
    if let Some(bounds) = element_bounds_mut(element) {
        section_label(ui, "POSITION & SIZE");
        egui::Grid::new("bounds-grid")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                changed |= compact_length(ui, "X", &mut bounds.x, "x");
                changed |= compact_length(ui, "Y", &mut bounds.y, "y");
                ui.end_row();
                changed |= compact_length(ui, "W", &mut bounds.width, "w");
                changed |= compact_length(ui, "H", &mut bounds.height, "h");
                ui.end_row();
            });
        ui.add_space(12.0);
    }

    match element {
        Element::Text(text) => {
            section_label(ui, "TEXT");
            ui.label("Value");
            changed |= ui
                .add(
                    egui::TextEdit::multiline(&mut text.value)
                        .desired_width(f32::INFINITY)
                        .desired_rows(4),
                )
                .changed();
            changed |= length_editor(ui, "Font size", &mut text.font_size, "text-font-size");
            changed |= optional_text(ui, "Font family", &mut text.font);
            changed |= enum_combo(
                ui,
                "Style",
                &mut text.font_style,
                &[
                    (FontStyle::Regular, "Regular"),
                    (FontStyle::Bold, "Bold"),
                    (FontStyle::Italic, "Italic"),
                    (FontStyle::BoldItalic, "Bold italic"),
                ],
            );
            changed |= enum_combo(
                ui,
                "Alignment",
                &mut text.align,
                &[
                    (TextAlign::Left, "Left"),
                    (TextAlign::Center, "Center"),
                    (TextAlign::Right, "Right"),
                    (TextAlign::Justify, "Justify"),
                ],
            );
            changed |= enum_combo(
                ui,
                "Overflow",
                &mut text.overflow,
                &[
                    (TextOverflow::Error, "Error"),
                    (TextOverflow::Clip, "Clip"),
                    (TextOverflow::Shrink, "Shrink"),
                ],
            );
            changed |= string_editor(ui, "Color", &mut text.color);
        }
        Element::Rectangle(rectangle) => {
            section_label(ui, "APPEARANCE");
            changed |= optional_text(ui, "Fill", &mut rectangle.fill);
            if let Some(stroke) = &mut rectangle.stroke {
                changed |= string_editor(ui, "Stroke color", &mut stroke.color);
                changed |= length_editor(ui, "Stroke width", &mut stroke.width, "rect-stroke");
                if ui.small_button("Remove stroke").clicked() {
                    rectangle.stroke = None;
                    changed = true;
                }
            } else if ui.small_button("Add stroke").clicked() {
                rectangle.stroke = Some(print_forge_template::Stroke {
                    width: Length::points(1.0),
                    color: "#20252A".to_owned(),
                    dash: DashStyle::Solid,
                });
                changed = true;
            }
        }
        Element::Image(image) => {
            section_label(ui, "IMAGE");
            changed |= string_editor(ui, "Source path", &mut image.source);
            changed |= image_fit_combo(ui, &mut image.fit);
        }
        Element::Svg(svg) => {
            section_label(ui, "VECTOR SVG");
            changed |= string_editor(ui, "Source path", &mut svg.source);
            changed |= image_fit_combo(ui, &mut svg.fit);
        }
        Element::QrCode(qr) => {
            section_label(ui, "QR CODE");
            changed |= string_editor(ui, "Value", &mut qr.value);
            changed |= enum_combo(
                ui,
                "Error correction",
                &mut qr.error_correction,
                &[
                    (QrErrorCorrection::Low, "Low"),
                    (QrErrorCorrection::Medium, "Medium"),
                    (QrErrorCorrection::Quartile, "Quartile"),
                    (QrErrorCorrection::High, "High"),
                ],
            );
            ui.label("Quiet zone (modules)");
            changed |= ui
                .add(egui::DragValue::new(&mut qr.quiet_zone).range(4..=32))
                .changed();
            changed |= string_editor(ui, "Color", &mut qr.color);
            changed |= string_editor(ui, "Background", &mut qr.background);
        }
        Element::Barcode(barcode) => {
            section_label(ui, "CODE 128");
            changed |= string_editor(ui, "Value", &mut barcode.value);
            ui.label("Quiet zone (modules)");
            changed |= ui
                .add(egui::DragValue::new(&mut barcode.quiet_zone).range(10..=64))
                .changed();
            changed |= string_editor(ui, "Color", &mut barcode.color);
            changed |= string_editor(ui, "Background", &mut barcode.background);
        }
        Element::Line(line) => {
            section_label(ui, "LINE");
            changed |= length_editor(ui, "X1", &mut line.x1, "line-x1");
            changed |= length_editor(ui, "Y1", &mut line.y1, "line-y1");
            changed |= length_editor(ui, "X2", &mut line.x2, "line-x2");
            changed |= length_editor(ui, "Y2", &mut line.y2, "line-y2");
            changed |= length_editor(ui, "Width", &mut line.width, "line-width");
            changed |= string_editor(ui, "Color", &mut line.color);
        }
        Element::Group(_)
        | Element::Stack(_)
        | Element::Table(_)
        | Element::Repeater(_)
        | Element::PageBreak => {
            ui.label(
                RichText::new("This advanced element is preserved and previewed. Edit its complete structure in the JSON source view for now.")
                    .color(Color32::from_gray(155)),
            );
        }
    }
    changed
}

fn length_editor(ui: &mut egui::Ui, label: &str, length: &mut Length, id: &str) -> bool {
    ui.label(label);
    ui.horizontal(|ui| {
        let mut changed = ui
            .add(egui::DragValue::new(&mut length.value).speed(0.1))
            .changed();
        changed |= unit_combo(ui, length, id);
        changed
    })
    .inner
}

fn compact_length(ui: &mut egui::Ui, label: &str, length: &mut Length, id: &str) -> bool {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(Color32::from_gray(150)));
        let mut changed = ui
            .add_sized(
                [72.0, 24.0],
                egui::DragValue::new(&mut length.value).speed(0.1),
            )
            .changed();
        changed |= unit_combo(ui, length, id);
        changed
    })
    .inner
}

fn unit_combo(ui: &mut egui::Ui, length: &mut Length, id: &str) -> bool {
    let previous = length.unit;
    ComboBox::from_id_salt(("unit", id))
        .selected_text(unit_label(length.unit))
        .width(42.0)
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut length.unit, Unit::Points, "pt");
            ui.selectable_value(&mut length.unit, Unit::Inches, "in");
            ui.selectable_value(&mut length.unit, Unit::Millimeters, "mm");
        });
    if previous != length.unit {
        let points = Length {
            value: length.value,
            unit: previous,
        }
        .to_points();
        length.value = match length.unit {
            Unit::Points => points,
            Unit::Inches => points / 72.0,
            Unit::Millimeters => points * 25.4 / 72.0,
        };
        true
    } else {
        false
    }
}

fn enum_combo<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut T,
    options: &[(T, &str)],
) -> bool {
    ui.label(label);
    let selected = options
        .iter()
        .find(|(candidate, _)| candidate == value)
        .map(|(_, label)| *label)
        .unwrap_or("Unknown");
    let previous = *value;
    ComboBox::from_id_salt(("enum", label))
        .selected_text(selected)
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            for (candidate, label) in options {
                ui.selectable_value(value, *candidate, *label);
            }
        });
    previous != *value
}

fn image_fit_combo(ui: &mut egui::Ui, fit: &mut ImageFit) -> bool {
    enum_combo(
        ui,
        "Fit",
        fit,
        &[
            (ImageFit::Contain, "Contain"),
            (ImageFit::Cover, "Cover"),
            (ImageFit::Stretch, "Stretch"),
        ],
    )
}

fn string_editor(ui: &mut egui::Ui, label: &str, value: &mut String) -> bool {
    ui.label(label);
    ui.text_edit_singleline(value).changed()
}

fn optional_text(ui: &mut egui::Ui, label: &str, value: &mut Option<String>) -> bool {
    let mut text = value.clone().unwrap_or_default();
    let changed = string_editor(ui, label, &mut text);
    if changed {
        *value = if text.trim().is_empty() {
            None
        } else {
            Some(text)
        };
    }
    changed
}

fn section_label(ui: &mut egui::Ui, label: &str) {
    ui.label(
        RichText::new(label)
            .color(Color32::from_gray(135))
            .size(10.0)
            .strong(),
    );
}

fn selectable_row(ui: &mut egui::Ui, selected: bool, label: &str) -> egui::Response {
    ui.add_sized(
        [ui.available_width(), 28.0],
        egui::Button::new(RichText::new(label).color(if selected {
            CREAM
        } else {
            Color32::from_gray(190)
        }))
        .selected(selected),
    )
}

fn accent_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(egui::Button::new(RichText::new(label).color(Color32::WHITE).strong()).fill(ORANGE))
}

fn danger_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).color(Color32::from_rgb(245, 135, 135)))
            .fill(Color32::from_rgb(65, 38, 40)),
    )
}

fn small_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add_sized([24.0, 22.0], egui::Button::new(label))
}

fn parse_color(value: &str) -> Color32 {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix('#') {
        if hex.len() == 6 {
            if let (Ok(red), Ok(green), Ok(blue)) = (
                u8::from_str_radix(&hex[0..2], 16),
                u8::from_str_radix(&hex[2..4], 16),
                u8::from_str_radix(&hex[4..6], 16),
            ) {
                return Color32::from_rgb(red, green, blue);
            }
        }
    }
    if value.eq_ignore_ascii_case("#ffffff") {
        Color32::WHITE
    } else {
        CHARCOAL
    }
}

const fn unit_label(unit: Unit) -> &'static str {
    match unit {
        Unit::Points => "pt",
        Unit::Inches => "in",
        Unit::Millimeters => "mm",
    }
}

fn serialize_template(template: &Template) -> Result<String, String> {
    let mut json = serde_json::to_string_pretty(template).map_err(|error| error.to_string())?;
    json.push('\n');
    Ok(json)
}

fn safe_template_name(name: &str) -> String {
    format!("{}.json", safe_stem(name))
}

fn safe_stem(name: &str) -> String {
    let mut value = String::new();
    let mut separator = false;
    for character in name.trim().chars() {
        if character.is_ascii_alphanumeric() {
            if separator && !value.is_empty() {
                value.push('-');
            }
            value.push(character.to_ascii_lowercase());
            separator = false;
        } else {
            separator = true;
        }
    }
    if value.is_empty() {
        "print-forge-template".to_owned()
    } else {
        value
    }
}

fn display_name(path: &std::path::Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("template.json")
        .to_owned()
}

fn configure_style(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = PANEL;
    visuals.window_fill = PANEL;
    visuals.extreme_bg_color = PANEL_DEEP;
    visuals.selection.bg_fill = ORANGE;
    visuals.widgets.active.bg_fill = Color32::from_rgb(73, 79, 84);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(62, 69, 75);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(49, 56, 62);
    ctx.set_visuals(visuals);
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = Vec2::new(8.0, 7.0);
    style.spacing.button_padding = Vec2::new(10.0, 6.0);
    ctx.set_style(style);
}

#[cfg(test)]
mod tests {
    use super::{safe_stem, serialize_template};
    use crate::model::starter_template;

    #[test]
    fn serialized_studio_templates_round_trip() {
        let template = starter_template();
        let json = serialize_template(&template).unwrap();
        assert!(json.ends_with('\n'));
        assert_eq!(template, serde_json::from_str(&json).unwrap());
    }

    #[test]
    fn output_names_are_safe() {
        assert_eq!(safe_stem(" ACME / Summer Catalog "), "acme-summer-catalog");
    }
}
