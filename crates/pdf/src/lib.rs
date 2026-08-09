//! PDF rendering boundary.
//!
//! The concrete `printpdf` adapter belongs in this crate; other crates should
//! only depend on the renderer-neutral engine types.

use anyhow::{Result, bail};
use print_forge_engine::{
    DrawCommand, LineCommand, LineDash, RectangleCommand, ResolvedDocument, StrokeCommand,
    TextCommand,
};
use printpdf::{
    BuiltinFont, Color, Line, LineDashPattern, LinePoint, Mm, Op, PaintMode, PdfDocument,
    PdfFontHandle, PdfPage, PdfSaveOptions, Point, Pt, Rect, Rgb, TextItem, WindingOrder,
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
        let mut pages = Vec::with_capacity(document.pages.len());

        for page in &document.pages {
            let mut ops = Vec::new();

            for command in &page.commands {
                match command {
                    DrawCommand::Text(text) => render_text(text, &mut ops)?,
                    DrawCommand::Rectangle(rectangle) => {
                        render_rectangle(rectangle, &mut ops)?;
                    }
                    DrawCommand::Line(line) => render_line(line, &mut ops)?,
                    DrawCommand::Image(_) => {
                        bail!("image rendering is not implemented yet")
                    }
                    DrawCommand::Svg(_) => bail!("SVG rendering is not implemented yet"),
                }
            }

            pages.push(PdfPage::new(width, height, ops));
        }

        let mut pdf = PdfDocument::new(&document.title);
        pdf.with_pages(pages);

        Ok(pdf.save(&PdfSaveOptions::default(), &mut Vec::new()))
    }
}

fn render_text(command: &TextCommand, ops: &mut Vec<Op>) -> Result<()> {
    let font = builtin_font(command.font.as_deref())?;
    let color = parse_color(&command.color)?;

    ops.extend([
        Op::SaveGraphicsState,
        Op::StartTextSection,
        Op::SetTextCursor {
            pos: pdf_point(command.bounds.x, command.bounds.y),
        },
        Op::SetFont {
            font: PdfFontHandle::Builtin(font),
            size: Pt(command.font_size_pt),
        },
        Op::SetLineHeight {
            lh: Pt(command.font_size_pt * 1.2),
        },
        Op::SetFillColor { col: color },
    ]);

    for (index, line) in command.value.split('\n').enumerate() {
        if index > 0 {
            ops.push(Op::AddLineBreak);
        }
        ops.push(Op::ShowText {
            items: vec![TextItem::Text(line.to_owned())],
        });
    }

    ops.extend([Op::EndTextSection, Op::RestoreGraphicsState]);
    Ok(())
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

fn builtin_font(name: Option<&str>) -> Result<BuiltinFont> {
    let Some(name) = name else {
        return Ok(BuiltinFont::Helvetica);
    };

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
    use print_forge_engine::{
        DrawCommand, LineCommand, LineDash, Point, Rect, ResolvedDocument, ResolvedPage,
        TextCommand,
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
                    DrawCommand::Text(TextCommand {
                        bounds: Rect {
                            x: 36.0,
                            y: 96.0,
                            width: 180.0,
                            height: 18.0,
                        },
                        value: "Ada Lovelace".to_owned(),
                        font_size_pt: 16.0,
                        font: None,
                        color: "#112233".to_owned(),
                    }),
                    DrawCommand::Line(LineCommand {
                        start: Point { x: 36.0, y: 72.0 },
                        end: Point { x: 216.0, y: 72.0 },
                        width_pt: 0.5,
                        color: "#000000".to_owned(),
                        dash: LineDash::Solid,
                    }),
                ],
            }],
        };

        let bytes = PdfRenderer.render(&document).unwrap();
        let parsed =
            PdfDocument::parse(&bytes, &PdfParseOptions::default(), &mut Vec::new()).unwrap();

        assert!(bytes.starts_with(b"%PDF-"));
        assert_eq!(parsed.pages.len(), 1);
    }
}
