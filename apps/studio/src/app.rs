use std::{
    collections::{BTreeSet, HashMap},
    fs,
    ops::Deref,
    path::Path,
    path::PathBuf,
    sync::Arc,
};

use eframe::egui::{
    self, Align, Align2, Color32, ComboBox, CornerRadius, FontId, Frame, Id, Key, Layout, Margin,
    Pos2, Rect, RichText, ScrollArea, Sense, Stroke, StrokeKind, TextureHandle, TextureOptions,
    Vec2,
};
use print_forge_dataset::DataRow;
use print_forge_engine::{
    BasicLayoutEngine, DrawCommand, ElementLayoutError, LayoutError, LayoutOptions, LineDash,
    ResolvedDocument, ResolvedPage,
};
use print_forge_pdf::{PdfRenderOptions, PdfRenderer};
use print_forge_template::{
    Color as PrintColor, DashStyle, Element, FieldType, FontStyle, ImageFit, Length,
    QrErrorCorrection, Template, TextAlign, TextOverflow, Unit,
};
use print_forge_validation::{ValidationReport, validate_template};
use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
use serde::{Deserialize, Serialize};

use crate::model::{
    AlignMode, DistributionAxis, ElementKind, LayerMove, LineEndpoint, SnapResult, align_elements,
    blank_page, bounds_points, distribute_elements, element_alignment_bounds, element_bounds,
    element_bounds_mut, element_label, element_rotation, move_element, new_element, new_field,
    reorder_element, resize_element, set_element_rotation, snap_point, snap_size, snap_translation,
    starter_template, translate_element, translate_line_endpoint,
};

const APP_STATE_KEY: &str = "print-forge-studio-state";
const ORANGE: Color32 = Color32::from_rgb(244, 91, 32);
const CHARCOAL: Color32 = Color32::from_rgb(31, 36, 41);
const PANEL: Color32 = Color32::from_rgb(38, 44, 50);
const PANEL_DEEP: Color32 = Color32::from_rgb(26, 31, 36);
const CREAM: Color32 = Color32::from_rgb(250, 247, 239);
const HISTORY_LIMIT: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Selection {
    Document,
    Page,
    Field(usize),
    Element(usize),
    Elements {
        indices: BTreeSet<usize>,
        primary: usize,
    },
}

fn remap_selection_after_layer_move(selection: &Selection, from: usize, to: usize) -> Selection {
    let remap = |index: usize| {
        if index == from {
            to
        } else if from < to && index > from && index <= to {
            index - 1
        } else if to < from && index >= to && index < from {
            index + 1
        } else {
            index
        }
    };
    match selection {
        Selection::Element(index) => Selection::Element(remap(*index)),
        Selection::Elements { indices, primary } => Selection::Elements {
            indices: indices.iter().map(|index| remap(*index)).collect(),
            primary: remap(*primary),
        },
        Selection::Document => Selection::Document,
        Selection::Page => Selection::Page,
        Selection::Field(index) => Selection::Field(*index),
    }
}

#[derive(Serialize, Deserialize)]
struct PersistedState {
    template: Template,
    current_path: Option<PathBuf>,
    current_page: usize,
    preview_data: String,
    #[serde(default)]
    guides: Vec<Vec<EditorGuide>>,
    #[serde(default = "default_true")]
    show_guides: bool,
    #[serde(default = "default_true")]
    snap_enabled: bool,
}

const fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum GuideAxis {
    Vertical,
    Horizontal,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
struct EditorGuide {
    axis: GuideAxis,
    position_pt: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct GuideDraft {
    axis: GuideAxis,
    position_pt: f32,
    valid: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ElementDragKind {
    Translate,
    Resize,
    LineEndpoint(LineEndpoint),
}

#[derive(Clone)]
struct ElementDragState {
    page: usize,
    index: usize,
    kind: ElementDragKind,
    original: Element,
    originals: Vec<(usize, Element)>,
    accumulated: Vec2,
}

impl ElementDragState {
    fn advance(&mut self, frame_delta: Vec2) -> (Element, Vec2) {
        self.accumulated += frame_delta;
        (self.original.clone(), self.accumulated)
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
struct SnapFeedback {
    x: Option<f32>,
    y: Option<f32>,
}

impl From<SnapResult> for SnapFeedback {
    fn from(result: SnapResult) -> Self {
        Self {
            x: result.x,
            y: result.y,
        }
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CanvasMode {
    Design,
    Preview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum AssetKind {
    Raster,
    Svg,
}

#[derive(Clone, Copy)]
struct AssetPaint {
    fit: ImageFit,
    kind: AssetKind,
    transform: ScreenTransform,
}

#[derive(Default)]
struct AssetCache {
    textures: HashMap<(PathBuf, AssetKind), Result<TextureHandle, String>>,
}

struct ResolvedPreview {
    document: ResolvedDocument,
    placeholder_variables: Vec<String>,
}

#[derive(Clone)]
struct EditSnapshot {
    template: Template,
    guides: Vec<Vec<EditorGuide>>,
    current_page: usize,
    selection: Selection,
}

impl EditSnapshot {
    fn same_content(&self, other: &Self) -> bool {
        self.template == other.template && self.guides == other.guides
    }
}

#[derive(Default)]
struct EditHistory {
    undo: Vec<EditSnapshot>,
    redo: Vec<EditSnapshot>,
    observed: Option<EditSnapshot>,
    transaction: Option<EditSnapshot>,
}

impl EditHistory {
    fn reset(&mut self, snapshot: EditSnapshot) {
        self.undo.clear();
        self.redo.clear();
        self.transaction = None;
        self.observed = Some(snapshot);
    }

    fn observe(&mut self, snapshot: EditSnapshot, coalesce: bool) {
        let Some(observed) = self.observed.take() else {
            self.observed = Some(snapshot);
            return;
        };
        let content_changed = !observed.same_content(&snapshot);
        if content_changed && coalesce {
            self.transaction.get_or_insert(observed);
        } else if content_changed {
            let before = self.transaction.take().unwrap_or(observed);
            self.push_undo(before);
            self.redo.clear();
        } else if !coalesce
            && let Some(before) = self.transaction.take()
            && !before.same_content(&snapshot)
        {
            self.push_undo(before);
            self.redo.clear();
        }
        self.observed = Some(snapshot);
    }

    fn finish_transaction(&mut self, current: &EditSnapshot) {
        if let Some(before) = self.transaction.take()
            && !before.same_content(current)
        {
            self.push_undo(before);
            self.redo.clear();
        }
        self.observed = Some(current.clone());
    }

    fn undo(&mut self, current: EditSnapshot) -> Option<EditSnapshot> {
        self.finish_transaction(&current);
        let target = self.undo.pop()?;
        self.redo.push(current);
        self.observed = Some(target.clone());
        Some(target)
    }

    fn redo(&mut self, current: EditSnapshot) -> Option<EditSnapshot> {
        self.finish_transaction(&current);
        let target = self.redo.pop()?;
        self.push_undo(current);
        self.observed = Some(target.clone());
        Some(target)
    }

    fn push_undo(&mut self, snapshot: EditSnapshot) {
        if self.undo.len() == HISTORY_LIMIT {
            self.undo.remove(0);
        }
        self.undo.push(snapshot);
    }

    fn can_undo(&self) -> bool {
        !self.undo.is_empty() || self.transaction.is_some()
    }

    fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

impl Deref for ResolvedPreview {
    type Target = ResolvedDocument;

    fn deref(&self) -> &Self::Target {
        &self.document
    }
}

pub struct StudioApp {
    template: Template,
    saved_template: Template,
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
    canvas_mode: CanvasMode,
    preview_page: usize,
    asset_cache: AssetCache,
    guides: Vec<Vec<EditorGuide>>,
    show_guides: bool,
    snap_enabled: bool,
    active_drag: Option<ElementDragState>,
    dragged_layer: Option<usize>,
    guide_draft: Option<GuideDraft>,
    history: EditHistory,
}

impl StudioApp {
    pub fn new(creation: &eframe::CreationContext<'_>) -> Self {
        configure_style(&creation.egui_ctx);
        let persisted = creation
            .storage
            .and_then(|storage| eframe::get_value::<PersistedState>(storage, APP_STATE_KEY));
        let (template, current_path, current_page, preview_data, guides, show_guides, snap_enabled) =
            persisted.map_or_else(
                || {
                    (
                        starter_template(),
                        None,
                        0,
                        "{}".to_owned(),
                        vec![Vec::new()],
                        true,
                        true,
                    )
                },
                |state| {
                    (
                        state.template,
                        state.current_path,
                        state.current_page,
                        state.preview_data,
                        state.guides,
                        state.show_guides,
                        state.snap_enabled,
                    )
                },
            );

        let saved_template = template.clone();
        let mut app = Self {
            template,
            saved_template,
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
            canvas_mode: CanvasMode::Design,
            preview_page: 0,
            asset_cache: AssetCache::default(),
            guides,
            show_guides,
            snap_enabled,
            active_drag: None,
            dragged_layer: None,
            guide_draft: None,
            history: EditHistory::default(),
        };
        app.reset_history();
        app
    }

    fn new_project(&mut self) {
        if !self.confirm_discard() {
            return;
        }
        self.template = starter_template();
        self.saved_template = self.template.clone();
        self.current_path = None;
        self.current_page = 0;
        self.preview_page = 0;
        self.guides = vec![Vec::new()];
        self.active_drag = None;
        self.dragged_layer = None;
        self.guide_draft = None;
        self.asset_cache.textures.clear();
        self.selection = Selection::Document;
        self.dirty = false;
        self.reset_history();
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
                self.saved_template = self.template.clone();
                self.current_path = Some(path.clone());
                self.current_page = 0;
                self.preview_page = 0;
                self.guides = vec![Vec::new(); self.template.pages.len()];
                self.active_drag = None;
                self.dragged_layer = None;
                self.guide_draft = None;
                self.asset_cache.textures.clear();
                self.selection = Selection::Document;
                self.dirty = false;
                self.reset_history();
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
                self.saved_template = self.template.clone();
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

    fn preview_document(&self) -> Result<ResolvedPreview, String> {
        resolve_preview(&self.template, &self.preview_data, self.asset_base())
    }

    fn asset_base(&self) -> PathBuf {
        self.current_path
            .as_deref()
            .and_then(Path::parent)
            .unwrap_or_else(|| Path::new("."))
            .to_owned()
    }

    fn drag_from_origin(
        &mut self,
        index: usize,
        kind: ElementDragKind,
        frame_delta: Vec2,
    ) -> (Element, Vec2, Vec<(usize, Element)>) {
        let should_reset = self.active_drag.as_ref().is_none_or(|drag| {
            drag.page != self.current_page || drag.index != index || drag.kind != kind
        });
        if should_reset {
            let mut indices = self.editable_selected_element_indices();
            if !indices.contains(&index) {
                indices = vec![index];
            }
            let originals = indices
                .into_iter()
                .filter_map(|selected_index| {
                    self.template.pages[self.current_page]
                        .elements
                        .get(selected_index)
                        .cloned()
                        .map(|element| (selected_index, element))
                })
                .collect();
            self.active_drag = Some(ElementDragState {
                page: self.current_page,
                index,
                kind,
                original: self.template.pages[self.current_page].elements[index].clone(),
                originals,
                accumulated: Vec2::ZERO,
            });
        }
        let drag = self.active_drag.as_mut().unwrap();
        let (original, accumulated) = drag.advance(frame_delta);
        (original, accumulated, drag.originals.clone())
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

    fn selected_element_indices(&self) -> Vec<usize> {
        match &self.selection {
            Selection::Element(index) => vec![*index],
            Selection::Elements { indices, .. } => indices.iter().copied().collect(),
            Selection::Document | Selection::Page | Selection::Field(_) => Vec::new(),
        }
    }

    fn editable_selected_element_indices(&self) -> Vec<usize> {
        self.selected_element_indices()
            .into_iter()
            .filter(|index| {
                self.template.pages[self.current_page]
                    .elements
                    .get(*index)
                    .is_some_and(|element| !element.is_locked())
            })
            .collect()
    }

    fn is_element_selected(&self, index: usize) -> bool {
        match &self.selection {
            Selection::Element(selected) => *selected == index,
            Selection::Elements { indices, .. } => indices.contains(&index),
            Selection::Document | Selection::Page | Selection::Field(_) => false,
        }
    }

    fn is_primary_element(&self, index: usize) -> bool {
        match &self.selection {
            Selection::Element(selected) => *selected == index,
            Selection::Elements { primary, .. } => *primary == index,
            Selection::Document | Selection::Page | Selection::Field(_) => false,
        }
    }

    fn select_element(&mut self, index: usize, additive: bool) {
        if !additive {
            self.selection = Selection::Element(index);
            return;
        }
        let mut indices = self
            .selected_element_indices()
            .into_iter()
            .collect::<BTreeSet<_>>();
        if !indices.insert(index) {
            indices.remove(&index);
        }
        self.selection = match indices.len() {
            0 => Selection::Page,
            1 => Selection::Element(*indices.first().unwrap()),
            _ => Selection::Elements {
                indices,
                primary: index,
            },
        };
        self.active_drag = None;
    }

    fn align_selection(&mut self, mode: AlignMode) {
        let indices = self.editable_selected_element_indices();
        if align_elements(
            &mut self.template.pages[self.current_page].elements,
            &indices,
            mode,
        ) {
            self.dirty = true;
        }
    }

    fn distribute_selection(&mut self, axis: DistributionAxis) {
        let indices = self.editable_selected_element_indices();
        if distribute_elements(
            &mut self.template.pages[self.current_page].elements,
            &indices,
            axis,
        ) {
            self.dirty = true;
        }
    }

    fn duplicate_selection(&mut self) {
        let indices = self.editable_selected_element_indices();
        if indices.is_empty() {
            self.set_notice(NoticeKind::Info, "Unlock a layer before duplicating it");
            return;
        }
        let duplicates = indices
            .iter()
            .filter_map(|index| {
                self.template.pages[self.current_page]
                    .elements
                    .get(*index)
                    .cloned()
            })
            .collect::<Vec<_>>();
        let first = self.template.pages[self.current_page].elements.len();
        self.template.pages[self.current_page]
            .elements
            .extend(duplicates);
        let selected =
            (first..self.template.pages[self.current_page].elements.len()).collect::<BTreeSet<_>>();
        self.selection = if selected.len() == 1 {
            Selection::Element(first)
        } else {
            Selection::Elements {
                indices: selected,
                primary: first,
            }
        };
        self.dirty = true;
    }

    fn delete_selection(&mut self) {
        let mut indices = self.editable_selected_element_indices();
        if indices.is_empty() {
            self.set_notice(NoticeKind::Info, "Unlock a layer before deleting it");
            return;
        }
        indices.sort_unstable_by(|left, right| right.cmp(left));
        for index in indices {
            if index < self.template.pages[self.current_page].elements.len() {
                self.template.pages[self.current_page]
                    .elements
                    .remove(index);
            }
        }
        self.selection = Selection::Page;
        self.active_drag = None;
        self.dirty = true;
    }

    fn move_layer(&mut self, from: usize, to: usize) {
        if !move_element(
            &mut self.template.pages[self.current_page].elements,
            from,
            to,
        ) {
            return;
        }
        self.selection = remap_selection_after_layer_move(&self.selection, from, to);
        self.active_drag = None;
        self.dirty = true;
    }

    fn edit_snapshot(&self) -> EditSnapshot {
        EditSnapshot {
            template: self.template.clone(),
            guides: self.guides.clone(),
            current_page: self.current_page,
            selection: self.selection.clone(),
        }
    }

    fn restore_snapshot(&mut self, snapshot: EditSnapshot) {
        self.template = snapshot.template;
        self.guides = snapshot.guides;
        self.current_page = snapshot
            .current_page
            .min(self.template.pages.len().saturating_sub(1));
        self.selection = snapshot.selection;
        self.preview_page = 0;
        self.active_drag = None;
        self.dragged_layer = None;
        self.guide_draft = None;
        self.asset_cache.textures.clear();
        self.dirty = self.template != self.saved_template;
    }

    fn reset_history(&mut self) {
        let snapshot = self.edit_snapshot();
        self.history.reset(snapshot);
    }

    fn undo(&mut self) {
        let current = self.edit_snapshot();
        if let Some(snapshot) = self.history.undo(current) {
            self.restore_snapshot(snapshot);
            self.set_notice(NoticeKind::Success, "Undid last edit");
        }
    }

    fn redo(&mut self) {
        let current = self.edit_snapshot();
        if let Some(snapshot) = self.history.redo(current) {
            self.restore_snapshot(snapshot);
            self.set_notice(NoticeKind::Success, "Redid last edit");
        }
    }

    fn observe_history(&mut self, ctx: &egui::Context) {
        let coalesce = self.active_drag.is_some()
            || ctx.input(|input| input.pointer.any_down())
            || ctx.wants_keyboard_input();
        let snapshot = self.edit_snapshot();
        self.history.observe(snapshot, coalesce);
        self.dirty = self.template != self.saved_template;
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
                    ui.separator();
                    let can_undo = self.history.can_undo();
                    let can_redo = self.history.can_redo();
                    if ui
                        .add_enabled(can_undo, egui::Button::new("Undo"))
                        .on_hover_text("Undo last edit (Cmd/Ctrl+Z)")
                        .clicked()
                    {
                        self.undo();
                    }
                    if ui
                        .add_enabled(can_redo, egui::Button::new("Redo"))
                        .on_hover_text("Redo last edit (Cmd/Ctrl+Shift+Z)")
                        .clicked()
                    {
                        self.redo();
                    }
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
                                self.guides.push(Vec::new());
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
                            self.active_drag = None;
                            self.guide_draft = None;
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
                    let layers = self.template.pages[self.current_page]
                        .elements
                        .iter()
                        .enumerate()
                        .map(|(index, element)| {
                            (
                                index,
                                element_label(element, index),
                                element.is_visible(),
                                element.is_locked(),
                            )
                        })
                        .collect::<Vec<_>>();
                    let mut drop_target = None;
                    for (index, label, visible, locked) in layers.into_iter().rev() {
                        let selected = self.is_element_selected(index);
                        let row = ui.horizontal(|ui| {
                            let visibility = ui
                                .add_sized(
                                    [30.0, 22.0],
                                    egui::Button::new(if visible { "VIS" } else { "HID" }),
                                )
                                .on_hover_text(if visible {
                                    "Hide this layer"
                                } else {
                                    "Show this layer"
                                });
                            let lock = ui
                                .add_sized(
                                    [34.0, 22.0],
                                    egui::Button::new(if locked { "LOCK" } else { "OPEN" }),
                                )
                                .on_hover_text(if locked {
                                    "Unlock this layer"
                                } else {
                                    "Lock this layer"
                                });
                            let drag = ui
                                .add(egui::Label::new("::").sense(if locked {
                                    Sense::hover()
                                } else {
                                    Sense::drag()
                                }))
                                .on_hover_text(if locked {
                                    "Unlock this layer to change paint order"
                                } else {
                                    "Drag to change paint order"
                                });
                            let text = RichText::new(label).color(if visible {
                                Color32::from_gray(225)
                            } else {
                                Color32::from_gray(115)
                            });
                            let select = ui.selectable_label(selected, text);
                            (visibility, lock, drag, select)
                        });
                        let (visibility, lock, drag, select) = row.inner;
                        if visibility.clicked()
                            && let Some(element) = self.template.pages[self.current_page]
                                .elements
                                .get_mut(index)
                        {
                            element.set_visible(!visible);
                            self.dirty = true;
                        }
                        if lock.clicked()
                            && let Some(element) = self.template.pages[self.current_page]
                                .elements
                                .get_mut(index)
                        {
                            element.set_locked(!locked);
                            self.active_drag = None;
                            self.dirty = true;
                        }
                        if select.clicked() {
                            let additive = ui.input(|input| input.modifiers.shift);
                            self.select_element(index, additive);
                        }
                        if drag.drag_started() {
                            self.dragged_layer = Some(index);
                        }
                        if self.dragged_layer.is_some() && row.response.hovered() {
                            drop_target = Some(index);
                            ui.painter().rect_stroke(
                                row.response.rect.expand(2.0),
                                CornerRadius::same(2),
                                Stroke::new(1.0_f32, ORANGE),
                                StrokeKind::Outside,
                            );
                        }
                    }
                    if ui.input(|input| input.pointer.any_released()) {
                        if let (Some(from), Some(to)) = (self.dragged_layer.take(), drop_target) {
                            self.move_layer(from, to);
                        } else {
                            self.dragged_layer = None;
                        }
                    }
                    if let Selection::Element(index) = self.selection.clone() {
                        let layer_count = self.template.pages[self.current_page].elements.len();
                        let selected_locked = self.template.pages[self.current_page]
                            .elements
                            .get(index)
                            .is_some_and(Element::is_locked);
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if ui
                                .add_enabled(
                                    index > 0 && !selected_locked,
                                    egui::Button::new("Back"),
                                )
                                .on_hover_text("Send behind every other layer")
                                .clicked()
                            {
                                let selected = reorder_element(
                                    &mut self.template.pages[self.current_page].elements,
                                    index,
                                    LayerMove::Back,
                                );
                                self.selection = Selection::Element(selected);
                                self.dirty = true;
                            }
                            if ui
                                .add_enabled(
                                    index > 0 && !selected_locked,
                                    egui::Button::new("Lower"),
                                )
                                .on_hover_text("Move backward one layer")
                                .clicked()
                            {
                                let selected = reorder_element(
                                    &mut self.template.pages[self.current_page].elements,
                                    index,
                                    LayerMove::Backward,
                                );
                                self.selection = Selection::Element(selected);
                                self.dirty = true;
                            }
                            if ui
                                .add_enabled(
                                    index + 1 < layer_count && !selected_locked,
                                    egui::Button::new("Raise"),
                                )
                                .on_hover_text("Move forward one layer")
                                .clicked()
                            {
                                let selected = reorder_element(
                                    &mut self.template.pages[self.current_page].elements,
                                    index,
                                    LayerMove::Forward,
                                );
                                self.selection = Selection::Element(selected);
                                self.dirty = true;
                            }
                            if ui
                                .add_enabled(
                                    index + 1 < layer_count && !selected_locked,
                                    egui::Button::new("Front"),
                                )
                                .on_hover_text("Bring in front of every other layer")
                                .clicked()
                            {
                                let selected = reorder_element(
                                    &mut self.template.pages[self.current_page].elements,
                                    index,
                                    LayerMove::Front,
                                );
                                self.selection = Selection::Element(selected);
                                self.dirty = true;
                            }
                        });
                        ui.small("Topmost layer paints in front.");
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
                ScrollArea::vertical().show(ui, |ui| match self.selection.clone() {
                    Selection::Document => {
                        if document_inspector(ui, &mut self.template) {
                            self.dirty = true;
                        }
                    }
                    Selection::Page => self.page_inspector(ui),
                    Selection::Field(index) => self.field_inspector(ui, index),
                    Selection::Element(index) => self.element_inspector(ui, index),
                    Selection::Elements { indices, .. } => {
                        self.multi_element_inspector(ui, &indices)
                    }
                });
            });
    }

    fn multi_element_inspector(&mut self, ui: &mut egui::Ui, indices: &BTreeSet<usize>) {
        ui.heading(format!("{} layers selected", indices.len()));
        ui.label(
            RichText::new("Shift-click the canvas or Layers list to change the selection.")
                .color(Color32::from_gray(155)),
        );
        ui.add_space(14.0);
        let editable_count = self.editable_selected_element_indices().len();
        if editable_count != indices.len() {
            ui.label(
                RichText::new(format!(
                    "{} locked layer(s) will stay unchanged.",
                    indices.len() - editable_count
                ))
                .color(Color32::from_gray(155))
                .small(),
            );
            ui.add_space(8.0);
        }
        self.alignment_controls(ui, editable_count);
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            if ui.button("Duplicate selected").clicked() {
                self.duplicate_selection();
            }
            if danger_button(ui, "Delete selected").clicked() {
                self.delete_selection();
            }
        });
    }

    fn alignment_controls(&mut self, ui: &mut egui::Ui, selected_count: usize) {
        section_label(ui, "ALIGN");
        ui.horizontal_wrapped(|ui| {
            for (label, mode, help) in [
                ("Left", AlignMode::Left, "Align left edges"),
                (
                    "H center",
                    AlignMode::HorizontalCenter,
                    "Align horizontal centers",
                ),
                ("Right", AlignMode::Right, "Align right edges"),
                ("Bottom", AlignMode::Bottom, "Align bottom edges"),
                (
                    "V center",
                    AlignMode::VerticalCenter,
                    "Align vertical centers",
                ),
                ("Top", AlignMode::Top, "Align top edges"),
            ] {
                if ui
                    .add_enabled(selected_count >= 2, egui::Button::new(label))
                    .on_hover_text(help)
                    .clicked()
                {
                    self.align_selection(mode);
                }
            }
        });
        ui.add_space(10.0);
        section_label(ui, "DISTRIBUTE");
        ui.horizontal(|ui| {
            if ui
                .add_enabled(selected_count >= 3, egui::Button::new("Horizontal gaps"))
                .on_hover_text("Distribute with equal horizontal gaps")
                .clicked()
            {
                self.distribute_selection(DistributionAxis::Horizontal);
            }
            if ui
                .add_enabled(selected_count >= 3, egui::Button::new("Vertical gaps"))
                .on_hover_text("Distribute with equal vertical gaps")
                .clicked()
            {
                self.distribute_selection(DistributionAxis::Vertical);
            }
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
            let guides = self
                .guides
                .get(self.current_page)
                .cloned()
                .unwrap_or_default();
            self.guides.insert(self.current_page + 1, guides);
            self.current_page += 1;
            self.dirty = true;
        }
        ui.add_enabled_ui(self.template.pages.len() > 1, |ui| {
            if danger_button(ui, "Delete page").clicked() {
                self.template.pages.remove(self.current_page);
                if self.current_page < self.guides.len() {
                    self.guides.remove(self.current_page);
                }
                self.current_page = self.current_page.min(self.template.pages.len() - 1);
                self.selection = Selection::Page;
                self.dirty = true;
            }
        });
        ui.add_space(16.0);
        section_label(ui, "NON-PRINTING GUIDES");
        let width = self.template.document.width.to_points();
        let height = self.template.document.height.to_points();
        ui.horizontal_wrapped(|ui| {
            if ui.button("+ V center").clicked() {
                push_editor_guide(
                    &mut self.guides[self.current_page],
                    EditorGuide {
                        axis: GuideAxis::Vertical,
                        position_pt: self.template.document.width.to_points() / 2.0,
                    },
                );
            }
            if ui.button("+ H center").clicked() {
                push_editor_guide(
                    &mut self.guides[self.current_page],
                    EditorGuide {
                        axis: GuideAxis::Horizontal,
                        position_pt: self.template.document.height.to_points() / 2.0,
                    },
                );
            }
            if ui.button("+ Margins").clicked() {
                let horizontal_margin = 18.0_f32.min(width / 2.0);
                let vertical_margin = 18.0_f32.min(height / 2.0);
                for guide in [
                    EditorGuide {
                        axis: GuideAxis::Vertical,
                        position_pt: horizontal_margin,
                    },
                    EditorGuide {
                        axis: GuideAxis::Vertical,
                        position_pt: width - horizontal_margin,
                    },
                    EditorGuide {
                        axis: GuideAxis::Horizontal,
                        position_pt: vertical_margin,
                    },
                    EditorGuide {
                        axis: GuideAxis::Horizontal,
                        position_pt: height - vertical_margin,
                    },
                ] {
                    push_editor_guide(&mut self.guides[self.current_page], guide);
                }
            }
            let has_guides = !self.guides[self.current_page].is_empty();
            if ui
                .add_enabled(has_guides, egui::Button::new("Clear"))
                .clicked()
            {
                self.guides[self.current_page].clear();
            }
        });
        let mut remove_guide = None;
        for (index, guide) in self.guides[self.current_page].iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.label(match guide.axis {
                    GuideAxis::Vertical => "V",
                    GuideAxis::Horizontal => "H",
                });
                let maximum = match guide.axis {
                    GuideAxis::Vertical => width,
                    GuideAxis::Horizontal => height,
                };
                ui.add(
                    egui::DragValue::new(&mut guide.position_pt)
                        .range(0.0..=maximum)
                        .speed(1.0)
                        .suffix(" pt"),
                );
                if ui.small_button("×").on_hover_text("Delete guide").clicked() {
                    remove_guide = Some(index);
                }
            });
        }
        if let Some(index) = remove_guide {
            self.guides[self.current_page].remove(index);
        }
        ui.label(
            RichText::new(
                "Drag guides directly on the canvas. Guides are saved in Studio but never printed.",
            )
            .color(Color32::from_gray(145))
            .small(),
        );
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
        let element = &mut self.template.pages[self.current_page].elements[index];
        let mut changed = layer_properties(ui, element);
        let locked = element.is_locked();
        if locked {
            ui.label(
                RichText::new("Unlock this layer to edit its content or geometry.")
                    .color(Color32::from_gray(155))
                    .small(),
            );
            ui.add_space(12.0);
        }
        changed |= ui
            .add_enabled_ui(!locked, |ui| element_properties(ui, element))
            .inner;
        self.dirty |= changed;
        ui.add_space(16.0);
        ui.add_enabled_ui(!locked, |ui| {
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
                    self.delete_selection();
                }
            });
        });
    }

    fn show_canvas(&mut self, ctx: &egui::Context) {
        let preview = self.preview_document();
        if let Ok(document) = &preview {
            self.preview_page = self
                .preview_page
                .min(document.pages.len().saturating_sub(1));
        }
        egui::CentralPanel::default()
            .frame(Frame::new().fill(PANEL_DEEP).inner_margin(Margin::same(0)))
            .show(ctx, |ui| {
                Frame::new()
                    .fill(Color32::from_rgb(32, 37, 42))
                    .inner_margin(Margin::symmetric(16, 8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if ui
                                .selectable_label(self.canvas_mode == CanvasMode::Design, "Design")
                                .clicked()
                            {
                                self.canvas_mode = CanvasMode::Design;
                            }
                            if ui
                                .selectable_label(
                                    self.canvas_mode == CanvasMode::Preview,
                                    "Rendered preview",
                                )
                                .clicked()
                            {
                                self.canvas_mode = CanvasMode::Preview;
                            }
                            ui.separator();
                            let page_count = if self.canvas_mode == CanvasMode::Preview {
                                preview.as_ref().map_or(1, |document| document.pages.len())
                            } else {
                                self.template.pages.len()
                            };
                            let shown_page = if self.canvas_mode == CanvasMode::Preview {
                                self.preview_page
                            } else {
                                self.current_page
                            };
                            ui.label(
                                RichText::new(format!(
                                    "PAGE {} OF {}",
                                    shown_page + 1,
                                    page_count.max(1)
                                ))
                                .strong()
                                .color(Color32::from_gray(175)),
                            );
                            if self.canvas_mode == CanvasMode::Preview {
                                let can_go_back = self.preview_page > 0;
                                if ui
                                    .add_enabled(can_go_back, egui::Button::new("‹"))
                                    .clicked()
                                {
                                    self.preview_page -= 1;
                                }
                                let can_go_forward = self.preview_page + 1 < page_count;
                                if ui
                                    .add_enabled(can_go_forward, egui::Button::new("›"))
                                    .clicked()
                                {
                                    self.preview_page += 1;
                                }
                            }
                            if let Ok(preview) = &preview
                                && !preview.placeholder_variables.is_empty()
                            {
                                let count = preview.placeholder_variables.len();
                                ui.label(
                                    RichText::new(format!(
                                        "{count} missing value{}",
                                        if count == 1 { "" } else { "s" }
                                    ))
                                    .color(Color32::from_rgb(235, 164, 70)),
                                )
                                .on_hover_text(format!(
                                    "Using placeholders for: {}",
                                    preview.placeholder_variables.join(", ")
                                ));
                            }
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
                            ui.separator();
                            ui.toggle_value(&mut self.show_guides, "Guides")
                                .on_hover_text(
                                    "Show rulers and non-printing layout guides (Cmd/Ctrl+;)",
                                );
                            ui.toggle_value(&mut self.snap_enabled, "Snap")
                                .on_hover_text(
                                    "Snap to page, guides, and other layers (Cmd/Ctrl+Shift+;); hold Option/Alt to bypass",
                                );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.checkbox(&mut self.print_ready_preview, "PDF/X export");
                                if ui.small_button("Refresh assets").clicked() {
                                    self.asset_cache.textures.clear();
                                }
                            });
                        });
                    });

                let available = ui.available_size();
                let (workspace, background_response) =
                    ui.allocate_exact_size(available, Sense::click());
                if background_response.clicked() && self.canvas_mode == CanvasMode::Design {
                    self.selection = Selection::Page;
                }
                let (page_width, page_height) = preview.as_ref().map_or_else(
                    |_| {
                        (
                            self.template.document.width.to_points().max(1.0),
                            self.template.document.height.to_points().max(1.0),
                        )
                    },
                    |document| (document.width_pt.max(1.0), document.height_pt.max(1.0)),
                );
                let bleed_points = preview.as_ref().map_or_else(
                    |_| {
                        self.template
                            .document
                            .bleed
                            .map_or(0.0, |bleed| bleed.to_points())
                    },
                    |document| document.bleed_pt,
                );
                let media_width = page_width + bleed_points * 2.0;
                let media_height = page_height + bleed_points * 2.0;
                let fit = ((workspace.width() - 100.0) / media_width)
                    .min((workspace.height() - 80.0) / media_height)
                    .max(0.05);
                let scale = fit * self.zoom;
                let page_size = Vec2::new(page_width * scale, page_height * scale);
                let page_rect = Rect::from_center_size(workspace.center(), page_size);
                let painter = ui.painter().with_clip_rect(workspace);
                let media_rect = page_rect.expand(bleed_points * scale);

                painter.rect_filled(
                    media_rect.translate(Vec2::new(8.0, 10.0)),
                    CornerRadius::same(3),
                    Color32::from_black_alpha(80),
                );
                painter.rect_filled(media_rect, CornerRadius::same(2), Color32::WHITE);
                if bleed_points > 0.0 {
                    painter.rect_stroke(
                        media_rect,
                        CornerRadius::same(2),
                        Stroke::new(1.0_f32, Color32::from_rgb(151, 77, 54)),
                        StrokeKind::Outside,
                    );
                    paint_rect_stroke(
                        &painter,
                        page_rect,
                        Stroke::new(0.75_f32, Color32::from_gray(175)),
                        LineDash::Dashed,
                    );
                }
                if self.canvas_mode == CanvasMode::Design {
                    paint_grid(&painter, page_rect, page_width, page_height, scale);
                }

                let resolved_page = preview.as_ref().ok().and_then(|document| {
                    if self.canvas_mode == CanvasMode::Preview {
                        document.pages.get(self.preview_page)
                    } else {
                        resolved_template_page(document, self.current_page)
                    }
                });
                let resolved = if let Some(page) = resolved_page {
                    let document_painter = painter.with_clip_rect(media_rect);
                    paint_resolved_page(
                        ctx,
                        &document_painter,
                        page_rect,
                        scale,
                        page,
                        &mut self.asset_cache,
                    );
                    true
                } else {
                    false
                };

                if self.canvas_mode == CanvasMode::Design {
                    let elements = self.template.pages[self.current_page].elements.clone();
                    let visible_guides = if self.show_guides {
                        self.guides[self.current_page].clone()
                    } else {
                        Vec::new()
                    };
                    let snapping_active = self.snap_enabled
                        && !ctx.input(|input| input.modifiers.alt);
                    let snap_threshold = 7.0 / scale;
                    let mut snap_feedback = SnapFeedback::default();
                    for (index, element) in elements.iter().enumerate() {
                        if !element.is_visible() {
                            continue;
                        }
                        let selected = self.is_element_selected(index);
                        let interaction = paint_element(
                            ui,
                            &painter,
                            page_rect,
                            scale,
                            element,
                            ElementPaintOptions {
                                index,
                                selected,
                                primary: self.is_primary_element(index),
                                locked: element.is_locked(),
                                paint_content: !resolved,
                            },
                        );
                        if interaction.clicked {
                            let additive = ui.input(|input| input.modifiers.shift);
                            self.select_element(index, additive);
                        }
                        if let Some(delta) = interaction.translate {
                            let (mut updated, total_delta, originals) = self.drag_from_origin(
                                index,
                                ElementDragKind::Translate,
                                delta,
                            );
                            let moving_indices = originals
                                .iter()
                                .map(|(selected_index, _)| *selected_index)
                                .collect::<Vec<_>>();
                            let raw_delta = [total_delta.x / scale, -total_delta.y / scale];
                            let snapped = if snapping_active {
                                let (x_targets, y_targets) = alignment_targets(
                                    &elements,
                                    &moving_indices,
                                    page_width,
                                    page_height,
                                    &visible_guides,
                                );
                                element_alignment_bounds(&updated).map_or(
                                    SnapResult {
                                        delta: raw_delta,
                                        x: None,
                                        y: None,
                                    },
                                    |bounds| {
                                        snap_translation(
                                            bounds,
                                            raw_delta,
                                            &x_targets,
                                            &y_targets,
                                            snap_threshold,
                                        )
                                    },
                                )
                            } else {
                                SnapResult {
                                    delta: raw_delta,
                                    x: None,
                                    y: None,
                                }
                            };
                            if moving_indices.len() > 1 {
                                for (selected_index, mut original) in originals {
                                    translate_element(
                                        &mut original,
                                        snapped.delta[0],
                                        snapped.delta[1],
                                    );
                                    self.template.pages[self.current_page].elements
                                        [selected_index] = original;
                                }
                            } else {
                                translate_element(
                                    &mut updated,
                                    snapped.delta[0],
                                    snapped.delta[1],
                                );
                                self.template.pages[self.current_page].elements[index] = updated;
                            }
                            snap_feedback = snapped.into();
                            self.dirty = true;
                        }
                        if let Some(delta) = interaction.resize {
                            let (mut updated, total_delta, _) =
                                self.drag_from_origin(index, ElementDragKind::Resize, delta);
                            let original_bounds = element_bounds(&updated).map(bounds_points);
                            let raw_delta = [total_delta.x / scale, total_delta.y / scale];
                            let snapped = if snapping_active
                                && element_rotation(&updated).is_some_and(|angle| angle == 0.0)
                            {
                                let (x_targets, y_targets) = alignment_targets(
                                    &elements,
                                    &[index],
                                    page_width,
                                    page_height,
                                    &visible_guides,
                                );
                                original_bounds.map_or(
                                    SnapResult {
                                        delta: raw_delta,
                                        x: None,
                                        y: None,
                                    },
                                    |bounds| {
                                        snap_size(
                                            bounds,
                                            raw_delta,
                                            &x_targets,
                                            &y_targets,
                                            snap_threshold,
                                        )
                                    },
                                )
                            } else {
                                SnapResult {
                                    delta: raw_delta,
                                    x: None,
                                    y: None,
                                }
                            };
                            if let Some(bounds) = original_bounds {
                                resize_element(
                                    &mut updated,
                                    bounds[2] + snapped.delta[0],
                                    bounds[3] + snapped.delta[1],
                                );
                                self.template.pages[self.current_page].elements[index] = updated;
                                snap_feedback = snapped.into();
                            }
                            self.dirty = true;
                        }
                        if let Some((endpoint, delta)) = interaction.line_endpoint {
                            let (mut updated, total_delta, _) = self.drag_from_origin(
                                index,
                                ElementDragKind::LineEndpoint(endpoint),
                                delta,
                            );
                            let raw_delta = [total_delta.x / scale, -total_delta.y / scale];
                            let snapped = if snapping_active {
                                let (x_targets, y_targets) = alignment_targets(
                                    &elements,
                                    &[index],
                                    page_width,
                                    page_height,
                                    &visible_guides,
                                );
                                line_endpoint_position(&updated, endpoint).map_or(
                                    SnapResult {
                                        delta: raw_delta,
                                        x: None,
                                        y: None,
                                    },
                                    |point| {
                                        snap_point(
                                            point,
                                            raw_delta,
                                            &x_targets,
                                            &y_targets,
                                            snap_threshold,
                                        )
                                    },
                                )
                            } else {
                                SnapResult {
                                    delta: raw_delta,
                                    x: None,
                                    y: None,
                                }
                            };
                            translate_line_endpoint(
                                &mut updated,
                                endpoint,
                                snapped.delta[0],
                                snapped.delta[1],
                            );
                            self.template.pages[self.current_page].elements[index] = updated;
                            snap_feedback = snapped.into();
                            self.dirty = true;
                        }
                        if let Some(rotation) = interaction.rotation {
                            set_element_rotation(
                                &mut self.template.pages[self.current_page].elements[index],
                                rotation,
                            );
                            self.active_drag = None;
                            self.dirty = true;
                        }
                        if interaction.drag_stopped {
                            self.active_drag = None;
                        }
                    }
                    if self.show_guides {
                        paint_and_interact_rulers(
                            ui,
                            &painter,
                            page_rect,
                            scale,
                            &mut self.guides[self.current_page],
                            &mut self.guide_draft,
                        );
                        paint_and_interact_guides(
                            ui,
                            &painter,
                            page_rect,
                            scale,
                            page_width,
                            page_height,
                            &mut self.guides[self.current_page],
                        );
                    } else {
                        self.guide_draft = None;
                    }
                    paint_snap_feedback(&painter, page_rect, scale, snap_feedback);
                } else if let Err(error) = &preview {
                    paint_preview_error(&painter, page_rect, error);
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
                    self.preview_page = 0;
                    self.asset_cache.textures.clear();
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
                    "Enter one JSON object. Its values update the rendered canvas and preview PDF.",
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
        let (undo, redo, save, save_as, open, duplicate, delete, toggle_guides, toggle_snap) = ctx
            .input(|input| {
                (
                    input.modifiers.command && !input.modifiers.shift && input.key_pressed(Key::Z),
                    input.modifiers.command
                        && ((input.modifiers.shift && input.key_pressed(Key::Z))
                            || input.key_pressed(Key::Y)),
                    input.modifiers.command && input.key_pressed(Key::S) && !input.modifiers.shift,
                    input.modifiers.command && input.modifiers.shift && input.key_pressed(Key::S),
                    input.modifiers.command && input.key_pressed(Key::O),
                    input.modifiers.command && input.key_pressed(Key::D),
                    input.key_pressed(Key::Delete) || input.key_pressed(Key::Backspace),
                    input.modifiers.command
                        && !input.modifiers.shift
                        && input.key_pressed(Key::Semicolon),
                    input.modifiers.command
                        && input.modifiers.shift
                        && input.key_pressed(Key::Semicolon),
                )
            });
        if undo {
            self.undo();
        } else if redo {
            self.redo();
        } else if save {
            self.save_project(false);
        } else if save_as {
            self.save_project(true);
        } else if open {
            self.open_project();
        }
        if duplicate && !ctx.wants_keyboard_input() {
            self.duplicate_selection();
        }
        if delete && !ctx.wants_keyboard_input() {
            self.delete_selection();
        }
        if !ctx.wants_keyboard_input() {
            let (select_all, clear_selection, nudge) = ctx.input(|input| {
                let amount = if input.modifiers.shift { 10.0 } else { 1.0 };
                let nudge = if input.key_pressed(Key::ArrowLeft) {
                    Some((-amount, 0.0))
                } else if input.key_pressed(Key::ArrowRight) {
                    Some((amount, 0.0))
                } else if input.key_pressed(Key::ArrowUp) {
                    Some((0.0, amount))
                } else if input.key_pressed(Key::ArrowDown) {
                    Some((0.0, -amount))
                } else {
                    None
                };
                (
                    input.modifiers.command && input.key_pressed(Key::A),
                    input.key_pressed(Key::Escape),
                    nudge,
                )
            });
            if select_all {
                let indices = (0..self.template.pages[self.current_page].elements.len())
                    .collect::<BTreeSet<_>>();
                self.selection = match indices.len() {
                    0 => Selection::Page,
                    1 => Selection::Element(0),
                    _ => Selection::Elements {
                        indices,
                        primary: 0,
                    },
                };
            }
            if clear_selection {
                self.selection = Selection::Page;
                self.active_drag = None;
            }
            if let Some((dx, dy)) = nudge {
                let indices = self.editable_selected_element_indices();
                for index in indices {
                    if let Some(element) = self.template.pages[self.current_page]
                        .elements
                        .get_mut(index)
                    {
                        translate_element(element, dx, dy);
                        self.dirty = true;
                    }
                }
            }
            if toggle_guides {
                self.show_guides = !self.show_guides;
            }
            if toggle_snap {
                self.snap_enabled = !self.snap_enabled;
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
        self.guides.resize_with(self.template.pages.len(), Vec::new);
        self.guides.truncate(self.template.pages.len());
        self.handle_shortcuts(ctx);
        self.show_toolbar(ctx);
        self.show_left_panel(ctx);
        self.show_right_panel(ctx);
        self.show_canvas(ctx);
        let report = validate_template(&self.template);
        self.show_status(ctx, &report);
        self.show_json_window(ctx);
        self.show_preview_data_window(ctx);
        self.observe_history(ctx);
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
                guides: self.guides.clone(),
                show_guides: self.show_guides,
                snap_enabled: self.snap_enabled,
            },
        );
    }
}

fn resolve_preview(
    template: &Template,
    preview_data: &str,
    asset_base: PathBuf,
) -> Result<ResolvedPreview, String> {
    let mut data = serde_json::from_str::<DataRow>(preview_data)
        .map_err(|error| format!("Preview data is not a JSON object: {error}"))?;
    let options = LayoutOptions { asset_base };
    let mut placeholder_variables = BTreeSet::new();
    for _ in 0..128 {
        match BasicLayoutEngine.layout_with_options(template, &data, &options) {
            Ok(document) => {
                return Ok(ResolvedPreview {
                    document,
                    placeholder_variables: placeholder_variables.into_iter().collect(),
                });
            }
            Err(error) => {
                let Some(variable) = missing_layout_variable(&error) else {
                    return Err(format!("Preview layout failed: {error}"));
                };
                let value = if is_collection_variable(template, variable) {
                    serde_json::Value::Array(Vec::new())
                } else if is_color_variable(template, variable) {
                    serde_json::Value::String("#808080".to_owned())
                } else {
                    serde_json::Value::String(format!("{{{{{variable}}}}}"))
                };
                if !insert_missing_preview_value(&mut data, variable, value) {
                    return Err(format!("Preview layout failed: {error}"));
                }
                placeholder_variables.insert(variable.to_owned());
            }
        }
    }
    Err(
        "Preview needs more than 128 placeholder values; add representative preview data"
            .to_owned(),
    )
}

fn missing_layout_variable(error: &LayoutError) -> Option<&str> {
    let LayoutError::Element { source, .. } = error else {
        return None;
    };
    missing_element_variable(source)
}

fn missing_element_variable(error: &ElementLayoutError) -> Option<&str> {
    match error {
        ElementLayoutError::MissingVariable(variable) => Some(variable),
        ElementLayoutError::Nested { source, .. } => missing_element_variable(source),
        _ => None,
    }
}

fn insert_missing_preview_value(data: &mut DataRow, path: &str, value: serde_json::Value) -> bool {
    let segments = path.split('.').collect::<Vec<_>>();
    insert_missing_json_path(data, &segments, value)
}

fn insert_missing_json_path(
    object: &mut serde_json::Map<String, serde_json::Value>,
    segments: &[&str],
    value: serde_json::Value,
) -> bool {
    let Some((segment, remaining)) = segments.split_first() else {
        return false;
    };
    if remaining.is_empty() {
        if object.contains_key(*segment) {
            return false;
        }
        object.insert((*segment).to_owned(), value);
        return true;
    }
    let child = object
        .entry((*segment).to_owned())
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    let Some(child) = child.as_object_mut() else {
        return false;
    };
    insert_missing_json_path(child, remaining, value)
}

fn is_collection_variable(template: &Template, variable: &str) -> bool {
    template
        .fields
        .iter()
        .any(|field| field.name == variable && field.field_type == FieldType::Collection)
        || template.pages.iter().any(|page| {
            page.header
                .iter()
                .chain(&page.elements)
                .chain(&page.footer)
                .any(|element| element_uses_collection(element, variable))
        })
}

fn element_uses_collection(element: &Element, variable: &str) -> bool {
    match element {
        Element::Table(table) => table.source == variable,
        Element::Repeater(repeater) => {
            repeater.source == variable || element_uses_collection(&repeater.template, variable)
        }
        Element::Group(group) => group
            .children
            .iter()
            .any(|child| element_uses_collection(child, variable)),
        Element::Stack(stack) => stack
            .children
            .iter()
            .any(|child| element_uses_collection(child, variable)),
        _ => false,
    }
}

fn is_color_variable(template: &Template, variable: &str) -> bool {
    template.pages.iter().any(|page| {
        page.header
            .iter()
            .chain(&page.elements)
            .chain(&page.footer)
            .any(|element| element_uses_color_variable(element, variable))
    })
}

fn element_uses_color_variable(element: &Element, variable: &str) -> bool {
    let uses = |value: &str| first_template_variable(value) == Some(variable);
    let stroke_uses = |stroke: &print_forge_template::Stroke| uses(&stroke.color);

    match element {
        Element::Text(text) => uses(&text.color),
        Element::Rectangle(rectangle) => {
            rectangle.fill.as_deref().is_some_and(uses)
                || rectangle.stroke.as_ref().is_some_and(stroke_uses)
        }
        Element::Line(line) => uses(&line.color),
        Element::QrCode(qr_code) => uses(&qr_code.color) || uses(&qr_code.background),
        Element::Barcode(barcode) => uses(&barcode.color) || uses(&barcode.background),
        Element::Group(group) => group
            .children
            .iter()
            .any(|child| element_uses_color_variable(child, variable)),
        Element::Stack(stack) => stack
            .children
            .iter()
            .any(|child| element_uses_color_variable(child, variable)),
        Element::Table(table) => {
            uses(&table.color)
                || table.border.as_ref().is_some_and(stroke_uses)
                || table.header_background.as_deref().is_some_and(uses)
                || table.row_background.as_deref().is_some_and(uses)
                || table.alternate_row_background.as_deref().is_some_and(uses)
        }
        Element::Repeater(repeater) => element_uses_color_variable(&repeater.template, variable),
        Element::Image(_) | Element::Svg(_) | Element::PageBreak => false,
    }
}

fn resolved_template_page(
    document: &ResolvedDocument,
    template_page: usize,
) -> Option<&ResolvedPage> {
    let prefix = format!("pages[{template_page}].");
    document
        .pages
        .iter()
        .find(|page| {
            page.commands
                .iter()
                .any(|command| command.source_path.starts_with(&prefix))
        })
        .or_else(|| document.pages.get(template_page))
}

struct ElementInteraction {
    clicked: bool,
    translate: Option<Vec2>,
    resize: Option<Vec2>,
    line_endpoint: Option<(LineEndpoint, Vec2)>,
    rotation: Option<f32>,
    drag_stopped: bool,
}

struct ElementPaintOptions {
    index: usize,
    selected: bool,
    primary: bool,
    locked: bool,
    paint_content: bool,
}

fn paint_resolved_page(
    ctx: &egui::Context,
    painter: &egui::Painter,
    page: Rect,
    scale: f32,
    resolved: &ResolvedPage,
    assets: &mut AssetCache,
) {
    for resolved_command in &resolved.commands {
        let transform = command_screen_transform(
            page,
            scale,
            &resolved_command.command,
            resolved_command.rotation,
        );
        match &resolved_command.command {
            DrawCommand::Text(text) => {
                let bounds = resolved_bounds(page, scale, text.bounds);
                let text_painter = if text.clip {
                    painter.with_clip_rect(
                        painter
                            .clip_rect()
                            .intersect(transformed_rect_bounds(bounds, transform)),
                    )
                } else {
                    painter.clone()
                };
                let font = FontId::proportional((text.font_size_pt * scale).max(1.0));
                let color = print_color(text.color);
                for line in &text.lines {
                    let baseline = page_point(page, scale, line.x, line.y);
                    if line.word_spacing_pt.abs() < f32::EPSILON {
                        let galley =
                            text_painter.layout_no_wrap(line.value.clone(), font.clone(), color);
                        let position = aligned_preview_text_origin(
                            baseline,
                            line.width_pt * scale,
                            galley.size().x,
                            galley_baseline_offset(&galley),
                            text.align,
                        );
                        paint_rotated_galley(&text_painter, position, galley, color, transform);
                        continue;
                    }
                    let words = line
                        .value
                        .split_inclusive(' ')
                        .map(|word| {
                            (
                                word.ends_with(' '),
                                text_painter.layout_no_wrap(word.to_owned(), font.clone(), color),
                            )
                        })
                        .collect::<Vec<_>>();
                    let spaces = words.iter().filter(|(space, _)| *space).count();
                    let actual_width = words.iter().map(|(_, galley)| galley.size().x).sum::<f32>();
                    let target_width =
                        (line.width_pt + line.word_spacing_pt * spaces as f32) * scale;
                    let preview_word_spacing = if spaces == 0 {
                        0.0
                    } else {
                        (target_width - actual_width) / spaces as f32
                    };
                    let mut position = baseline;
                    for (ends_with_space, galley) in words {
                        let width = galley.size().x;
                        let origin = position - Vec2::new(0.0, galley_baseline_offset(&galley));
                        paint_rotated_galley(&text_painter, origin, galley, color, transform);
                        position.x += width;
                        if ends_with_space {
                            position.x += preview_word_spacing;
                        }
                    }
                }
            }
            DrawCommand::Rectangle(rectangle) => {
                let rect = resolved_bounds(page, scale, rectangle.bounds);
                if let Some(fill) = rectangle.fill {
                    paint_transformed_rect(painter, rect, print_color(fill), transform);
                }
                if let Some(stroke) = &rectangle.stroke {
                    paint_transformed_rect_stroke(
                        painter,
                        rect,
                        Stroke::new(
                            (stroke.width_pt * scale).max(0.5),
                            print_color(stroke.color),
                        ),
                        stroke.dash,
                        transform,
                    );
                }
            }
            DrawCommand::Line(line) => paint_styled_line(
                painter,
                page_point(page, scale, line.start.x, line.start.y),
                page_point(page, scale, line.end.x, line.end.y),
                Stroke::new((line.width_pt * scale).max(0.5), print_color(line.color)),
                line.dash,
            ),
            DrawCommand::Image(image) => paint_asset(
                ctx,
                painter,
                resolved_bounds(page, scale, image.bounds),
                &image.source,
                assets,
                AssetPaint {
                    fit: image.fit,
                    kind: AssetKind::Raster,
                    transform,
                },
            ),
            DrawCommand::Svg(svg) => paint_asset(
                ctx,
                painter,
                resolved_bounds(page, scale, svg.bounds),
                &svg.source,
                assets,
                AssetPaint {
                    fit: svg.fit,
                    kind: AssetKind::Svg,
                    transform,
                },
            ),
            DrawCommand::QrCode(qr) => {
                let rect = resolved_bounds(page, scale, qr.bounds);
                paint_transformed_rect(painter, rect, print_color(qr.background), transform);
                let total = qr.size + usize::from(qr.quiet_zone) * 2;
                let module = rect.width() / total as f32;
                let quiet = usize::from(qr.quiet_zone);
                for y in 0..qr.size {
                    for x in 0..qr.size {
                        if qr.modules[y * qr.size + x] {
                            paint_transformed_rect(
                                painter,
                                Rect::from_min_size(
                                    Pos2::new(
                                        rect.left() + (quiet + x) as f32 * module,
                                        rect.top() + (quiet + y) as f32 * module,
                                    ),
                                    Vec2::splat(module + 0.25),
                                ),
                                print_color(qr.color),
                                transform,
                            );
                        }
                    }
                }
            }
            DrawCommand::Barcode(barcode) => {
                let rect = resolved_bounds(page, scale, barcode.bounds);
                paint_transformed_rect(painter, rect, print_color(barcode.background), transform);
                let total = barcode.modules.len() + usize::from(barcode.quiet_zone) * 2;
                let module = rect.width() / total as f32;
                let quiet = usize::from(barcode.quiet_zone);
                for (index, active) in barcode.modules.iter().enumerate() {
                    if *active {
                        paint_transformed_rect(
                            painter,
                            Rect::from_min_max(
                                Pos2::new(
                                    rect.left() + (quiet + index) as f32 * module,
                                    rect.top(),
                                ),
                                Pos2::new(
                                    rect.left() + (quiet + index + 1) as f32 * module + 0.25,
                                    rect.bottom(),
                                ),
                            ),
                            print_color(barcode.color),
                            transform,
                        );
                    }
                }
            }
        }
    }
}

fn resolved_bounds(page: Rect, scale: f32, bounds: print_forge_engine::Rect) -> Rect {
    page_bounds(
        page,
        scale,
        [bounds.x, bounds.y, bounds.width, bounds.height],
    )
}

#[derive(Clone, Copy)]
struct ScreenTransform {
    center: Pos2,
    angle: f32,
}

impl ScreenTransform {
    fn point(self, point: Pos2) -> Pos2 {
        if self.angle.abs() < f32::EPSILON {
            return point;
        }
        let offset = point - self.center;
        let sine = self.angle.sin();
        let cosine = self.angle.cos();
        self.center
            + Vec2::new(
                cosine * offset.x - sine * offset.y,
                sine * offset.x + cosine * offset.y,
            )
    }
}

fn command_screen_transform(
    page: Rect,
    scale: f32,
    command: &DrawCommand,
    clockwise_degrees: f32,
) -> ScreenTransform {
    let bounds = match command {
        DrawCommand::Text(command) => Some(command.bounds),
        DrawCommand::Image(command) => Some(command.bounds),
        DrawCommand::Rectangle(command) => Some(command.bounds),
        DrawCommand::Svg(command) => Some(command.bounds),
        DrawCommand::QrCode(command) => Some(command.bounds),
        DrawCommand::Barcode(command) => Some(command.bounds),
        DrawCommand::Line(_) => None,
    };
    let center = bounds.map_or(page.center(), |bounds| {
        resolved_bounds(page, scale, bounds).center()
    });
    ScreenTransform {
        center,
        angle: clockwise_degrees.to_radians(),
    }
}

fn aligned_preview_text_origin(
    baseline: Pos2,
    target_width: f32,
    preview_width: f32,
    baseline_offset: f32,
    align: TextAlign,
) -> Pos2 {
    let x_offset = match align {
        TextAlign::Center => (target_width - preview_width) / 2.0,
        TextAlign::Right => target_width - preview_width,
        TextAlign::Left | TextAlign::Justify => 0.0,
    };
    baseline + Vec2::new(x_offset, -baseline_offset)
}

fn galley_baseline_offset(galley: &egui::Galley) -> f32 {
    galley
        .rows
        .first()
        .and_then(|row| row.glyphs.first().map(|glyph| row.pos.y + glyph.pos.y))
        .unwrap_or_else(|| galley.size().y)
}

fn paint_rotated_galley(
    painter: &egui::Painter,
    top_left: Pos2,
    galley: Arc<egui::Galley>,
    color: Color32,
    transform: ScreenTransform,
) {
    painter.add(
        egui::epaint::TextShape::new(transform.point(top_left), galley, color)
            .with_angle(transform.angle),
    );
}

fn transformed_rect_points(rect: Rect, transform: ScreenTransform) -> [Pos2; 4] {
    [
        transform.point(rect.left_top()),
        transform.point(rect.right_top()),
        transform.point(rect.right_bottom()),
        transform.point(rect.left_bottom()),
    ]
}

fn transformed_rect_bounds(rect: Rect, transform: ScreenTransform) -> Rect {
    let points = transformed_rect_points(rect, transform);
    let mut bounds = Rect::NOTHING;
    for point in points {
        bounds.extend_with(point);
    }
    bounds
}

fn paint_transformed_rect(
    painter: &egui::Painter,
    rect: Rect,
    color: Color32,
    transform: ScreenTransform,
) {
    if transform.angle.abs() < f32::EPSILON {
        painter.rect_filled(rect, CornerRadius::ZERO, color);
    } else {
        painter.add(egui::Shape::convex_polygon(
            transformed_rect_points(rect, transform).to_vec(),
            color,
            Stroke::NONE,
        ));
    }
}

fn paint_transformed_rect_stroke(
    painter: &egui::Painter,
    rect: Rect,
    stroke: Stroke,
    dash: LineDash,
    transform: ScreenTransform,
) {
    let points = transformed_rect_points(rect, transform);
    for index in 0..points.len() {
        paint_styled_line(
            painter,
            points[index],
            points[(index + 1) % points.len()],
            stroke,
            dash,
        );
    }
}

fn print_color(color: PrintColor) -> Color32 {
    match color {
        PrintColor::Rgb { red, green, blue } => Color32::from_rgb(red, green, blue),
        PrintColor::Cmyk {
            cyan,
            magenta,
            yellow,
            black,
        } => {
            let convert = |ink: f32| {
                ((1.0 - ink / 100.0) * (1.0 - black / 100.0) * 255.0)
                    .round()
                    .clamp(0.0, 255.0) as u8
            };
            Color32::from_rgb(convert(cyan), convert(magenta), convert(yellow))
        }
    }
}

fn paint_rect_stroke(painter: &egui::Painter, rect: Rect, stroke: Stroke, dash: LineDash) {
    paint_styled_line(painter, rect.left_top(), rect.right_top(), stroke, dash);
    paint_styled_line(painter, rect.right_top(), rect.right_bottom(), stroke, dash);
    paint_styled_line(
        painter,
        rect.right_bottom(),
        rect.left_bottom(),
        stroke,
        dash,
    );
    paint_styled_line(painter, rect.left_bottom(), rect.left_top(), stroke, dash);
}

fn paint_styled_line(
    painter: &egui::Painter,
    start: Pos2,
    end: Pos2,
    stroke: Stroke,
    dash: LineDash,
) {
    if dash == LineDash::Solid {
        painter.line_segment([start, end], stroke);
        return;
    }
    let delta = end - start;
    let length = delta.length();
    if length <= f32::EPSILON {
        return;
    }
    let direction = delta / length;
    let (mark, gap) = match dash {
        LineDash::Dashed => ((stroke.width * 4.0).max(5.0), (stroke.width * 2.5).max(3.0)),
        LineDash::Dotted => (stroke.width.max(1.0), (stroke.width * 2.5).max(3.0)),
        LineDash::Solid => unreachable!(),
    };
    let mut offset = 0.0;
    while offset < length {
        let mark_end = (offset + mark).min(length);
        painter.line_segment(
            [start + direction * offset, start + direction * mark_end],
            stroke,
        );
        offset += mark + gap;
    }
}

fn paint_asset(
    ctx: &egui::Context,
    painter: &egui::Painter,
    bounds: Rect,
    path: &Path,
    assets: &mut AssetCache,
    paint: AssetPaint,
) {
    let path_text = path.to_string_lossy();
    if let Some(variable) = first_template_variable(&path_text) {
        let title = match paint.kind {
            AssetKind::Raster => "IMAGE PLACEHOLDER",
            AssetKind::Svg => "SVG PLACEHOLDER",
        };
        let detail = format!("{{{{{variable}}}}}");
        paint_placeholder(painter, bounds, title, &detail, Some(paint.transform));
        return;
    }
    match asset_texture(ctx, path, paint.kind, assets) {
        Ok(texture) => {
            paint_fitted_texture(painter, bounds, &texture, paint.fit, paint.transform);
        }
        Err(error) => paint_placeholder(
            painter,
            bounds,
            "ASSET UNAVAILABLE",
            &error,
            Some(paint.transform),
        ),
    }
}

fn first_template_variable(value: &str) -> Option<&str> {
    let start = value.find("{{")? + 2;
    let end = value[start..].find("}}")? + start;
    let variable = value[start..end].trim();
    (!variable.is_empty()).then_some(variable)
}

fn asset_texture(
    ctx: &egui::Context,
    path: &Path,
    kind: AssetKind,
    assets: &mut AssetCache,
) -> Result<TextureHandle, String> {
    let key = (path.to_owned(), kind);
    if let Some(cached) = assets.textures.get(&key) {
        return cached.clone();
    }
    let loaded = match kind {
        AssetKind::Raster => load_raster_texture(ctx, path),
        AssetKind::Svg => load_svg_texture(ctx, path),
    };
    assets.textures.insert(key, loaded.clone());
    loaded
}

fn load_raster_texture(ctx: &egui::Context, path: &Path) -> Result<TextureHandle, String> {
    let image = image::open(path)
        .map_err(|error| format!("{}: {error}", display_name(path)))?
        .to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let pixels = image.into_raw();
    Ok(ctx.load_texture(
        path.display().to_string(),
        egui::ColorImage::from_rgba_unmultiplied(size, &pixels),
        TextureOptions::LINEAR,
    ))
}

fn load_svg_texture(ctx: &egui::Context, path: &Path) -> Result<TextureHandle, String> {
    let bytes = fs::read(path).map_err(|error| format!("{}: {error}", display_name(path)))?;
    let tree = resvg::usvg::Tree::from_data(&bytes, &resvg::usvg::Options::default())
        .map_err(|error| format!("{}: {error}", display_name(path)))?;
    let intrinsic = tree.size();
    let largest = intrinsic.width().max(intrinsic.height());
    let raster_scale = (2048.0 / largest).min(4.0);
    let width = (intrinsic.width() * raster_scale).round().max(1.0) as u32;
    let height = (intrinsic.height() * raster_scale).round().max(1.0) as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| format!("{} is too large to preview", display_name(path)))?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(raster_scale, raster_scale),
        &mut pixmap.as_mut(),
    );
    Ok(ctx.load_texture(
        path.display().to_string(),
        egui::ColorImage::from_rgba_premultiplied([width as usize, height as usize], pixmap.data()),
        TextureOptions::LINEAR,
    ))
}

fn paint_fitted_texture(
    painter: &egui::Painter,
    bounds: Rect,
    texture: &TextureHandle,
    fit: ImageFit,
    transform: ScreenTransform,
) {
    let source = texture.size_vec2();
    if source.x <= 0.0 || source.y <= 0.0 {
        return;
    }
    let full_uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
    match fit {
        ImageFit::Stretch => {
            paint_transformed_texture(painter, bounds, full_uv, texture, transform)
        }
        ImageFit::Contain => {
            let factor = (bounds.width() / source.x).min(bounds.height() / source.y);
            let destination = Rect::from_center_size(bounds.center(), source * factor);
            paint_transformed_texture(painter, destination, full_uv, texture, transform)
        }
        ImageFit::Cover => {
            let source_aspect = source.x / source.y;
            let target_aspect = bounds.width() / bounds.height();
            let uv = if source_aspect > target_aspect {
                let visible = target_aspect / source_aspect;
                let inset = (1.0 - visible) / 2.0;
                Rect::from_min_max(Pos2::new(inset, 0.0), Pos2::new(1.0 - inset, 1.0))
            } else {
                let visible = source_aspect / target_aspect;
                let inset = (1.0 - visible) / 2.0;
                Rect::from_min_max(Pos2::new(0.0, inset), Pos2::new(1.0, 1.0 - inset))
            };
            paint_transformed_texture(painter, bounds, uv, texture, transform)
        }
    };
}

fn paint_transformed_texture(
    painter: &egui::Painter,
    bounds: Rect,
    uv: Rect,
    texture: &TextureHandle,
    transform: ScreenTransform,
) {
    if transform.angle.abs() < f32::EPSILON {
        painter.image(texture.id(), bounds, uv, Color32::WHITE);
        return;
    }
    let mut mesh = egui::Mesh::with_texture(texture.id());
    mesh.add_rect_with_uv(bounds, uv, Color32::WHITE);
    for vertex in &mut mesh.vertices {
        vertex.pos = transform.point(vertex.pos);
    }
    painter.add(mesh);
}

#[derive(Debug, PartialEq, Eq)]
struct PreviewErrorCopy {
    location: Option<String>,
    message: String,
    suggestion: Option<String>,
}

fn preview_error_copy(error: &str) -> PreviewErrorCopy {
    let error = error
        .strip_prefix("Preview layout failed: ")
        .unwrap_or(error);
    let (location, detail) = error
        .split_once(": layout failed: ")
        .map_or((None, error), |(location, detail)| {
            (friendly_error_location(location), detail)
        });
    let (message, suggestion) = detail
        .split_once("; ")
        .map_or((detail, None), |(message, suggestion)| {
            (message, Some(sentence_case(suggestion)))
        });
    PreviewErrorCopy {
        location,
        message: sentence_case(message),
        suggestion,
    }
}

fn friendly_error_location(location: &str) -> Option<String> {
    let page = number_after(location, "page ").map(|index| index + 1);
    let element = number_after(location, ".elements[").map(|index| index + 1);
    match (page, element) {
        (Some(page), Some(element)) => Some(format!("Page {page} · Element {element}")),
        (Some(page), None) => Some(format!("Page {page}")),
        (None, _) if location.trim().is_empty() => None,
        (None, _) => Some(location.trim().to_owned()),
    }
}

fn number_after(value: &str, marker: &str) -> Option<usize> {
    let start = value.find(marker)? + marker.len();
    let digits = value[start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>();
    (!digits.is_empty()).then(|| digits.parse().ok()).flatten()
}

fn sentence_case(value: &str) -> String {
    let value = value.trim().trim_end_matches('.');
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return String::new();
    };
    format!("{}{}.", first.to_uppercase(), characters.as_str())
}

fn paint_preview_error(painter: &egui::Painter, page: Rect, error: &str) {
    let copy = preview_error_copy(error);
    let width = (page.width() - 40.0).clamp(80.0, 620.0);
    let text_width = (width - 48.0).max(48.0);
    let title = painter.layout(
        "Preview unavailable".to_owned(),
        FontId::proportional(18.0),
        Color32::from_rgb(115, 32, 24),
        text_width,
    );
    let location = copy.location.as_ref().map(|location| {
        painter.layout(
            location.to_uppercase(),
            FontId::proportional(10.0),
            Color32::from_rgb(151, 60, 45),
            text_width,
        )
    });
    let message = painter.layout(
        copy.message,
        FontId::proportional(13.0),
        Color32::from_rgb(73, 46, 42),
        text_width,
    );
    let suggestion = copy.suggestion.map(|suggestion| {
        painter.layout(
            format!("Fix: {suggestion}"),
            FontId::proportional(12.0),
            Color32::from_rgb(96, 56, 49),
            text_width,
        )
    });
    let location_height = location
        .as_ref()
        .map_or(0.0, |galley| galley.size().y + 10.0);
    let suggestion_height = suggestion
        .as_ref()
        .map_or(0.0, |galley| galley.size().y + 12.0);
    let content_height =
        48.0 + title.size().y + location_height + message.size().y + suggestion_height;
    let height = content_height.min((page.height() - 40.0).max(96.0));
    let card = Rect::from_center_size(page.center(), Vec2::new(width, height));
    painter.rect_filled(
        card,
        CornerRadius::same(8),
        Color32::from_rgb(255, 246, 243),
    );
    painter.rect_stroke(
        card,
        CornerRadius::same(8),
        Stroke::new(1.0_f32, Color32::from_rgb(220, 106, 81)),
        StrokeKind::Inside,
    );
    painter.rect_filled(
        Rect::from_min_max(card.left_top(), Pos2::new(card.left() + 5.0, card.bottom())),
        CornerRadius::same(8),
        Color32::from_rgb(224, 80, 54),
    );
    let clipped = painter.with_clip_rect(card.shrink(16.0));
    let mut position = card.left_top() + Vec2::new(24.0, 20.0);
    clipped.galley(position, title.clone(), Color32::PLACEHOLDER);
    position.y += title.size().y + 10.0;
    if let Some(location) = location {
        clipped.galley(position, location.clone(), Color32::PLACEHOLDER);
        position.y += location.size().y + 10.0;
    }
    clipped.galley(position, message.clone(), Color32::PLACEHOLDER);
    position.y += message.size().y + 12.0;
    if let Some(suggestion) = suggestion {
        clipped.galley(position, suggestion, Color32::PLACEHOLDER);
    }
}

fn paint_element(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    page: Rect,
    scale: f32,
    element: &Element,
    options: ElementPaintOptions,
) -> ElementInteraction {
    let mut interaction = ElementInteraction {
        clicked: false,
        translate: None,
        resize: None,
        line_endpoint: None,
        rotation: None,
        drag_stopped: false,
    };
    if let Element::Line(line) = element {
        let start = page_point(page, scale, line.x1.to_points(), line.y1.to_points());
        let end = page_point(page, scale, line.x2.to_points(), line.y2.to_points());
        if options.paint_content {
            painter.line_segment(
                [start, end],
                Stroke::new(line.width.to_points().max(1.0), parse_color(&line.color)),
            );
        }
        let rect = Rect::from_two_pos(start, end).expand(7.0);
        let response = ui.interact(
            rect,
            Id::new(("canvas-line", options.index)),
            if options.locked {
                Sense::click()
            } else {
                Sense::click_and_drag()
            },
        );
        interaction.clicked = response.clicked();
        interaction.drag_stopped |= response.drag_stopped();
        if response.hovered() && !options.locked {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        if options.selected {
            painter.rect_stroke(
                rect,
                CornerRadius::same(1),
                Stroke::new(1.5_f32, ORANGE),
                StrokeKind::Outside,
            );
        }
        if options.primary && !options.locked {
            for (endpoint, center) in [(LineEndpoint::Start, start), (LineEndpoint::End, end)] {
                let handle = Rect::from_center_size(center, Vec2::splat(12.0));
                painter.rect_filled(handle, CornerRadius::same(2), Color32::WHITE);
                painter.rect_stroke(
                    handle,
                    CornerRadius::same(2),
                    Stroke::new(2.0_f32, ORANGE),
                    StrokeKind::Inside,
                );
                let endpoint_response = ui.interact(
                    handle,
                    Id::new(("canvas-line-endpoint", options.index, endpoint)),
                    Sense::click_and_drag(),
                );
                interaction.clicked |= endpoint_response.clicked();
                interaction.drag_stopped |= endpoint_response.drag_stopped();
                if endpoint_response.dragged() {
                    interaction.line_endpoint = Some((endpoint, endpoint_response.drag_delta()));
                    interaction.translate = None;
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
                } else if endpoint_response.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
                }
            }
        }
        if !options.locked && interaction.line_endpoint.is_none() && response.dragged() {
            interaction.translate = Some(response.drag_delta());
        }
        return interaction;
    }

    let Some(bounds) = element_bounds(element) else {
        return interaction;
    };
    let points = bounds_points(bounds);
    let rect = page_bounds(page, scale, points);
    let rotation = element_rotation(element).unwrap_or(0.0);
    let transform = ScreenTransform {
        center: rect.center(),
        angle: rotation.to_radians(),
    };
    if options.paint_content {
        paint_element_content(painter, rect, scale, element, transform);
    }
    let interaction_bounds = transformed_rect_bounds(rect, transform);
    let response = ui.interact(
        interaction_bounds,
        Id::new(("canvas-element", options.index)),
        if options.locked {
            Sense::click()
        } else {
            Sense::click_and_drag()
        },
    );
    interaction.clicked = response.clicked();
    interaction.drag_stopped |= response.drag_stopped();
    if !options.locked && response.dragged() {
        interaction.translate = Some(response.drag_delta());
    }
    if response.hovered() && !options.locked {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    if options.selected {
        paint_transformed_rect_stroke(
            painter,
            rect,
            Stroke::new(2.0_f32, ORANGE),
            LineDash::Solid,
            transform,
        );
    }
    if options.primary && !options.locked {
        let resize_center = transform.point(rect.right_top());
        let handle = Rect::from_center_size(resize_center, Vec2::splat(12.0));
        painter.rect_filled(handle, CornerRadius::same(2), ORANGE);
        let resize = ui.interact(
            handle,
            Id::new(("canvas-resize", options.index)),
            Sense::drag(),
        );
        interaction.drag_stopped |= resize.drag_stopped();
        if resize.dragged() {
            let delta = inverse_rotate_vector(resize.drag_delta(), transform.angle);
            interaction.translate = None;
            interaction.resize = Some(Vec2::new(delta.x, -delta.y));
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNeSw);
        }
        if element_rotation(element).is_some() {
            let stem_start = transform.point(rect.center_top());
            let rotation_center = transform.point(rect.center_top() - Vec2::new(0.0, 26.0));
            painter.line_segment([stem_start, rotation_center], Stroke::new(1.5_f32, ORANGE));
            painter.circle_filled(rotation_center, 7.0, Color32::WHITE);
            painter.circle_stroke(rotation_center, 7.0, Stroke::new(2.0_f32, ORANGE));
            let rotation_handle = Rect::from_center_size(rotation_center, Vec2::splat(18.0));
            let rotate = ui.interact(
                rotation_handle,
                Id::new(("canvas-rotation", options.index)),
                Sense::click_and_drag(),
            );
            interaction.drag_stopped |= rotate.drag_stopped();
            if rotate.dragged()
                && let Some(pointer) = ui.input(|input| input.pointer.interact_pos())
            {
                let vector = pointer - rect.center();
                interaction.rotation = Some(vector.y.atan2(vector.x).to_degrees() + 90.0);
                interaction.translate = None;
                interaction.resize = None;
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            } else if rotate.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            }
        }
    }
    interaction
}

fn inverse_rotate_vector(vector: Vec2, angle: f32) -> Vec2 {
    let sine = angle.sin();
    let cosine = angle.cos();
    Vec2::new(
        cosine * vector.x + sine * vector.y,
        -sine * vector.x + cosine * vector.y,
    )
}

const fn line_dash(dash: DashStyle) -> LineDash {
    match dash {
        DashStyle::Solid => LineDash::Solid,
        DashStyle::Dashed => LineDash::Dashed,
        DashStyle::Dotted => LineDash::Dotted,
    }
}

fn paint_element_content(
    painter: &egui::Painter,
    rect: Rect,
    scale: f32,
    element: &Element,
    transform: ScreenTransform,
) {
    match element {
        Element::Text(text) => {
            let font_size = (text.font_size.to_points() * scale).clamp(8.0, 34.0);
            let color = parse_color(&text.color);
            let galley =
                painter.layout_no_wrap(text.value.clone(), FontId::proportional(font_size), color);
            painter.add(
                egui::epaint::TextShape::new(
                    transform.point(rect.left_top() + Vec2::new(3.0, 2.0)),
                    galley,
                    color,
                )
                .with_angle(transform.angle),
            );
        }
        Element::Rectangle(rectangle) => {
            paint_transformed_rect(
                painter,
                rect,
                rectangle
                    .fill
                    .as_deref()
                    .map(parse_color)
                    .unwrap_or(Color32::TRANSPARENT),
                transform,
            );
            if let Some(stroke) = &rectangle.stroke {
                paint_transformed_rect_stroke(
                    painter,
                    rect,
                    Stroke::new(
                        stroke.width.to_points().max(1.0),
                        parse_color(&stroke.color),
                    ),
                    line_dash(stroke.dash),
                    transform,
                );
            }
        }
        Element::Image(image) => {
            paint_placeholder(painter, rect, "IMAGE", &image.source, Some(transform));
        }
        Element::Svg(svg) => {
            paint_placeholder(painter, rect, "VECTOR SVG", &svg.source, Some(transform));
        }
        Element::QrCode(_) => paint_qr_placeholder(painter, rect, transform),
        Element::Barcode(barcode) => {
            paint_barcode_placeholder(painter, rect, &barcode.value, transform);
        }
        Element::Group(_) => paint_placeholder(painter, rect, "GROUP", "composed elements", None),
        Element::Stack(_) => {
            paint_placeholder(painter, rect, "FLOW STACK", "paginating region", None)
        }
        Element::Table(table) => paint_placeholder(painter, rect, "TABLE", &table.source, None),
        Element::Line(_) | Element::Repeater(_) | Element::PageBreak => {}
    }
}

fn paint_placeholder(
    painter: &egui::Painter,
    rect: Rect,
    title: &str,
    detail: &str,
    transform: Option<ScreenTransform>,
) {
    let transform = transform.unwrap_or(ScreenTransform {
        center: rect.center(),
        angle: 0.0,
    });
    paint_transformed_rect(painter, rect, Color32::from_rgb(226, 228, 225), transform);
    paint_transformed_rect_stroke(
        painter,
        rect,
        Stroke::new(1.0_f32, Color32::from_rgb(165, 170, 168)),
        LineDash::Solid,
        transform,
    );
    painter.line_segment(
        [
            transform.point(rect.left_top()),
            transform.point(rect.right_bottom()),
        ],
        Stroke::new(1.0_f32, Color32::from_rgb(190, 194, 191)),
    );
    painter.line_segment(
        [
            transform.point(rect.right_top()),
            transform.point(rect.left_bottom()),
        ],
        Stroke::new(1.0_f32, Color32::from_rgb(190, 194, 191)),
    );
    paint_rotated_centered_text(
        painter,
        rect.center() - Vec2::new(0.0, 8.0),
        title,
        FontId::proportional(12.0),
        CHARCOAL,
        transform,
    );
    paint_rotated_centered_text(
        painter,
        rect.center() + Vec2::new(0.0, 10.0),
        detail,
        FontId::proportional(9.0),
        Color32::from_gray(95),
        transform,
    );
}

fn paint_rotated_centered_text(
    painter: &egui::Painter,
    center: Pos2,
    value: &str,
    font: FontId,
    color: Color32,
    transform: ScreenTransform,
) {
    let galley = painter.layout_no_wrap(value.to_owned(), font, color);
    let top_left = center - galley.size() / 2.0;
    painter.add(
        egui::epaint::TextShape::new(transform.point(top_left), galley, color)
            .with_angle(transform.angle),
    );
}

fn paint_qr_placeholder(painter: &egui::Painter, rect: Rect, transform: ScreenTransform) {
    paint_transformed_rect(painter, rect, Color32::WHITE, transform);
    let cells = 11;
    let size = rect.width().min(rect.height()) / cells as f32;
    let origin = rect.center() - Vec2::splat(size * cells as f32 / 2.0);
    for y in 0..cells {
        for x in 0..cells {
            let finder = ((x < 3 || x >= cells - 3) && y < 3) || (x < 3 && y >= cells - 3);
            if finder || (x * 3 + y * 5 + x * y) % 4 == 0 {
                paint_transformed_rect(
                    painter,
                    Rect::from_min_size(
                        origin + Vec2::new(x as f32 * size, y as f32 * size),
                        Vec2::splat(size + 0.2),
                    ),
                    CHARCOAL,
                    transform,
                );
            }
        }
    }
}

fn paint_barcode_placeholder(
    painter: &egui::Painter,
    rect: Rect,
    value: &str,
    transform: ScreenTransform,
) {
    paint_transformed_rect(painter, rect, Color32::WHITE, transform);
    let bars = 43;
    let bar_width = rect.width() / bars as f32;
    for index in 0..bars {
        if (index * 7 + 3) % 5 < 2 {
            paint_transformed_rect(
                painter,
                Rect::from_min_max(
                    Pos2::new(rect.left() + index as f32 * bar_width, rect.top() + 5.0),
                    Pos2::new(
                        rect.left() + (index + 1) as f32 * bar_width,
                        rect.bottom() - 16.0,
                    ),
                ),
                CHARCOAL,
                transform,
            );
        }
    }
    paint_rotated_centered_text(
        painter,
        Pos2::new(rect.center().x, rect.bottom() - 8.0),
        value,
        FontId::monospace(9.0),
        CHARCOAL,
        transform,
    );
}

fn alignment_targets(
    elements: &[Element],
    excluded_indices: &[usize],
    page_width: f32,
    page_height: f32,
    guides: &[EditorGuide],
) -> (Vec<f32>, Vec<f32>) {
    let mut x_targets = vec![0.0, page_width / 2.0, page_width];
    let mut y_targets = vec![0.0, page_height / 2.0, page_height];
    for guide in guides {
        match guide.axis {
            GuideAxis::Vertical => x_targets.push(guide.position_pt),
            GuideAxis::Horizontal => y_targets.push(guide.position_pt),
        }
    }
    for (index, element) in elements.iter().enumerate() {
        if excluded_indices.contains(&index) || !element.is_visible() {
            continue;
        }
        if let Some(bounds) = element_alignment_bounds(element) {
            x_targets.extend([
                bounds[0],
                bounds[0] + bounds[2] / 2.0,
                bounds[0] + bounds[2],
            ]);
            y_targets.extend([
                bounds[1],
                bounds[1] + bounds[3] / 2.0,
                bounds[1] + bounds[3],
            ]);
        }
    }
    x_targets.sort_by(f32::total_cmp);
    x_targets.dedup_by(|left, right| (*left - *right).abs() < 0.001);
    y_targets.sort_by(f32::total_cmp);
    y_targets.dedup_by(|left, right| (*left - *right).abs() < 0.001);
    (x_targets, y_targets)
}

fn push_editor_guide(guides: &mut Vec<EditorGuide>, guide: EditorGuide) {
    if !guides.iter().any(|existing| {
        existing.axis == guide.axis && (existing.position_pt - guide.position_pt).abs() < 0.001
    }) {
        guides.push(guide);
    }
}

fn line_endpoint_position(element: &Element, endpoint: LineEndpoint) -> Option<[f32; 2]> {
    let Element::Line(line) = element else {
        return None;
    };
    Some(match endpoint {
        LineEndpoint::Start => [line.x1.to_points(), line.y1.to_points()],
        LineEndpoint::End => [line.x2.to_points(), line.y2.to_points()],
    })
}

fn paint_and_interact_rulers(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    page: Rect,
    scale: f32,
    guides: &mut Vec<EditorGuide>,
    draft: &mut Option<GuideDraft>,
) {
    const RULER_SIZE: f32 = 22.0;
    let page_width = page.width() / scale;
    let page_height = page.height() / scale;
    let background = Color32::from_rgb(42, 48, 53);
    let tick_color = Color32::from_gray(145);
    let top = Rect::from_min_max(
        Pos2::new(page.left(), page.top() - RULER_SIZE),
        page.right_top(),
    );
    let left = Rect::from_min_max(
        Pos2::new(page.left() - RULER_SIZE, page.top()),
        page.left_bottom(),
    );
    painter.rect_filled(top, CornerRadius::ZERO, background);
    painter.rect_filled(left, CornerRadius::ZERO, background);
    let corner = Rect::from_min_max(
        Pos2::new(page.left() - RULER_SIZE, page.top() - RULER_SIZE),
        page.left_top(),
    );
    painter.rect_filled(corner, CornerRadius::ZERO, Color32::from_rgb(34, 40, 45));
    painter.text(
        corner.center(),
        Align2::CENTER_CENTER,
        "pt",
        FontId::monospace(7.0),
        Color32::from_gray(165),
    );

    let minor_step = if 18.0 * scale >= 6.0 { 18.0 } else { 36.0 };
    let major_step = minor_step * 4.0;
    let mut x = 0.0;
    while x <= page_width + 0.001 {
        let screen_x = page.left() + x * scale;
        let major = (x / major_step).fract().abs() < 0.001;
        let length = if major { 10.0 } else { 5.0 };
        painter.line_segment(
            [
                Pos2::new(screen_x, top.bottom()),
                Pos2::new(screen_x, top.bottom() - length),
            ],
            Stroke::new(1.0_f32, tick_color),
        );
        if major && x > 0.0 {
            painter.text(
                Pos2::new(screen_x + 3.0, top.top() + 3.0),
                Align2::LEFT_TOP,
                format!("{x:.0}"),
                FontId::monospace(8.0),
                Color32::from_gray(170),
            );
        }
        x += minor_step;
    }

    let mut y = 0.0;
    while y <= page_height + 0.001 {
        let screen_y = page.bottom() - y * scale;
        let major = (y / major_step).fract().abs() < 0.001;
        let length = if major { 10.0 } else { 5.0 };
        painter.line_segment(
            [
                Pos2::new(left.right(), screen_y),
                Pos2::new(left.right() - length, screen_y),
            ],
            Stroke::new(1.0_f32, tick_color),
        );
        if major && y > 0.0 {
            painter.text(
                Pos2::new(left.left() + 2.0, screen_y - 3.0),
                Align2::LEFT_BOTTOM,
                format!("{y:.0}"),
                FontId::monospace(7.0),
                Color32::from_gray(170),
            );
        }
        y += minor_step;
    }

    let top_response = ui
        .interact(top, Id::new("horizontal-guide-ruler"), Sense::drag())
        .on_hover_text("Drag down to create a horizontal guide");
    let left_response = ui
        .interact(left, Id::new("vertical-guide-ruler"), Sense::drag())
        .on_hover_text("Drag right to create a vertical guide");
    if top_response.hovered() || top_response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }
    if left_response.hovered() || left_response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    if let Some(guide) =
        update_ruler_guide_draft(ui, &top_response, GuideAxis::Horizontal, page, scale, draft)
    {
        push_editor_guide(guides, guide);
    }
    if let Some(guide) =
        update_ruler_guide_draft(ui, &left_response, GuideAxis::Vertical, page, scale, draft)
    {
        push_editor_guide(guides, guide);
    }
    if let Some(draft) = draft.as_ref().filter(|draft| draft.valid) {
        paint_guide_line(
            painter,
            page,
            scale,
            EditorGuide {
                axis: draft.axis,
                position_pt: draft.position_pt,
            },
            Color32::from_rgb(85, 215, 246),
            2.0,
        );
    }
}

fn update_ruler_guide_draft(
    ui: &egui::Ui,
    response: &egui::Response,
    axis: GuideAxis,
    page: Rect,
    scale: f32,
    draft: &mut Option<GuideDraft>,
) -> Option<EditorGuide> {
    if response.dragged()
        && let Some(pointer) = ui.input(|input| input.pointer.latest_pos())
    {
        let (position_pt, valid) = guide_position_from_pointer(axis, pointer, page, scale);
        *draft = Some(GuideDraft {
            axis,
            position_pt,
            valid,
        });
    }
    if response.drag_stopped() && draft.is_some_and(|draft| draft.axis == axis) {
        let completed = draft.take().filter(|draft| draft.valid);
        return completed.map(|draft| EditorGuide {
            axis: draft.axis,
            position_pt: draft.position_pt,
        });
    }
    None
}

fn guide_position_from_pointer(
    axis: GuideAxis,
    pointer: Pos2,
    page: Rect,
    scale: f32,
) -> (f32, bool) {
    match axis {
        GuideAxis::Vertical => (
            ((pointer.x - page.left()) / scale).clamp(0.0, page.width() / scale),
            (page.left()..=page.right()).contains(&pointer.x),
        ),
        GuideAxis::Horizontal => (
            ((page.bottom() - pointer.y) / scale).clamp(0.0, page.height() / scale),
            (page.top()..=page.bottom()).contains(&pointer.y),
        ),
    }
}

fn paint_guide_line(
    painter: &egui::Painter,
    page: Rect,
    scale: f32,
    guide: EditorGuide,
    color: Color32,
    width: f32,
) {
    match guide.axis {
        GuideAxis::Vertical => {
            let x = page.left() + guide.position_pt * scale;
            painter.line_segment(
                [Pos2::new(x, page.top()), Pos2::new(x, page.bottom())],
                Stroke::new(width, color),
            );
        }
        GuideAxis::Horizontal => {
            let y = page.bottom() - guide.position_pt * scale;
            painter.line_segment(
                [Pos2::new(page.left(), y), Pos2::new(page.right(), y)],
                Stroke::new(width, color),
            );
        }
    }
}

fn paint_and_interact_guides(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    page: Rect,
    scale: f32,
    page_width: f32,
    page_height: f32,
    guides: &mut [EditorGuide],
) {
    let color = Color32::from_rgb(55, 190, 232);
    for (index, guide) in guides.iter_mut().enumerate() {
        match guide.axis {
            GuideAxis::Vertical => {
                let x = page.left() + guide.position_pt * scale;
                painter.line_segment(
                    [Pos2::new(x, page.top()), Pos2::new(x, page.bottom())],
                    Stroke::new(1.0_f32, color),
                );
                let response = ui.interact(
                    Rect::from_min_max(
                        Pos2::new(x - 4.0, page.top()),
                        Pos2::new(x + 4.0, page.bottom()),
                    ),
                    Id::new(("vertical-guide", index)),
                    Sense::drag(),
                );
                if response.dragged()
                    && let Some(pointer) = ui.input(|input| input.pointer.interact_pos())
                {
                    guide.position_pt = ((pointer.x - page.left()) / scale).clamp(0.0, page_width);
                }
                if response.hovered() || response.dragged() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }
            }
            GuideAxis::Horizontal => {
                let y = page.bottom() - guide.position_pt * scale;
                painter.line_segment(
                    [Pos2::new(page.left(), y), Pos2::new(page.right(), y)],
                    Stroke::new(1.0_f32, color),
                );
                let response = ui.interact(
                    Rect::from_min_max(
                        Pos2::new(page.left(), y - 4.0),
                        Pos2::new(page.right(), y + 4.0),
                    ),
                    Id::new(("horizontal-guide", index)),
                    Sense::drag(),
                );
                if response.dragged()
                    && let Some(pointer) = ui.input(|input| input.pointer.interact_pos())
                {
                    guide.position_pt =
                        ((page.bottom() - pointer.y) / scale).clamp(0.0, page_height);
                }
                if response.hovered() || response.dragged() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                }
            }
        }
    }
}

fn paint_snap_feedback(painter: &egui::Painter, page: Rect, scale: f32, feedback: SnapFeedback) {
    let color = Color32::from_rgb(232, 66, 190);
    if let Some(x) = feedback.x {
        let screen_x = page.left() + x * scale;
        painter.line_segment(
            [
                Pos2::new(screen_x, page.top()),
                Pos2::new(screen_x, page.bottom()),
            ],
            Stroke::new(1.25_f32, color),
        );
        painter.text(
            Pos2::new(screen_x + 4.0, page.top() + 4.0),
            Align2::LEFT_TOP,
            format!("x {x:.1} pt"),
            FontId::monospace(9.0),
            color,
        );
    }
    if let Some(y) = feedback.y {
        let screen_y = page.bottom() - y * scale;
        painter.line_segment(
            [
                Pos2::new(page.left(), screen_y),
                Pos2::new(page.right(), screen_y),
            ],
            Stroke::new(1.25_f32, color),
        );
        painter.text(
            Pos2::new(page.left() + 4.0, screen_y - 4.0),
            Align2::LEFT_BOTTOM,
            format!("y {y:.1} pt"),
            FontId::monospace(9.0),
            color,
        );
    }
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
    if let Some(mut rotation) = element_rotation(element) {
        section_label(ui, "ROTATION");
        let mut rotation_changed = false;
        ui.horizontal(|ui| {
            rotation_changed |= ui
                .add(egui::DragValue::new(&mut rotation).speed(1.0).suffix("°"))
                .changed();
            for (label, value) in [("−90°", -90.0), ("0°", 0.0), ("+90°", 90.0)] {
                if ui.small_button(label).clicked() {
                    rotation = value;
                    rotation_changed = true;
                }
            }
        });
        if rotation_changed {
            set_element_rotation(element, rotation);
            changed = true;
        }
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
            changed |= color_editor(ui, "Color", &mut text.color);
        }
        Element::Rectangle(rectangle) => {
            section_label(ui, "APPEARANCE");
            changed |= optional_color_editor(ui, "Fill", &mut rectangle.fill);
            if let Some(stroke) = &mut rectangle.stroke {
                changed |= color_editor(ui, "Stroke color", &mut stroke.color);
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
            changed |= color_editor(ui, "Color", &mut qr.color);
            changed |= color_editor(ui, "Background", &mut qr.background);
        }
        Element::Barcode(barcode) => {
            section_label(ui, "CODE 128");
            changed |= string_editor(ui, "Value", &mut barcode.value);
            ui.label("Quiet zone (modules)");
            changed |= ui
                .add(egui::DragValue::new(&mut barcode.quiet_zone).range(10..=64))
                .changed();
            changed |= color_editor(ui, "Color", &mut barcode.color);
            changed |= color_editor(ui, "Background", &mut barcode.background);
        }
        Element::Line(line) => {
            section_label(ui, "LINE");
            changed |= length_editor(ui, "X1", &mut line.x1, "line-x1");
            changed |= length_editor(ui, "Y1", &mut line.y1, "line-y1");
            changed |= length_editor(ui, "X2", &mut line.x2, "line-x2");
            changed |= length_editor(ui, "Y2", &mut line.y2, "line-y2");
            changed |= length_editor(ui, "Width", &mut line.width, "line-width");
            changed |= color_editor(ui, "Color", &mut line.color);
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

fn layer_properties(ui: &mut egui::Ui, element: &mut Element) -> bool {
    section_label(ui, "LAYER");
    ui.label("Name");
    let mut name = element.layer_name().unwrap_or_default().to_owned();
    let mut changed = false;
    if ui
        .add(
            egui::TextEdit::singleline(&mut name)
                .desired_width(f32::INFINITY)
                .hint_text("Use the generated layer name"),
        )
        .changed()
    {
        element.set_layer_name((!name.trim().is_empty()).then_some(name));
        changed = true;
    }
    let mut visible = element.is_visible();
    if ui
        .checkbox(&mut visible, "Visible in canvas and PDF")
        .changed()
    {
        element.set_visible(visible);
        changed = true;
    }
    let mut locked = element.is_locked();
    if ui.checkbox(&mut locked, "Lock editing").changed() {
        element.set_locked(locked);
        changed = true;
    }
    ui.add_space(12.0);
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

fn color_editor(ui: &mut egui::Ui, label: &str, value: &mut String) -> bool {
    ui.label(label);
    color_value_editor(ui, value)
}

fn optional_color_editor(ui: &mut egui::Ui, label: &str, value: &mut Option<String>) -> bool {
    let mut enabled = value.is_some();
    let mut changed = ui.checkbox(&mut enabled, label).changed();
    if enabled && value.is_none() {
        *value = Some("#FFFFFF".to_owned());
        changed = true;
    } else if !enabled && value.is_some() {
        *value = None;
        changed = true;
    }
    if let Some(value) = value {
        changed |= color_value_editor(ui, value);
    }
    changed
}

fn color_value_editor(ui: &mut egui::Ui, value: &mut String) -> bool {
    let parsed = value.parse::<PrintColor>();
    let screen_color = parsed
        .as_ref()
        .map_or(CHARCOAL, |color| print_color(*color));
    let mut rgb = [screen_color.r(), screen_color.g(), screen_color.b()];
    let changed = ui
        .horizontal(|ui| {
            let mut changed = false;
            if ui
                .color_edit_button_srgb(&mut rgb)
                .on_hover_text("Choose an RGB color")
                .changed()
            {
                *value = rgb_hex(rgb);
                changed = true;
            }
            changed |= ui
                .add_sized(
                    [ui.available_width(), 24.0],
                    egui::TextEdit::singleline(value).hint_text("#RRGGBB"),
                )
                .changed();
            changed
        })
        .inner;
    if value.parse::<PrintColor>().is_err() {
        ui.label(
            RichText::new("Use #RRGGBB, rgb(R, G, B), or cmyk(C%, M%, Y%, K%).")
                .size(10.0)
                .color(Color32::from_rgb(242, 111, 111)),
        );
    }
    changed
}

fn rgb_hex(rgb: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
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
    value
        .trim()
        .parse::<PrintColor>()
        .map_or(CHARCOAL, print_color)
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
    use std::path::PathBuf;

    use eframe::egui::{Color32, Pos2, Vec2};
    use print_forge_engine::DrawCommand;
    use print_forge_template::{Color as PrintColor, Element, TextAlign};

    use super::{
        EditHistory, EditSnapshot, EditorGuide, ElementDragKind, ElementDragState, GuideAxis,
        PersistedState, PreviewErrorCopy, ScreenTransform, Selection, aligned_preview_text_origin,
        alignment_targets, first_template_variable, guide_position_from_pointer,
        inverse_rotate_vector, parse_color, preview_error_copy, print_color, push_editor_guide,
        remap_selection_after_layer_move, resolve_preview, rgb_hex, safe_stem, serialize_template,
    };
    use crate::model::{ElementKind, new_element, starter_template};

    #[test]
    fn serialized_studio_templates_round_trip() {
        let template = starter_template();
        let json = serialize_template(&template).unwrap();
        assert!(json.ends_with('\n'));
        assert_eq!(template, serde_json::from_str(&json).unwrap());
    }

    #[test]
    fn dragged_layers_preserve_multi_selection_membership_and_primary() {
        let selection = Selection::Elements {
            indices: [0, 2].into_iter().collect(),
            primary: 0,
        };

        assert_eq!(
            remap_selection_after_layer_move(&selection, 0, 2),
            Selection::Elements {
                indices: [1, 2].into_iter().collect(),
                primary: 2,
            }
        );
    }

    #[test]
    fn edit_history_coalesces_interactions_and_supports_redo() {
        let snapshot = |name: &str| {
            let mut template = starter_template();
            template.name = name.to_owned();
            EditSnapshot {
                template,
                guides: vec![Vec::new()],
                current_page: 0,
                selection: Selection::Document,
            }
        };
        let mut history = EditHistory::default();
        history.reset(snapshot("Original"));
        history.observe(snapshot("Dragging 1"), true);
        history.observe(snapshot("Dragging 2"), true);
        history.observe(snapshot("Dragging 2"), false);

        let undone = history.undo(snapshot("Dragging 2")).unwrap();
        assert_eq!(undone.template.name, "Original");
        assert!(history.can_redo());

        let redone = history.redo(undone).unwrap();
        assert_eq!(redone.template.name, "Dragging 2");
    }

    #[test]
    fn a_new_edit_clears_redo_history() {
        let snapshot = |name: &str| {
            let mut template = starter_template();
            template.name = name.to_owned();
            EditSnapshot {
                template,
                guides: vec![Vec::new()],
                current_page: 0,
                selection: Selection::Document,
            }
        };
        let mut history = EditHistory::default();
        history.reset(snapshot("Original"));
        history.observe(snapshot("First edit"), false);
        let original = history.undo(snapshot("First edit")).unwrap();
        history.observe(snapshot("Replacement edit"), false);

        assert_eq!(original.template.name, "Original");
        assert!(!history.can_redo());
    }

    #[test]
    fn output_names_are_safe() {
        assert_eq!(safe_stem(" ACME / Summer Catalog "), "acme-summer-catalog");
    }

    #[test]
    fn preview_text_uses_pdf_alignment_anchor_and_baseline() {
        let baseline = Pos2::new(100.0, 80.0);

        assert_eq!(
            aligned_preview_text_origin(baseline, 100.0, 80.0, 15.0, TextAlign::Left),
            Pos2::new(100.0, 65.0)
        );
        assert_eq!(
            aligned_preview_text_origin(baseline, 100.0, 80.0, 15.0, TextAlign::Center),
            Pos2::new(110.0, 65.0)
        );
        assert_eq!(
            aligned_preview_text_origin(baseline, 100.0, 80.0, 15.0, TextAlign::Right),
            Pos2::new(120.0, 65.0)
        );
    }

    #[test]
    fn older_studio_state_defaults_to_visible_enabled_guides() {
        let state: PersistedState = serde_json::from_value(serde_json::json!({
            "template": starter_template(),
            "current_path": null,
            "current_page": 0,
            "preview_data": "{}"
        }))
        .unwrap();

        assert!(state.guides.is_empty());
        assert!(state.show_guides);
        assert!(state.snap_enabled);
    }

    #[test]
    fn alignment_targets_include_page_guides_and_neighboring_layers() {
        let elements = vec![
            new_element(ElementKind::Text, 0.0),
            new_element(ElementKind::Rectangle, 0.0),
        ];
        let mut guides = Vec::new();
        let guide = EditorGuide {
            axis: GuideAxis::Vertical,
            position_pt: 100.0,
        };
        push_editor_guide(&mut guides, guide);
        push_editor_guide(&mut guides, guide);
        let (x_targets, y_targets) = alignment_targets(&elements, &[0], 612.0, 792.0, &guides);

        assert_eq!(guides, [guide]);
        assert!(x_targets.contains(&100.0));
        assert!(x_targets.contains(&306.0));
        assert!(x_targets.contains(&54.0));
        assert!(y_targets.contains(&396.0));
    }

    #[test]
    fn drag_sessions_accumulate_per_frame_pointer_movement() {
        let original = new_element(ElementKind::Text, 0.0);
        let mut drag = ElementDragState {
            page: 0,
            index: 0,
            kind: ElementDragKind::Translate,
            original: original.clone(),
            originals: vec![(0, original.clone())],
            accumulated: Vec2::ZERO,
        };

        let (_, first) = drag.advance(Vec2::new(3.0, -2.0));
        let (returned_original, second) = drag.advance(Vec2::new(4.0, -5.0));

        assert_eq!(first, Vec2::new(3.0, -2.0));
        assert_eq!(second, Vec2::new(7.0, -7.0));
        assert_eq!(returned_original, original);
    }

    #[test]
    fn ruler_drags_map_to_page_guide_coordinates() {
        let page =
            eframe::egui::Rect::from_min_max(Pos2::new(100.0, 100.0), Pos2::new(500.0, 700.0));

        assert_eq!(
            guide_position_from_pointer(GuideAxis::Vertical, Pos2::new(300.0, 80.0), page, 2.0,),
            (100.0, true)
        );
        assert_eq!(
            guide_position_from_pointer(GuideAxis::Horizontal, Pos2::new(550.0, 500.0), page, 2.0,),
            (100.0, true)
        );
        assert_eq!(
            guide_position_from_pointer(GuideAxis::Vertical, Pos2::new(80.0, 300.0), page, 2.0,),
            (0.0, false)
        );
    }

    #[test]
    fn rendered_preview_uses_variable_data_and_engine_text_layout() {
        let mut template = starter_template();
        let Element::Text(text) = &mut template.pages[0].elements[0] else {
            panic!("starter element should be text");
        };
        text.value = "Hello {{customer}}".to_owned();

        let preview =
            resolve_preview(&template, r#"{"customer":"Ada"}"#, PathBuf::from(".")).unwrap();
        let DrawCommand::Text(text) = &preview.pages[0].commands[0].command else {
            panic!("resolved preview command should be text");
        };

        assert_eq!(text.lines[0].value, "Hello Ada");
        assert!(preview.placeholder_variables.is_empty());
    }

    #[test]
    fn rendered_preview_keeps_placeholders_for_missing_values() {
        let mut template = starter_template();
        let Element::Text(text) = &mut template.pages[0].elements[0] else {
            panic!("starter element should be text");
        };
        text.value = "Hello {{first_name}} {{last_name}}".to_owned();

        let preview = resolve_preview(&template, "{}", PathBuf::from(".")).unwrap();
        let DrawCommand::Text(text) = &preview.pages[0].commands[0].command else {
            panic!("resolved preview command should be text");
        };

        assert_eq!(text.lines[0].value, "Hello {{first_name}} {{last_name}}");
        assert_eq!(preview.placeholder_variables, ["first_name", "last_name"]);
        assert_eq!(
            first_template_variable("/assets/{{profile.photo}}"),
            Some("profile.photo")
        );
    }

    #[test]
    fn rendered_preview_uses_neutral_fallbacks_for_missing_theme_colors() {
        let mut template = starter_template();
        let Element::Text(text) = &mut template.pages[0].elements[0] else {
            panic!("starter element should be text");
        };
        text.color = "{{theme.ink}}".to_owned();

        let preview = resolve_preview(&template, "{}", PathBuf::from(".")).unwrap();
        let DrawCommand::Text(text) = &preview.pages[0].commands[0].command else {
            panic!("resolved preview command should be text");
        };

        assert_eq!(
            text.color,
            PrintColor::Rgb {
                red: 128,
                green: 128,
                blue: 128
            }
        );
        assert_eq!(preview.placeholder_variables, ["theme.ink"]);
    }

    #[test]
    fn preview_converts_process_color_to_screen_rgb() {
        assert_eq!(
            print_color(PrintColor::Cmyk {
                cyan: 100.0,
                magenta: 0.0,
                yellow: 0.0,
                black: 0.0,
            }),
            Color32::from_rgb(0, 255, 255)
        );
    }

    #[test]
    fn color_controls_preserve_exact_rgb_strings_and_preview_cmyk() {
        assert_eq!(rgb_hex([0, 15, 255]), "#000FFF");
        assert_eq!(
            parse_color("cmyk(100%, 0%, 0%, 0%)"),
            Color32::from_rgb(0, 255, 255)
        );
    }

    #[test]
    fn canvas_rotation_and_resize_geometry_share_the_same_transform() {
        let transform = ScreenTransform {
            center: Pos2::new(10.0, 10.0),
            angle: 90.0_f32.to_radians(),
        };
        let rotated = transform.point(Pos2::new(12.0, 10.0));
        let local_delta = inverse_rotate_vector(Vec2::new(0.0, 2.0), transform.angle);

        assert!((rotated.x - 10.0).abs() < 0.001);
        assert!((rotated.y - 12.0).abs() < 0.001);
        assert!((local_delta.x - 2.0).abs() < 0.001);
        assert!(local_delta.y.abs() < 0.001);
    }

    #[test]
    fn preview_errors_are_split_into_readable_actionable_copy() {
        assert_eq!(
            preview_error_copy(
                "Preview layout failed: page 0, element pages[0].elements[2]: layout failed: Code 128 module size is 0.37pt; increase its width to provide at least 0.50pt per module",
            ),
            PreviewErrorCopy {
                location: Some("Page 1 · Element 3".to_owned()),
                message: "Code 128 module size is 0.37pt.".to_owned(),
                suggestion: Some(
                    "Increase its width to provide at least 0.50pt per module.".to_owned(),
                ),
            }
        );
    }
}
