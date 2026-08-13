use std::{collections::BTreeSet, path::Path};

use fontdb::{Database, Family, ID, Query, Source, Stretch, Style as DatabaseStyle, Weight};
use ttf_parser::Permissions;

use crate::project::ImportedFontStyle;

pub struct SystemFontCatalog {
    database: Database,
    families: Vec<SystemFontFamily>,
}

pub struct SystemFontFamily {
    pub name: String,
    pub faces: Vec<SystemFontFace>,
}

pub struct SystemFontFace {
    id: ID,
    pub style: ImportedFontStyle,
    pub face_index: u32,
    pub post_script_name: String,
    pub file_name: String,
    pub embedding: FontEmbedding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontEmbedding {
    Installable,
    Editable,
    PreviewAndPrint,
    Restricted,
    Unsupported,
}

impl FontEmbedding {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Installable => "installable embedding",
            Self::Editable => "editable embedding",
            Self::PreviewAndPrint => "preview/print embedding",
            Self::Restricted => "restricted by font",
            Self::Unsupported => "embedding unavailable",
        }
    }

    pub const fn allows_pdf_embedding(self) -> bool {
        matches!(
            self,
            Self::Installable | Self::Editable | Self::PreviewAndPrint
        )
    }
}

pub struct SystemFontExportFace {
    pub style: ImportedFontStyle,
    pub bytes: Vec<u8>,
    pub face_index: u32,
    pub file_name: String,
}

#[derive(Clone)]
pub struct PreviewFontFace {
    pub name: &'static str,
    pub bytes: Vec<u8>,
    pub face_index: u32,
}

impl SystemFontCatalog {
    pub fn load() -> Self {
        let mut database = Database::new();
        database.load_system_fonts();
        let names = database
            .faces()
            .filter_map(|face| face.families.first().map(|family| family.0.clone()))
            .filter(|name| !name.trim().is_empty() && !name.starts_with('.'))
            .collect::<BTreeSet<_>>();
        let families = names
            .into_iter()
            .filter_map(|name| {
                let faces = available_faces(&database, &name);
                faces
                    .iter()
                    .any(|face| face.style == ImportedFontStyle::Regular)
                    .then_some(SystemFontFamily { name, faces })
            })
            .collect();
        Self { database, families }
    }

    pub fn families(&self) -> &[SystemFontFamily] {
        &self.families
    }

    pub fn exportable_faces(&self, family_index: usize) -> Vec<SystemFontExportFace> {
        self.families
            .get(family_index)
            .into_iter()
            .flat_map(|family| &family.faces)
            .filter(|face| face.embedding.allows_pdf_embedding())
            .filter_map(|face| {
                self.database
                    .with_face_data(face.id, |bytes, face_index| SystemFontExportFace {
                        style: face.style,
                        bytes: bytes.to_vec(),
                        face_index,
                        file_name: face.file_name.clone(),
                    })
            })
            .collect()
    }

    pub fn builtin_preview_faces(&self) -> Vec<PreviewFontFace> {
        const TIMES: &[Family<'static>] = &[
            Family::Name("Times New Roman"),
            Family::Name("Liberation Serif"),
            Family::Name("Nimbus Roman"),
            Family::Serif,
        ];
        const COURIER: &[Family<'static>] = &[
            Family::Name("Courier New"),
            Family::Name("Liberation Mono"),
            Family::Name("Nimbus Mono PS"),
            Family::Monospace,
        ];
        [
            (
                "print-forge-preview-times",
                TIMES,
                ImportedFontStyle::Regular,
            ),
            (
                "print-forge-preview-times-bold",
                TIMES,
                ImportedFontStyle::Bold,
            ),
            (
                "print-forge-preview-times-italic",
                TIMES,
                ImportedFontStyle::Italic,
            ),
            (
                "print-forge-preview-times-bold-italic",
                TIMES,
                ImportedFontStyle::BoldItalic,
            ),
            (
                "print-forge-preview-courier",
                COURIER,
                ImportedFontStyle::Regular,
            ),
            (
                "print-forge-preview-courier-bold",
                COURIER,
                ImportedFontStyle::Bold,
            ),
            (
                "print-forge-preview-courier-oblique",
                COURIER,
                ImportedFontStyle::Italic,
            ),
            (
                "print-forge-preview-courier-bold-oblique",
                COURIER,
                ImportedFontStyle::BoldItalic,
            ),
        ]
        .into_iter()
        .filter_map(|(name, families, style)| {
            query_face(&self.database, families, style).and_then(|id| {
                self.database
                    .with_face_data(id, |bytes, face_index| PreviewFontFace {
                        name,
                        bytes: bytes.to_vec(),
                        face_index,
                    })
            })
        })
        .collect()
    }
}

fn available_faces(database: &Database, family_name: &str) -> Vec<SystemFontFace> {
    let families = [Family::Name(family_name)];
    [
        ImportedFontStyle::Regular,
        ImportedFontStyle::Bold,
        ImportedFontStyle::Italic,
        ImportedFontStyle::BoldItalic,
    ]
    .into_iter()
    .filter_map(|style| {
        let id = query_face(database, &families, style)?;
        let face = database.face(id)?;
        let embedding = database
            .with_face_data(id, |bytes, face_index| {
                ttf_parser::Face::parse(bytes, face_index)
                    .map(|face| embedding_status(&face))
                    .unwrap_or(FontEmbedding::Unsupported)
            })
            .unwrap_or(FontEmbedding::Unsupported);
        Some(SystemFontFace {
            id,
            style,
            face_index: face.index,
            post_script_name: face.post_script_name.clone(),
            file_name: source_file_name(&face.source, family_name),
            embedding,
        })
    })
    .collect()
}

fn query_face(
    database: &Database,
    families: &[Family<'_>],
    style: ImportedFontStyle,
) -> Option<ID> {
    let wants_bold = matches!(
        style,
        ImportedFontStyle::Bold | ImportedFontStyle::BoldItalic
    );
    let wants_italic = matches!(
        style,
        ImportedFontStyle::Italic | ImportedFontStyle::BoldItalic
    );
    let query = Query {
        families,
        weight: if wants_bold {
            Weight::BOLD
        } else {
            Weight::NORMAL
        },
        stretch: Stretch::Normal,
        style: if wants_italic {
            DatabaseStyle::Italic
        } else {
            DatabaseStyle::Normal
        },
    };
    let id = database.query(&query)?;
    let face = database.face(id)?;
    if wants_bold && face.weight < Weight::SEMIBOLD {
        return None;
    }
    if !wants_bold && face.weight >= Weight::SEMIBOLD {
        return None;
    }
    if wants_italic && face.style == DatabaseStyle::Normal {
        return None;
    }
    if !wants_italic && face.style != DatabaseStyle::Normal {
        return None;
    }
    Some(id)
}

fn embedding_status(face: &ttf_parser::Face<'_>) -> FontEmbedding {
    if face.permissions() == Some(Permissions::Restricted) {
        return FontEmbedding::Restricted;
    }
    if !face.is_outline_embedding_allowed() {
        return FontEmbedding::Unsupported;
    }
    match face.permissions() {
        Some(Permissions::Installable) => FontEmbedding::Installable,
        Some(Permissions::Editable) => FontEmbedding::Editable,
        Some(Permissions::PreviewAndPrint) => FontEmbedding::PreviewAndPrint,
        Some(Permissions::Restricted) => FontEmbedding::Restricted,
        None => FontEmbedding::Unsupported,
    }
}

fn source_file_name(source: &Source, family_name: &str) -> String {
    let path = match source {
        Source::File(path) | Source::SharedFile(path, _) => Some(path.as_path()),
        Source::Binary(_) => None,
    };
    path.and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{}.ttf", safe_file_stem(family_name)))
}

fn safe_file_stem(value: &str) -> String {
    let stem = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    let stem = stem.trim_matches('-');
    if stem.is_empty() {
        "system-font".to_owned()
    } else {
        stem.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, time::SystemTime};

    use print_forge_dataset::DataRow;
    use print_forge_engine::{BasicLayoutEngine, LayoutOptions};
    use print_forge_pdf::{PdfRenderOptions, PdfRenderer};
    use print_forge_template::{Element, FontStyle};

    use super::{FontEmbedding, SystemFontCatalog};
    use crate::{
        model::starter_template,
        project::{FontFaceExport, ImportedFontStyle, export_font_family},
    };

    #[test]
    fn installed_font_catalog_has_regular_faces_and_valid_metadata() {
        let catalog = SystemFontCatalog::load();
        if catalog.families().is_empty() {
            return;
        }
        let mut exportable_regular = false;
        for family in catalog.families() {
            assert!(!family.name.trim().is_empty());
            assert!(
                family
                    .faces
                    .iter()
                    .any(|face| face.style == crate::project::ImportedFontStyle::Regular)
            );
            for face in &family.faces {
                assert!(!face.post_script_name.is_empty());
                assert!(!face.file_name.is_empty());
                assert!(face.face_index < 10_000);
                assert!(matches!(
                    face.embedding,
                    FontEmbedding::Installable
                        | FontEmbedding::Editable
                        | FontEmbedding::PreviewAndPrint
                        | FontEmbedding::Restricted
                        | FontEmbedding::Unsupported
                ));
                exportable_regular |= face.style == crate::project::ImportedFontStyle::Regular
                    && face.embedding.allows_pdf_embedding();
            }
        }
        assert!(exportable_regular);
    }

    #[test]
    fn exported_system_font_renders_as_an_embedded_pdf_font() {
        let catalog = SystemFontCatalog::load();
        let Some(family_index) = catalog
            .families()
            .iter()
            .position(|family| {
                family.faces.iter().any(|face| {
                    face.style == ImportedFontStyle::Regular
                        && face.embedding.allows_pdf_embedding()
                }) && family
                    .faces
                    .iter()
                    .any(|face| face.face_index > 0 && face.embedding.allows_pdf_embedding())
            })
            .or_else(|| {
                catalog.families().iter().position(|family| {
                    family.faces.iter().any(|face| {
                        face.style == ImportedFontStyle::Regular
                            && face.embedding.allows_pdf_embedding()
                    })
                })
            })
        else {
            return;
        };
        let family_name = catalog.families()[family_index].name.clone();
        let system_faces = catalog.exportable_faces(family_index);
        let selected_style = system_faces
            .iter()
            .find(|face| face.face_index > 0)
            .map_or(ImportedFontStyle::Regular, |face| face.style);
        let faces = system_faces
            .into_iter()
            .map(|face| FontFaceExport {
                style: face.style,
                bytes: face.bytes,
                face_index: face.face_index,
                file_name: face.file_name,
            })
            .collect::<Vec<_>>();
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let project = std::env::temp_dir().join(format!("print-forge-system-font-{unique}"));
        let mut template = starter_template();
        export_font_family(&family_name, &faces, &project, &mut template.fonts).unwrap();
        let Element::Text(text) = &mut template.pages[0].elements[0] else {
            panic!("starter template should contain text");
        };
        text.font = Some(family_name);
        text.font_style = match selected_style {
            ImportedFontStyle::Regular => FontStyle::Regular,
            ImportedFontStyle::Bold => FontStyle::Bold,
            ImportedFontStyle::Italic => FontStyle::Italic,
            ImportedFontStyle::BoldItalic => FontStyle::BoldItalic,
        };

        let document = BasicLayoutEngine
            .layout_with_options(
                &template,
                &DataRow::default(),
                &LayoutOptions {
                    asset_base: project.clone(),
                },
            )
            .unwrap();
        let pdf = PdfRenderer
            .render_with_options(&document, &PdfRenderOptions::default())
            .unwrap();

        assert!(pdf.starts_with(b"%PDF-"));
        fs::remove_dir_all(project).unwrap();
    }
}
