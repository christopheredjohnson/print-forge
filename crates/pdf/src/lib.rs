//! PDF rendering boundary.
//!
//! The concrete `printpdf` adapter belongs in this crate; other crates should
//! only depend on the renderer-neutral engine types.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail, ensure};
use lopdf::{Document as LoDocument, Object, StringFormat};
use print_forge_engine::{
    BarcodeCommand, DrawCommand, ImageCommand, LineCommand, LineDash, QrCodeCommand,
    RectangleCommand, ResolvedDocument, ResolvedFont, StrokeCommand, SvgCommand, TextCommand,
};
use print_forge_template::Color as PrintColor;
use printpdf::{
    BuiltinFont, Cmyk, Color, CurTransMat, FontId, Line, LineDashPattern, LinePoint, Mm, Op,
    PaintMode, ParsedFont, PdfConformance, PdfDocument, PdfFont, PdfFontHandle, PdfPage,
    PdfSaveOptions, Point, Pt, RawImage, Rect, Rgb, Svg, TextItem, WindingOrder, XObject,
    XObjectId, XObjectTransform, XmpMetadata,
};
use thiserror::Error;

/// A stable, phase-specific PDF operation failure.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PdfError {
    #[error("PDF preflight failed: {0:#}")]
    Preflight(#[source] anyhow::Error),
    #[error("PDF rendering failed: {0:#}")]
    Render(#[source] anyhow::Error),
    #[error("PDF conformance validation failed: {0:#}")]
    Conformance(#[source] anyhow::Error),
}

pub type PdfResult<T> = std::result::Result<T, PdfError>;

/// Renderer boundary for applications that provide an alternative PDF backend.
pub trait DocumentRenderer {
    fn render(&self, document: &ResolvedDocument) -> PdfResult<Vec<u8>>;
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum PdfXStandard {
    #[default]
    X4,
}

impl PdfXStandard {
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::X4 => "PDF/X-4",
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct PdfRenderOptions {
    pub min_image_dpi: Option<f32>,
    pub require_embedded_fonts: bool,
    pub pdf_x: Option<PdfXStandard>,
}

impl PdfRenderOptions {
    #[must_use]
    pub fn print_ready() -> Self {
        Self {
            min_image_dpi: Some(300.0),
            require_embedded_fonts: true,
            pdf_x: Some(PdfXStandard::X4),
        }
    }
}

/// Marker for the initial `printpdf`-backed renderer implementation.
#[derive(Debug, Default, Clone, Copy)]
pub struct PdfRenderer;

impl DocumentRenderer for PdfRenderer {
    fn render(&self, document: &ResolvedDocument) -> PdfResult<Vec<u8>> {
        self.render_with_options(document, &PdfRenderOptions::default())
    }
}

impl PdfRenderer {
    pub fn render_with_options(
        &self,
        document: &ResolvedDocument,
        options: &PdfRenderOptions,
    ) -> PdfResult<Vec<u8>> {
        preflight_document(document, options)?;
        self.render_inner(document, options)
            .map_err(PdfError::Render)
    }

    fn render_inner(
        &self,
        document: &ResolvedDocument,
        options: &PdfRenderOptions,
    ) -> Result<Vec<u8>> {
        let bleed = document.bleed_pt;
        let media_width_pt = document.width_pt + bleed * 2.0;
        let media_height_pt = document.height_pt + bleed * 2.0;
        let width = points_to_mm(media_width_pt);
        let height = points_to_mm(media_height_pt);
        let mut pdf = PdfDocument::new(&document.title);
        configure_metadata(&mut pdf, document, options);
        let mut fonts = FontResources::default();
        let mut images = BTreeMap::new();
        let mut svgs = BTreeMap::new();
        let mut pages = Vec::with_capacity(document.pages.len());

        for (page_index, page) in document.pages.iter().enumerate() {
            let mut ops = vec![
                Op::SaveGraphicsState,
                Op::SetTransformationMatrix {
                    matrix: CurTransMat::Translate(Pt(bleed), Pt(bleed)),
                },
            ];

            for command in &page.commands {
                ops.push(Op::SaveGraphicsState);
                if command.rotation != 0.0
                    && let Some((center_x, center_y)) = command_center(&command.command)
                {
                    ops.push(Op::SetTransformationMatrix {
                        matrix: rotation_matrix(command.rotation, center_x, center_y),
                    });
                }
                let result = match &command.command {
                    DrawCommand::Text(text) => {
                        render_text(text, &mut pdf, &mut fonts, options, &mut ops)
                    }
                    DrawCommand::Rectangle(rectangle) => render_rectangle(rectangle, &mut ops),
                    DrawCommand::Line(line) => render_line(line, &mut ops),
                    DrawCommand::Image(image) => {
                        render_image(image, &mut pdf, &mut images, &mut ops)
                    }
                    DrawCommand::Svg(svg) => render_svg(svg, &mut pdf, &mut svgs, &mut ops),
                    DrawCommand::QrCode(qr_code) => render_qr_code(qr_code, &mut ops),
                    DrawCommand::Barcode(barcode) => render_barcode(barcode, &mut ops),
                };
                ops.push(Op::RestoreGraphicsState);

                result.with_context(|| {
                    format!("page {page_index}, element {}", command.source_path)
                })?;
            }

            ops.push(Op::RestoreGraphicsState);
            pages.push(PdfPage::new(width, height, ops));
        }

        pdf.with_pages(pages);
        let identifier = pdf.metadata.info.identifier.clone();
        let mut warnings = Vec::new();
        let mut output = printpdf::to_lopdf_doc(&pdf, &PdfSaveOptions::default(), &mut warnings);
        if options.pdf_x == Some(PdfXStandard::X4) {
            output.version = "1.6".to_owned();
        }
        apply_page_boxes(&mut output, document.width_pt, document.height_pt, bleed)?;
        normalize_document_ids(&mut output, &identifier)?;

        let mut bytes = Vec::new();
        output
            .save_to(&mut bytes)
            .context("failed to serialize deterministic PDF")?;
        if let Some(standard) = options.pdf_x {
            validate_pdf_x_inner(&bytes, standard)?;
        }
        Ok(bytes)
    }
}

fn command_center(command: &DrawCommand) -> Option<(f32, f32)> {
    let bounds = match command {
        DrawCommand::Text(command) => command.bounds,
        DrawCommand::Image(command) => command.bounds,
        DrawCommand::Rectangle(command) => command.bounds,
        DrawCommand::Svg(command) => command.bounds,
        DrawCommand::QrCode(command) => command.bounds,
        DrawCommand::Barcode(command) => command.bounds,
        DrawCommand::Line(_) => return None,
    };
    Some((
        bounds.x + bounds.width / 2.0,
        bounds.y + bounds.height / 2.0,
    ))
}

fn rotation_matrix(clockwise_degrees: f32, center_x: f32, center_y: f32) -> CurTransMat {
    let radians = (360.0 - clockwise_degrees).to_radians();
    let cosine = radians.cos();
    let sine = radians.sin();
    CurTransMat::Raw([
        cosine,
        -sine,
        sine,
        cosine,
        center_x - cosine * center_x - sine * center_y,
        center_y + sine * center_x - cosine * center_y,
    ])
}

fn configure_metadata(
    pdf: &mut PdfDocument,
    document: &ResolvedDocument,
    options: &PdfRenderOptions,
) {
    let metadata = &document.metadata;
    let info = &mut pdf.metadata.info;
    info.document_title = metadata
        .title
        .clone()
        .unwrap_or_else(|| document.title.clone());
    info.author = metadata.author.clone().unwrap_or_default();
    info.subject = metadata.subject.clone().unwrap_or_default();
    info.keywords = metadata.keywords.clone();
    info.creator = "Print Forge".to_owned();
    info.producer = format!("Print Forge {}", env!("CARGO_PKG_VERSION"));
    info.identifier = metadata
        .identifier
        .clone()
        .unwrap_or_else(|| stable_document_identifier(document));

    if let Some(standard) = options.pdf_x {
        info.conformance = match standard {
            PdfXStandard::X4 => PdfConformance::X4_2010_PDF_1_4,
        };
        pdf.metadata.xmp = Some(XmpMetadata {
            rendition_class: Some("default".to_owned()),
        });
    }
}

fn stable_document_identifier(document: &ResolvedDocument) -> String {
    let representation = format!("{document:#?}");
    let first = fnv1a(representation.as_bytes(), 0xcbf29ce484222325);
    let second = fnv1a(representation.as_bytes(), 0x84222325cbf29ce4);
    format!("{first:016x}{second:016x}")
}

fn fnv1a(bytes: &[u8], offset: u64) -> u64 {
    bytes.iter().fold(offset, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

pub fn preflight_document(
    document: &ResolvedDocument,
    options: &PdfRenderOptions,
) -> PdfResult<()> {
    preflight_document_inner(document, options).map_err(PdfError::Preflight)
}

fn preflight_document_inner(document: &ResolvedDocument, options: &PdfRenderOptions) -> Result<()> {
    ensure!(
        document.bleed_pt.is_finite() && document.bleed_pt >= 0.0,
        "document bleed must be nonnegative and finite"
    );
    if let Some(minimum) = options.min_image_dpi {
        ensure!(
            minimum.is_finite() && minimum > 0.0,
            "minimum image DPI must be positive and finite"
        );
    }

    let require_embedded_fonts = options.require_embedded_fonts || options.pdf_x.is_some();
    for (page_index, page) in document.pages.iter().enumerate() {
        for command in &page.commands {
            match &command.command {
                DrawCommand::Text(text) if require_embedded_fonts => {
                    if let ResolvedFont::Builtin(name) = &text.font {
                        ensure!(
                            bundled_font(name).is_some(),
                            "page {page_index}, element {}: built-in font {name:?} has no bundled embeddable equivalent; declare an external font family for print-ready output",
                            command.source_path
                        );
                    }
                }
                DrawCommand::Image(image) => {
                    if let Some(minimum) = options.min_image_dpi {
                        let dpi = effective_image_dpi(image).with_context(|| {
                            format!("page {page_index}, element {}", command.source_path)
                        })?;
                        ensure!(
                            dpi + 0.01 >= minimum,
                            "page {page_index}, element {}: image {} resolves to {dpi:.1} DPI; minimum is {minimum:.1} DPI",
                            command.source_path,
                            image.source.display()
                        );
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn effective_image_dpi(command: &ImageCommand) -> Result<f32> {
    ensure_supported_image(&command.source)?;
    let bytes = fs::read(&command.source)
        .with_context(|| format!("failed to read image {}", command.source.display()))?;
    let image = RawImage::decode_from_bytes(&bytes, &mut Vec::new()).map_err(|error| {
        anyhow!(
            "failed to decode image {}: {error}",
            command.source.display()
        )
    })?;
    let image_width = image.width as f32;
    let image_height = image.height as f32;
    let (bounds_width, bounds_height) = (command.bounds.width, command.bounds.height);
    let (placed_width, placed_height) = match command.fit {
        print_forge_template::ImageFit::Contain => {
            let scale = (bounds_width / image_width).min(bounds_height / image_height);
            (image_width * scale, image_height * scale)
        }
        print_forge_template::ImageFit::Cover => {
            let scale = (bounds_width / image_width).max(bounds_height / image_height);
            (image_width * scale, image_height * scale)
        }
        print_forge_template::ImageFit::Stretch => (bounds_width, bounds_height),
    };

    let horizontal = image_width * 72.0 / placed_width;
    let vertical = image_height * 72.0 / placed_height;
    Ok(horizontal.min(vertical))
}

fn apply_page_boxes(
    document: &mut LoDocument,
    trim_width: f32,
    trim_height: f32,
    bleed: f32,
) -> Result<()> {
    let media = pdf_box(
        0.0,
        0.0,
        trim_width + bleed * 2.0,
        trim_height + bleed * 2.0,
    );
    let trim = pdf_box(bleed, bleed, bleed + trim_width, bleed + trim_height);
    for (_, page_id) in document.get_pages() {
        let page = document
            .get_dictionary_mut(page_id)
            .context("failed to access generated PDF page")?;
        page.set("MediaBox", media.clone());
        page.set("CropBox", media.clone());
        page.set("BleedBox", media.clone());
        page.set("TrimBox", trim.clone());
    }
    Ok(())
}

fn pdf_box(left: f32, bottom: f32, right: f32, top: f32) -> Object {
    Object::Array(vec![
        Object::Real(left),
        Object::Real(bottom),
        Object::Real(right),
        Object::Real(top),
    ])
}

fn normalize_document_ids(document: &mut LoDocument, identifier: &str) -> Result<()> {
    let id = Object::String(identifier.as_bytes().to_vec(), StringFormat::Literal);
    document
        .trailer
        .set("ID", Object::Array(vec![id.clone(), id]));

    let metadata_id = document
        .catalog()
        .ok()
        .and_then(|catalog| catalog.get(b"Metadata").ok())
        .and_then(|object| object.as_reference().ok());
    if let Some(metadata_id) = metadata_id {
        let stream = document
            .get_object_mut(metadata_id)
            .context("failed to access generated XMP metadata")?
            .as_stream_mut()
            .context("generated XMP metadata is not a stream")?;
        let content = String::from_utf8(stream.content.clone())
            .context("generated XMP metadata is not UTF-8")?;
        stream.set_content(replace_xmp_instance_id(&content, identifier)?.into_bytes());
    }
    Ok(())
}

fn replace_xmp_instance_id(content: &str, identifier: &str) -> Result<String> {
    let start_marker = "<xmpMM:InstanceID>uuid:";
    let end_marker = "</xmpMM:InstanceID>";
    let start = content
        .find(start_marker)
        .ok_or_else(|| anyhow!("generated XMP metadata has no instance ID"))?
        + start_marker.len();
    let end = content[start..]
        .find(end_marker)
        .map(|end| start + end)
        .ok_or_else(|| anyhow!("generated XMP metadata has an unclosed instance ID"))?;
    let mut output = String::with_capacity(content.len());
    output.push_str(&content[..start]);
    output.push_str(identifier);
    output.push_str(&content[end..]);
    Ok(output)
}

pub fn validate_pdf_x(bytes: &[u8], standard: PdfXStandard) -> PdfResult<()> {
    validate_pdf_x_inner(bytes, standard).map_err(PdfError::Conformance)
}

fn validate_pdf_x_inner(bytes: &[u8], standard: PdfXStandard) -> Result<()> {
    let document = LoDocument::load_mem(bytes).context("generated PDF cannot be parsed")?;
    match standard {
        PdfXStandard::X4 => ensure!(
            document.version == "1.6",
            "PDF/X-4 requires PDF version 1.6; generated {}",
            document.version
        ),
    }
    let catalog = document.catalog().context("generated PDF has no catalog")?;
    let output_intents = catalog
        .get(b"OutputIntents")
        .context("PDF/X output intent is missing")?
        .as_array()
        .context("PDF/X output intent is malformed")?;
    ensure!(!output_intents.is_empty(), "PDF/X output intent is empty");
    let output_intent = resolve_dictionary(&document, &output_intents[0])?;
    let profile_id = output_intent
        .get(b"DestinationOutputProfile")
        .context("PDF/X destination output profile is missing")?
        .as_reference()
        .context("PDF/X destination output profile is malformed")?;
    document
        .get_object(profile_id)
        .context("PDF/X destination output profile cannot be resolved")?
        .as_stream()
        .context("PDF/X destination output profile is not an ICC stream")?;

    let metadata_id = catalog
        .get(b"Metadata")
        .context("PDF/X XMP metadata is missing")?
        .as_reference()
        .context("PDF/X XMP metadata reference is malformed")?;
    let metadata = document
        .get_object(metadata_id)
        .context("PDF/X XMP metadata cannot be resolved")?
        .as_stream()
        .context("PDF/X XMP metadata is not a stream")?;
    let metadata = String::from_utf8_lossy(&metadata.content);
    ensure!(
        metadata.contains(standard.identifier()),
        "XMP metadata does not declare {}",
        standard.identifier()
    );

    let info_id = document
        .trailer
        .get(b"Info")
        .context("PDF/X document information dictionary is missing")?
        .as_reference()
        .context("PDF/X document information reference is malformed")?;
    let info = document
        .get_dictionary(info_id)
        .context("PDF/X document information cannot be resolved")?;
    let declared = pdf_string(
        info.get(b"GTS_PDFXVersion")
            .context("GTS_PDFXVersion is missing")?,
    )?;
    ensure!(
        declared == standard.identifier(),
        "GTS_PDFXVersion declares {declared:?}, expected {:?}",
        standard.identifier()
    );

    for (page_number, page_id) in document.get_pages() {
        let page = document
            .get_dictionary(page_id)
            .with_context(|| format!("cannot inspect PDF page {page_number}"))?;
        let media = read_box(page, b"MediaBox", page_number)?;
        let trim = read_box(page, b"TrimBox", page_number)?;
        let bleed = read_box(page, b"BleedBox", page_number)?;
        ensure!(
            box_contains(media, bleed) && box_contains(bleed, trim),
            "PDF page {page_number} boxes are not nested media > bleed > trim"
        );
    }

    ensure_fonts_embedded(&document)?;
    Ok(())
}

fn resolve_dictionary<'a>(
    document: &'a LoDocument,
    object: &'a Object,
) -> Result<&'a lopdf::Dictionary> {
    match object {
        Object::Dictionary(dictionary) => Ok(dictionary),
        Object::Reference(id) => document
            .get_dictionary(*id)
            .context("referenced PDF dictionary cannot be resolved"),
        _ => bail!("PDF object is not a dictionary"),
    }
}

fn pdf_string(object: &Object) -> Result<String> {
    match object {
        Object::String(value, _) => Ok(String::from_utf8_lossy(value).into_owned()),
        _ => bail!("PDF object is not a string"),
    }
}

fn read_box(dictionary: &lopdf::Dictionary, key: &[u8], page_number: u32) -> Result<[f32; 4]> {
    let values = dictionary
        .get(key)
        .with_context(|| {
            format!(
                "PDF page {page_number} is missing /{}",
                String::from_utf8_lossy(key)
            )
        })?
        .as_array()
        .with_context(|| {
            format!(
                "PDF page {page_number} has a malformed /{}",
                String::from_utf8_lossy(key)
            )
        })?;
    ensure!(
        values.len() == 4,
        "PDF page {page_number} box must have four coordinates"
    );
    Ok([
        pdf_number(&values[0])?,
        pdf_number(&values[1])?,
        pdf_number(&values[2])?,
        pdf_number(&values[3])?,
    ])
}

fn pdf_number(object: &Object) -> Result<f32> {
    match object {
        Object::Integer(value) => Ok(*value as f32),
        Object::Real(value) => Ok(*value),
        _ => bail!("PDF box coordinate is not numeric"),
    }
}

fn box_contains(outer: [f32; 4], inner: [f32; 4]) -> bool {
    outer[0] <= inner[0] && outer[1] <= inner[1] && outer[2] >= inner[2] && outer[3] >= inner[3]
}

fn ensure_fonts_embedded(document: &LoDocument) -> Result<()> {
    const BASE14: &[&[u8]] = &[
        b"Courier",
        b"Courier-Bold",
        b"Courier-Oblique",
        b"Courier-BoldOblique",
        b"Helvetica",
        b"Helvetica-Bold",
        b"Helvetica-Oblique",
        b"Helvetica-BoldOblique",
        b"Times-Roman",
        b"Times-Bold",
        b"Times-Italic",
        b"Times-BoldItalic",
        b"Symbol",
        b"ZapfDingbats",
    ];
    for object in document.objects.values() {
        let dictionary = match object {
            Object::Dictionary(dictionary) => dictionary,
            Object::Stream(stream) => &stream.dict,
            _ => continue,
        };
        let Ok(base_font) = dictionary.get(b"BaseFont").and_then(Object::as_name) else {
            continue;
        };
        ensure!(
            !BASE14.contains(&base_font),
            "PDF/X output references unembedded base-14 font {}",
            String::from_utf8_lossy(base_font)
        );

        let subtype = dictionary
            .get(b"Subtype")
            .and_then(Object::as_name)
            .unwrap_or_default();
        let font = if subtype == b"Type0" {
            let descendants = dictionary
                .get(b"DescendantFonts")
                .context("embedded Type0 font has no descendant font")?
                .as_array()
                .context("embedded Type0 descendant fonts are malformed")?;
            let descendant = descendants
                .first()
                .ok_or_else(|| anyhow!("embedded Type0 font has an empty descendant list"))?;
            resolve_dictionary(document, descendant)?
        } else {
            dictionary
        };
        let descriptor = font
            .get(b"FontDescriptor")
            .context("font has no FontDescriptor and is not embedded")?;
        let descriptor = resolve_dictionary(document, descriptor)?;
        ensure!(
            descriptor.has(b"FontFile")
                || descriptor.has(b"FontFile2")
                || descriptor.has(b"FontFile3"),
            "font {} has no embedded font program",
            String::from_utf8_lossy(base_font)
        );
    }
    Ok(())
}

fn render_text(
    command: &TextCommand,
    pdf: &mut PdfDocument,
    fonts: &mut FontResources,
    options: &PdfRenderOptions,
    ops: &mut Vec<Op>,
) -> Result<()> {
    let font = pdf_font(
        &command.font,
        pdf,
        fonts,
        options.require_embedded_fonts || options.pdf_x.is_some(),
    )?;
    let color = pdf_color(command.color);

    ops.push(Op::SaveGraphicsState);
    if command.clip {
        push_clip_rect(command.bounds, ops);
    }
    for line in &command.lines {
        ops.extend([
            Op::StartTextSection,
            Op::SetTextCursor {
                pos: pdf_point(line.x, line.y),
            },
            Op::SetFont {
                font: font.clone(),
                size: Pt(command.font_size_pt),
            },
            Op::SetLineHeight {
                lh: Pt(command.line_height_pt),
            },
            Op::SetFillColor { col: color.clone() },
            Op::SetWordSpacing {
                pt: Pt(line.word_spacing_pt),
            },
            Op::ShowText {
                items: vec![TextItem::Text(line.value.clone())],
            },
            Op::EndTextSection,
        ]);
    }

    ops.push(Op::RestoreGraphicsState);
    Ok(())
}

#[derive(Default)]
struct FontResources {
    external: BTreeMap<(PathBuf, u32), PdfFontHandle>,
    bundled: BTreeMap<&'static str, PdfFontHandle>,
}

impl FontResources {
    fn next_id(&self) -> FontId {
        FontId(format!(
            "font-{:04}",
            self.external.len() + self.bundled.len() + 1
        ))
    }
}

fn pdf_font(
    font: &ResolvedFont,
    pdf: &mut PdfDocument,
    fonts: &mut FontResources,
    embed_builtins: bool,
) -> Result<PdfFontHandle> {
    match font {
        ResolvedFont::Builtin(name) if embed_builtins => {
            let (canonical_name, bytes) = bundled_font(name).ok_or_else(|| {
                anyhow!(
                    "built-in font {name:?} has no bundled embeddable equivalent; declare an external font family"
                )
            })?;
            if let Some(font) = fonts.bundled.get(canonical_name) {
                return Ok(font.clone());
            }
            let parsed = ParsedFont::from_bytes(bytes, 0, &mut Vec::new())
                .ok_or_else(|| anyhow!("failed to parse bundled font {canonical_name:?}"))?;
            let id = fonts.next_id();
            pdf.resources
                .fonts
                .map
                .insert(id.clone(), PdfFont::new(parsed));
            let handle = PdfFontHandle::External(id);
            fonts.bundled.insert(canonical_name, handle.clone());
            Ok(handle)
        }
        ResolvedFont::Builtin(name) => Ok(PdfFontHandle::Builtin(builtin_font(name)?)),
        ResolvedFont::External(path) => external_pdf_font(path, 0, pdf, fonts),
        ResolvedFont::ExternalFace { path, face_index } => {
            external_pdf_font(path, *face_index, pdf, fonts)
        }
    }
}

fn external_pdf_font(
    path: &Path,
    face_index: u32,
    pdf: &mut PdfDocument,
    fonts: &mut FontResources,
) -> Result<PdfFontHandle> {
    let key = (path.to_owned(), face_index);
    if let Some(font) = fonts.external.get(&key) {
        return Ok(font.clone());
    }
    let bytes =
        fs::read(path).with_context(|| format!("failed to read font {}", path.display()))?;
    let parsed =
        ParsedFont::from_bytes(&bytes, face_index as usize, &mut Vec::new()).ok_or_else(|| {
            anyhow!(
                "failed to parse font {} at face index {face_index}",
                path.display()
            )
        })?;
    let id = fonts.next_id();
    pdf.resources
        .fonts
        .map
        .insert(id.clone(), PdfFont::new(parsed));
    let handle = PdfFontHandle::External(id);
    fonts.external.insert(key, handle.clone());
    Ok(handle)
}

fn bundled_font(name: &str) -> Option<(&'static str, &'static [u8])> {
    match name.to_ascii_lowercase().as_str() {
        "helvetica" | "sans-serif" => {
            Some(("helvetica", include_bytes!("../assets/fonts/Helvetica.ttf")))
        }
        "helvetica-bold" => Some((
            "helvetica-bold",
            include_bytes!("../assets/fonts/Helvetica-Bold.ttf"),
        )),
        "helvetica-oblique" => Some((
            "helvetica-oblique",
            include_bytes!("../assets/fonts/Helvetica-Oblique.ttf"),
        )),
        "helvetica-bold-oblique" => Some((
            "helvetica-bold-oblique",
            include_bytes!("../assets/fonts/Helvetica-BoldOblique.ttf"),
        )),
        _ => None,
    }
}

#[derive(Debug, Clone)]
struct SvgResource {
    id: XObjectId,
    width: f32,
    height: f32,
}

fn render_svg(
    command: &SvgCommand,
    pdf: &mut PdfDocument,
    svgs: &mut BTreeMap<PathBuf, SvgResource>,
    ops: &mut Vec<Op>,
) -> Result<()> {
    let resource = if let Some(resource) = svgs.get(&command.source) {
        resource.clone()
    } else {
        let source = fs::read_to_string(&command.source)
            .with_context(|| format!("failed to read SVG {}", command.source.display()))?;
        let mut warnings = Vec::new();
        let parsed = Svg::parse(&source, &mut warnings).map_err(|error| {
            anyhow!("failed to parse SVG {}: {error}", command.source.display())
        })?;
        let width = parsed
            .width
            .ok_or_else(|| anyhow!("SVG {} has no intrinsic width", command.source.display()))?
            .0 as f32;
        let height = parsed
            .height
            .ok_or_else(|| anyhow!("SVG {} has no intrinsic height", command.source.display()))?
            .0 as f32;
        ensure!(
            width > 0.0 && height > 0.0,
            "SVG {} must have positive intrinsic dimensions",
            command.source.display()
        );
        let resource = SvgResource {
            id: pdf.add_xobject(&parsed),
            width,
            height,
        };
        svgs.insert(command.source.clone(), resource.clone());
        resource
    };

    let (scale_x, scale_y) = match command.fit {
        print_forge_template::ImageFit::Contain => {
            let scale = (command.bounds.width / resource.width)
                .min(command.bounds.height / resource.height);
            (scale, scale)
        }
        print_forge_template::ImageFit::Cover => {
            let scale = (command.bounds.width / resource.width)
                .max(command.bounds.height / resource.height);
            (scale, scale)
        }
        print_forge_template::ImageFit::Stretch => (
            command.bounds.width / resource.width,
            command.bounds.height / resource.height,
        ),
    };
    let placed_width = resource.width * scale_x;
    let placed_height = resource.height * scale_y;
    let x = command.bounds.x + (command.bounds.width - placed_width) / 2.0;
    let y = command.bounds.y + (command.bounds.height - placed_height) / 2.0;
    ops.push(Op::SaveGraphicsState);
    if command.fit == print_forge_template::ImageFit::Cover {
        push_clip_rect(command.bounds, ops);
    }
    ops.push(Op::UseXobject {
        id: resource.id,
        transform: XObjectTransform {
            translate_x: Some(Pt(x)),
            translate_y: Some(Pt(y)),
            scale_x: Some(scale_x),
            scale_y: Some(scale_y),
            dpi: Some(72.0),
            ..Default::default()
        },
    });
    ops.push(Op::RestoreGraphicsState);
    Ok(())
}

fn render_qr_code(command: &QrCodeCommand, ops: &mut Vec<Op>) -> Result<()> {
    render_code_background(command.bounds, command.background, ops)?;
    let total_modules = command.size + usize::from(command.quiet_zone) * 2;
    let module_size = command.bounds.width / total_modules as f32;
    let quiet = usize::from(command.quiet_zone);

    ops.extend([
        Op::SaveGraphicsState,
        Op::SetFillColor {
            col: pdf_color(command.color),
        },
    ]);
    for y in 0..command.size {
        let mut x = 0;
        while x < command.size {
            if !command.modules[y * command.size + x] {
                x += 1;
                continue;
            }
            let start = x;
            while x < command.size && command.modules[y * command.size + x] {
                x += 1;
            }
            ops.push(Op::DrawRectangle {
                rectangle: Rect {
                    x: Pt(command.bounds.x + (quiet + start) as f32 * module_size),
                    y: Pt(command.bounds.y + (quiet + command.size - y - 1) as f32 * module_size),
                    width: Pt((x - start) as f32 * module_size),
                    height: Pt(module_size),
                    mode: Some(PaintMode::Fill),
                    winding_order: Some(WindingOrder::NonZero),
                },
            });
        }
    }
    ops.push(Op::RestoreGraphicsState);
    Ok(())
}

fn render_barcode(command: &BarcodeCommand, ops: &mut Vec<Op>) -> Result<()> {
    render_code_background(command.bounds, command.background, ops)?;
    let total_modules = command.modules.len() + usize::from(command.quiet_zone) * 2;
    let module_size = command.bounds.width / total_modules as f32;
    let quiet = usize::from(command.quiet_zone);

    ops.extend([
        Op::SaveGraphicsState,
        Op::SetFillColor {
            col: pdf_color(command.color),
        },
    ]);
    let mut x = 0;
    while x < command.modules.len() {
        if !command.modules[x] {
            x += 1;
            continue;
        }
        let start = x;
        while x < command.modules.len() && command.modules[x] {
            x += 1;
        }
        ops.push(Op::DrawRectangle {
            rectangle: Rect {
                x: Pt(command.bounds.x + (quiet + start) as f32 * module_size),
                y: Pt(command.bounds.y),
                width: Pt((x - start) as f32 * module_size),
                height: Pt(command.bounds.height),
                mode: Some(PaintMode::Fill),
                winding_order: Some(WindingOrder::NonZero),
            },
        });
    }
    ops.push(Op::RestoreGraphicsState);
    Ok(())
}

fn render_code_background(
    bounds: print_forge_engine::Rect,
    color: PrintColor,
    ops: &mut Vec<Op>,
) -> Result<()> {
    render_rectangle(
        &RectangleCommand {
            bounds,
            fill: Some(color),
            stroke: None,
        },
        ops,
    )
}

#[derive(Debug, Clone)]
struct ImageResource {
    id: XObjectId,
    width: f32,
    height: f32,
}

fn render_image(
    command: &ImageCommand,
    pdf: &mut PdfDocument,
    images: &mut BTreeMap<PathBuf, ImageResource>,
    ops: &mut Vec<Op>,
) -> Result<()> {
    let resource = if let Some(resource) = images.get(&command.source) {
        resource.clone()
    } else {
        ensure_supported_image(&command.source)?;
        let bytes = fs::read(&command.source)
            .with_context(|| format!("failed to read image {}", command.source.display()))?;
        let image = RawImage::decode_from_bytes(&bytes, &mut Vec::new()).map_err(|error| {
            anyhow!(
                "failed to decode image {}: {error}",
                command.source.display()
            )
        })?;
        let resource = ImageResource {
            id: XObjectId(format!("image-{:04}", images.len() + 1)),
            width: image.width as f32,
            height: image.height as f32,
        };
        pdf.resources
            .xobjects
            .map
            .insert(resource.id.clone(), XObject::Image(image));
        images.insert(command.source.clone(), resource.clone());
        resource
    };
    let image_width = resource.width;
    let image_height = resource.height;
    let (width, height) = (command.bounds.width, command.bounds.height);

    let (scale_x, scale_y) = match command.fit {
        print_forge_template::ImageFit::Contain => {
            let scale = (width / image_width).min(height / image_height);
            (scale, scale)
        }
        print_forge_template::ImageFit::Cover => {
            let scale = (width / image_width).max(height / image_height);
            (scale, scale)
        }
        print_forge_template::ImageFit::Stretch => (width / image_width, height / image_height),
    };
    let placed_width = image_width * scale_x;
    let placed_height = image_height * scale_y;
    let x = command.bounds.x + (width - placed_width) / 2.0;
    let y = command.bounds.y + (height - placed_height) / 2.0;
    ops.push(Op::SaveGraphicsState);
    if command.fit == print_forge_template::ImageFit::Cover {
        push_clip_rect(command.bounds, ops);
    }
    ops.push(Op::UseXobject {
        id: resource.id,
        transform: XObjectTransform {
            translate_x: Some(Pt(x)),
            translate_y: Some(Pt(y)),
            scale_x: Some(scale_x),
            scale_y: Some(scale_y),
            dpi: Some(72.0),
            ..Default::default()
        },
    });
    ops.push(Op::RestoreGraphicsState);
    Ok(())
}

fn ensure_supported_image(path: &Path) -> Result<()> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
    {
        Some(extension) if matches!(extension.as_str(), "png" | "jpg" | "jpeg") => Ok(()),
        _ => bail!("image must be a local PNG or JPEG file: {}", path.display()),
    }
}

fn push_clip_rect(bounds: print_forge_engine::Rect, ops: &mut Vec<Op>) {
    ops.push(Op::DrawRectangle {
        rectangle: Rect {
            x: Pt(bounds.x),
            y: Pt(bounds.y),
            width: Pt(bounds.width),
            height: Pt(bounds.height),
            mode: Some(PaintMode::Clip),
            winding_order: Some(WindingOrder::NonZero),
        },
    });
}

fn render_rectangle(command: &RectangleCommand, ops: &mut Vec<Op>) -> Result<()> {
    let mode = match (&command.fill, &command.stroke) {
        (Some(_), Some(_)) => PaintMode::FillStroke,
        (Some(_), None) => PaintMode::Fill,
        (None, Some(_)) => PaintMode::Stroke,
        (None, None) => return Ok(()),
    };

    ops.push(Op::SaveGraphicsState);

    if let Some(fill) = &command.fill {
        ops.push(Op::SetFillColor {
            col: pdf_color(*fill),
        });
    }

    if let Some(stroke) = &command.stroke {
        push_stroke(stroke, ops)?;
    }

    ops.push(Op::DrawRectangle {
        rectangle: Rect {
            x: Pt(command.bounds.x),
            y: Pt(command.bounds.y),
            width: Pt(command.bounds.width),
            height: Pt(command.bounds.height),
            mode: Some(mode),
            winding_order: Some(WindingOrder::NonZero),
        },
    });
    ops.push(Op::RestoreGraphicsState);
    Ok(())
}

fn render_line(command: &LineCommand, ops: &mut Vec<Op>) -> Result<()> {
    ops.extend([
        Op::SaveGraphicsState,
        Op::SetOutlineColor {
            col: pdf_color(command.color),
        },
        Op::SetOutlineThickness {
            pt: Pt(command.width_pt),
        },
        Op::SetLineDashPattern {
            dash: dash_pattern(command.dash),
        },
        Op::DrawLine {
            line: Line {
                points: vec![
                    LinePoint {
                        p: pdf_point(command.start.x, command.start.y),
                        bezier: false,
                    },
                    LinePoint {
                        p: pdf_point(command.end.x, command.end.y),
                        bezier: false,
                    },
                ],
                is_closed: false,
            },
        },
        Op::RestoreGraphicsState,
    ]);
    Ok(())
}

fn push_stroke(stroke: &StrokeCommand, ops: &mut Vec<Op>) -> Result<()> {
    ops.extend([
        Op::SetOutlineColor {
            col: pdf_color(stroke.color),
        },
        Op::SetOutlineThickness {
            pt: Pt(stroke.width_pt),
        },
        Op::SetLineDashPattern {
            dash: dash_pattern(stroke.dash),
        },
    ]);
    Ok(())
}

fn builtin_font(name: &str) -> Result<BuiltinFont> {
    let font = match name.to_ascii_lowercase().as_str() {
        "helvetica" | "sans-serif" => BuiltinFont::Helvetica,
        "helvetica-bold" => BuiltinFont::HelveticaBold,
        "helvetica-oblique" => BuiltinFont::HelveticaOblique,
        "helvetica-bold-oblique" => BuiltinFont::HelveticaBoldOblique,
        "times" | "times-roman" | "serif" => BuiltinFont::TimesRoman,
        "times-bold" => BuiltinFont::TimesBold,
        "times-italic" => BuiltinFont::TimesItalic,
        "times-bold-italic" => BuiltinFont::TimesBoldItalic,
        "courier" | "monospace" => BuiltinFont::Courier,
        "courier-bold" => BuiltinFont::CourierBold,
        "courier-oblique" => BuiltinFont::CourierOblique,
        "courier-bold-oblique" => BuiltinFont::CourierBoldOblique,
        _ => bail!("unsupported built-in font: {name}"),
    };

    Ok(font)
}

fn pdf_color(color: PrintColor) -> Color {
    let components = color.normalized();
    match color {
        PrintColor::Rgb { .. } => {
            Color::Rgb(Rgb::new(components[0], components[1], components[2], None))
        }
        PrintColor::Cmyk { .. } => Color::Cmyk(Cmyk::new(
            components[0],
            components[1],
            components[2],
            components[3],
            None,
        )),
    }
}

fn dash_pattern(dash: LineDash) -> LineDashPattern {
    match dash {
        LineDash::Solid => LineDashPattern::solid(),
        LineDash::Dashed => LineDashPattern::new(0.0, &[6.0, 3.0]),
        LineDash::Dotted => LineDashPattern::new(0.0, &[1.0, 3.0]),
    }
}

fn pdf_point(x: f32, y: f32) -> Point {
    Point { x: Pt(x), y: Pt(y) }
}

fn points_to_mm(points: f32) -> Mm {
    Mm(points * 25.4 / 72.0)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use print_forge_engine::{
        BasicLayoutEngine, DrawCommand, ImageCommand, LayoutOptions, LineCommand, LineDash, Point,
        Rect, ResolvedCommand, ResolvedDocument, ResolvedFont, ResolvedPage, TextCommand, TextLine,
    };
    use print_forge_template::Template;
    use printpdf::{PdfDocument, PdfParseOptions};
    use serde_json::json;

    use super::{
        DocumentRenderer, PdfRenderOptions, PdfRenderer, PdfXStandard, read_box, rotation_matrix,
        validate_pdf_x,
    };

    fn print_ready_fixture() -> ResolvedDocument {
        let template: Template = serde_json::from_str(include_str!(
            "../../../examples/business-card/template.json"
        ))
        .unwrap();
        let row = serde_json::from_value(json!({
            "first_name": "Ada",
            "last_name": "Lovelace",
            "title": "Engineer"
        }))
        .unwrap();
        let asset_base =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/business-card");
        BasicLayoutEngine
            .layout_with_options(&template, &row, &LayoutOptions { asset_base })
            .unwrap()
    }

    fn specialty_fixture() -> ResolvedDocument {
        let template: Template = serde_json::from_str(include_str!(
            "../../../examples/specialty-elements/template.json"
        ))
        .unwrap();
        let rows = serde_json::from_str::<Vec<serde_json::Value>>(include_str!(
            "../../../examples/specialty-elements/data.json"
        ))
        .unwrap();
        let row = rows[0].as_object().unwrap().clone();
        let asset_base =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/specialty-elements");
        BasicLayoutEngine
            .layout_with_options(&template, &row, &LayoutOptions { asset_base })
            .unwrap()
    }

    #[test]
    fn renders_a_parseable_pdf() {
        let document = ResolvedDocument {
            title: "Renderer test".to_owned(),
            width_pt: 252.0,
            height_pt: 144.0,
            bleed_pt: 0.0,
            metadata: Default::default(),
            pages: vec![ResolvedPage {
                commands: vec![
                    ResolvedCommand {
                        source_path: "pages[0].elements[0]".to_owned(),
                        rotation: 30.0,
                        command: DrawCommand::Text(TextCommand {
                            bounds: Rect {
                                x: 36.0,
                                y: 96.0,
                                width: 180.0,
                                height: 18.0,
                            },
                            lines: vec![TextLine {
                                value: "Ada Lovelace".to_owned(),
                                x: 36.0,
                                y: 96.0,
                                width_pt: 90.0,
                                word_spacing_pt: 0.0,
                            }],
                            font_size_pt: 16.0,
                            line_height_pt: 19.2,
                            align: print_forge_template::TextAlign::Left,
                            font: ResolvedFont::Builtin("helvetica".to_owned()),
                            color: "#112233".parse().unwrap(),
                            clip: false,
                        }),
                    },
                    ResolvedCommand {
                        source_path: "pages[0].elements[1]".to_owned(),
                        rotation: 0.0,
                        command: DrawCommand::Line(LineCommand {
                            start: Point { x: 36.0, y: 72.0 },
                            end: Point { x: 216.0, y: 72.0 },
                            width_pt: 0.5,
                            color: "#000000".parse().unwrap(),
                            dash: LineDash::Solid,
                        }),
                    },
                ],
            }],
        };

        let bytes = PdfRenderer.render(&document).unwrap();
        let parsed =
            PdfDocument::parse(&bytes, &PdfParseOptions::default(), &mut Vec::new()).unwrap();
        let lopdf = lopdf::Document::load_mem(&bytes).unwrap();
        let page_id = lopdf.get_pages()[&1];
        let content_bytes = lopdf.get_page_content(page_id);
        let content = String::from_utf8_lossy(&content_bytes);

        assert!(bytes.starts_with(b"%PDF-"));
        assert_eq!(parsed.pages.len(), 1);
        assert!(
            content.contains("0.866"),
            "rotation matrix should be emitted"
        );
    }

    #[test]
    fn rotation_matrix_keeps_the_element_center_fixed() {
        let matrix = rotation_matrix(90.0, 10.0, 20.0).as_array();
        let transformed_x = matrix[0] * 10.0 + matrix[2] * 20.0 + matrix[4];
        let transformed_y = matrix[1] * 10.0 + matrix[3] * 20.0 + matrix[5];

        assert!((transformed_x - 10.0).abs() < 0.001);
        assert!((transformed_y - 20.0).abs() < 0.001);
    }

    #[test]
    fn rendering_errors_include_page_and_element_paths() {
        let document = ResolvedDocument {
            title: "Renderer error".to_owned(),
            width_pt: 100.0,
            height_pt: 100.0,
            bleed_pt: 0.0,
            metadata: Default::default(),
            pages: vec![ResolvedPage {
                commands: vec![ResolvedCommand {
                    source_path: "pages[0].elements[3]".to_owned(),
                    rotation: 0.0,
                    command: DrawCommand::Image(ImageCommand {
                        bounds: Rect {
                            x: 0.0,
                            y: 0.0,
                            width: 50.0,
                            height: 50.0,
                        },
                        source: PathBuf::from("logo.png"),
                        fit: print_forge_template::ImageFit::Contain,
                    }),
                }],
            }],
        };

        let error = PdfRenderer.render(&document).unwrap_err().to_string();

        assert!(error.contains("page 0, element pages[0].elements[3]"));
    }

    #[test]
    fn renders_svg_qr_and_barcode_as_vector_pdf_content() {
        let bytes = PdfRenderer.render(&specialty_fixture()).unwrap();
        let parsed = lopdf::Document::load_mem(&bytes).unwrap();
        let page_id = parsed.get_pages()[&1];
        let content_bytes = parsed.get_page_content(page_id);
        let content = String::from_utf8_lossy(&content_bytes);
        let subtypes = parsed
            .objects
            .values()
            .filter_map(|object| object.as_stream().ok())
            .filter_map(|stream| stream.dict.get(b"Subtype").ok())
            .filter_map(|subtype| subtype.as_name().ok())
            .collect::<Vec<_>>();

        assert!(
            content.contains(" Do"),
            "SVG should be invoked as an XObject"
        );
        assert!(
            content.matches(" re").count() > 50,
            "QR and barcode modules should remain vector rectangles"
        );
        assert!(subtypes.contains(&b"Form".as_slice()));
        assert!(!subtypes.contains(&b"Image".as_slice()));
    }

    #[test]
    fn print_ready_output_has_pdf_x_metadata_and_explicit_page_boxes() {
        let document = print_ready_fixture();
        let bytes = PdfRenderer
            .render_with_options(&document, &PdfRenderOptions::print_ready())
            .unwrap();

        validate_pdf_x(&bytes, PdfXStandard::X4).unwrap();
        let parsed_metadata =
            PdfDocument::parse(&bytes, &PdfParseOptions::default(), &mut Vec::new()).unwrap();
        let parsed = lopdf::Document::load_mem(&bytes).unwrap();
        let page_id = parsed.get_pages()[&1];
        let page = parsed.get_dictionary(page_id).unwrap();
        let content_bytes = parsed.get_page_content(page_id);
        let content = String::from_utf8_lossy(&content_bytes);

        assert_eq!(parsed.version, "1.6");
        assert_eq!(
            parsed_metadata.metadata.info.document_title,
            "Employee Business Cards"
        );
        assert_eq!(parsed_metadata.metadata.info.author, "Print Forge");
        assert_eq!(
            read_box(page, b"MediaBox", 1).unwrap(),
            [0.0, 0.0, 270.0, 162.0]
        );
        assert_eq!(
            read_box(page, b"BleedBox", 1).unwrap(),
            [0.0, 0.0, 270.0, 162.0]
        );
        assert_eq!(
            read_box(page, b"TrimBox", 1).unwrap(),
            [9.0, 9.0, 261.0, 153.0]
        );
        assert!(content.contains("0 0 0 1 k"));
        assert!(content.contains("1 0.55 0 0.15 K"));
    }

    #[test]
    fn rendering_is_byte_for_byte_deterministic() {
        let document = print_ready_fixture();
        let options = PdfRenderOptions::print_ready();

        let first = PdfRenderer
            .render_with_options(&document, &options)
            .unwrap();
        let second = PdfRenderer
            .render_with_options(&document, &options)
            .unwrap();

        assert_eq!(first, second);
    }

    #[test]
    fn print_ready_embeds_bundled_helvetica_variants() {
        for name in [
            "helvetica",
            "helvetica-bold",
            "helvetica-oblique",
            "helvetica-bold-oblique",
        ] {
            let mut document = print_ready_fixture();
            let DrawCommand::Text(text) = &mut document.pages[0].commands[0].command else {
                panic!("expected text command");
            };
            text.font = ResolvedFont::Builtin(name.to_owned());

            let bytes = PdfRenderer
                .render_with_options(&document, &PdfRenderOptions::print_ready())
                .unwrap();

            validate_pdf_x(&bytes, PdfXStandard::X4).unwrap();
        }
    }

    #[test]
    fn print_preflight_rejects_builtins_without_an_embeddable_equivalent() {
        let mut document = print_ready_fixture();
        let DrawCommand::Text(text) = &mut document.pages[0].commands[0].command else {
            panic!("expected text command");
        };
        text.font = ResolvedFont::Builtin("times-roman".to_owned());

        let error = PdfRenderer
            .render_with_options(&document, &PdfRenderOptions::print_ready())
            .unwrap_err()
            .to_string();

        assert!(error.contains("has no bundled embeddable equivalent"));
        assert!(error.contains("declare an external font family"));
    }

    #[test]
    fn image_preflight_rejects_effective_dpi_below_the_configured_minimum() {
        let mut document = print_ready_fixture();
        let image = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/absolute-layout/assets/images/layout-fixture.png");
        document.pages[0].commands.push(ResolvedCommand {
            source_path: "pages[0].elements[3]".to_owned(),
            rotation: 0.0,
            command: DrawCommand::Image(ImageCommand {
                bounds: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 72.0,
                    height: 72.0,
                },
                source: image,
                fit: print_forge_template::ImageFit::Stretch,
            }),
        });
        let options = PdfRenderOptions {
            min_image_dpi: Some(300.0),
            ..Default::default()
        };

        let error = PdfRenderer
            .render_with_options(&document, &options)
            .unwrap_err()
            .to_string();

        assert!(error.contains("minimum is 300.0 DPI"));
        assert!(error.contains("pages[0].elements[3]"));
    }
}
