//! PDF rendering boundary.
//!
//! The concrete `printpdf` adapter belongs in this crate; other crates should
//! only depend on the renderer-neutral engine types.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};
use print_forge_engine::{
    DrawCommand, ImageCommand, LineCommand, LineDash, RectangleCommand, ResolvedDocument,
    ResolvedFont, StrokeCommand, TextCommand,
};
use printpdf::{
    BuiltinFont, Color, Line, LineDashPattern, LinePoint, Mm, Op, PaintMode, ParsedFont,
    PdfDocument, PdfFontHandle, PdfPage, PdfSaveOptions, Point, Pt, RawImage, Rect, Rgb, TextItem,
    WindingOrder, XObjectTransform,
};

pub trait DocumentRenderer {
    fn render(&self, document: &ResolvedDocument) -> Result<Vec<u8>>;
}

/// Marker for the initial `printpdf`-backed renderer implementation.
#[derive(Debug, Default, Clone, Copy)]
pub struct PdfRenderer;

impl DocumentRenderer for PdfRenderer {
    fn render(&self, document: &ResolvedDocument) -> Result<Vec<u8>> {
        let width = points_to_mm(document.width_pt);
        let height = points_to_mm(document.height_pt);
        let mut pdf = PdfDocument::new(&document.title);
        let mut fonts = HashMap::new();
        let mut pages = Vec::with_capacity(document.pages.len());

        for (page_index, page) in document.pages.iter().enumerate() {
            let mut ops = Vec::new();

            for command in &page.commands {
                let result = match &command.command {
                    DrawCommand::Text(text) => render_text(text, &mut pdf, &mut fonts, &mut ops),
                    DrawCommand::Rectangle(rectangle) => render_rectangle(rectangle, &mut ops),
                    DrawCommand::Line(line) => render_line(line, &mut ops),
                    DrawCommand::Image(image) => render_image(image, &mut pdf, &mut ops),
                    DrawCommand::Svg(_) => Err(anyhow!("SVG rendering is not implemented yet")),
                };

                result.with_context(|| {
                    format!("page {page_index}, element {}", command.source_path)
                })?;
            }

            pages.push(PdfPage::new(width, height, ops));
        }

        pdf.with_pages(pages);

        Ok(pdf.save(&PdfSaveOptions::default(), &mut Vec::new()))
    }
}

fn render_text(
    command: &TextCommand,
    pdf: &mut PdfDocument,
    fonts: &mut HashMap<PathBuf, PdfFontHandle>,
    ops: &mut Vec<Op>,
) -> Result<()> {
    let font = pdf_font(&command.font, pdf, fonts)?;
    let color = parse_color(&command.color)?;

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

fn pdf_font(
    font: &ResolvedFont,
    pdf: &mut PdfDocument,
    fonts: &mut HashMap<PathBuf, PdfFontHandle>,
) -> Result<PdfFontHandle> {
    match font {
        ResolvedFont::Builtin(name) => Ok(PdfFontHandle::Builtin(builtin_font(name)?)),
        ResolvedFont::External(path) => {
            if let Some(font) = fonts.get(path) {
                return Ok(font.clone());
            }
            let bytes = fs::read(path)
                .with_context(|| format!("failed to read font {}", path.display()))?;
            let parsed = ParsedFont::from_bytes(&bytes, 0, &mut Vec::new())
                .ok_or_else(|| anyhow!("failed to parse font {}", path.display()))?;
            let handle = PdfFontHandle::External(pdf.add_font(&parsed));
            fonts.insert(path.clone(), handle.clone());
            Ok(handle)
        }
    }
}

fn render_image(command: &ImageCommand, pdf: &mut PdfDocument, ops: &mut Vec<Op>) -> Result<()> {
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
    let image_id = pdf.add_image(&image);

    ops.push(Op::SaveGraphicsState);
    if command.fit == print_forge_template::ImageFit::Cover {
        push_clip_rect(command.bounds, ops);
    }
    ops.push(Op::UseXobject {
        id: image_id,
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
            col: parse_color(fill)?,
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
            col: parse_color(&command.color)?,
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
            col: parse_color(&stroke.color)?,
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

fn parse_color(value: &str) -> Result<Color> {
    let hex = value.strip_prefix('#').unwrap_or(value);
    if hex.len() != 6 {
        bail!("color must use #RRGGBB notation: {value}");
    }

    let red = u8::from_str_radix(&hex[0..2], 16)?;
    let green = u8::from_str_radix(&hex[2..4], 16)?;
    let blue = u8::from_str_radix(&hex[4..6], 16)?;

    Ok(Color::Rgb(Rgb::new(
        f32::from(red) / 255.0,
        f32::from(green) / 255.0,
        f32::from(blue) / 255.0,
        None,
    )))
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
        DrawCommand, ImageCommand, LineCommand, LineDash, Point, Rect, ResolvedCommand,
        ResolvedDocument, ResolvedFont, ResolvedPage, TextCommand, TextLine,
    };
    use printpdf::{PdfDocument, PdfParseOptions};

    use super::{DocumentRenderer, PdfRenderer};

    #[test]
    fn renders_a_parseable_pdf() {
        let document = ResolvedDocument {
            title: "Renderer test".to_owned(),
            width_pt: 252.0,
            height_pt: 144.0,
            pages: vec![ResolvedPage {
                commands: vec![
                    ResolvedCommand {
                        source_path: "pages[0].elements[0]".to_owned(),
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
                                word_spacing_pt: 0.0,
                            }],
                            font_size_pt: 16.0,
                            line_height_pt: 19.2,
                            font: ResolvedFont::Builtin("helvetica".to_owned()),
                            color: "#112233".to_owned(),
                            clip: false,
                        }),
                    },
                    ResolvedCommand {
                        source_path: "pages[0].elements[1]".to_owned(),
                        command: DrawCommand::Line(LineCommand {
                            start: Point { x: 36.0, y: 72.0 },
                            end: Point { x: 216.0, y: 72.0 },
                            width_pt: 0.5,
                            color: "#000000".to_owned(),
                            dash: LineDash::Solid,
                        }),
                    },
                ],
            }],
        };

        let bytes = PdfRenderer.render(&document).unwrap();
        let parsed =
            PdfDocument::parse(&bytes, &PdfParseOptions::default(), &mut Vec::new()).unwrap();

        assert!(bytes.starts_with(b"%PDF-"));
        assert_eq!(parsed.pages.len(), 1);
    }

    #[test]
    fn rendering_errors_include_page_and_element_paths() {
        let document = ResolvedDocument {
            title: "Renderer error".to_owned(),
            width_pt: 100.0,
            height_pt: 100.0,
            pages: vec![ResolvedPage {
                commands: vec![ResolvedCommand {
                    source_path: "pages[0].elements[3]".to_owned(),
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
}
