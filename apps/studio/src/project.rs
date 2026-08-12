use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use print_forge_template::{Element, FontFamily, PROJECT_MANIFEST_FILE, Template};

pub const PROJECT_MANIFEST: &str = PROJECT_MANIFEST_FILE;
pub const IMAGES_DIR: &str = "assets/images";
pub const FONTS_DIR: &str = "assets/fonts";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedAssetKind {
    RasterImage,
    Svg,
    Font,
}

impl ManagedAssetKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::RasterImage => "Image",
            Self::Svg => "SVG",
            Self::Font => "Font",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedAsset {
    pub kind: ManagedAssetKind,
    pub relative_path: String,
    pub absolute_path: PathBuf,
    pub file_name: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedFont {
    pub family: String,
    pub style: ImportedFontStyle,
    pub relative_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportedFontStyle {
    Regular,
    Bold,
    Italic,
    BoldItalic,
}

pub struct ProjectSave {
    pub template: Template,
    pub manifest: PathBuf,
    pub copied_assets: usize,
    pub missing_assets: Vec<String>,
}

pub fn save_project_folder(
    template: &Template,
    source_base: &Path,
    project_root: &Path,
) -> Result<ProjectSave, String> {
    fs::create_dir_all(project_root.join(IMAGES_DIR)).map_err(|error| {
        format!(
            "could not create {}: {error}",
            project_root.join(IMAGES_DIR).display()
        )
    })?;
    fs::create_dir_all(project_root.join(FONTS_DIR)).map_err(|error| {
        format!(
            "could not create {}: {error}",
            project_root.join(FONTS_DIR).display()
        )
    })?;

    let mut template = template.clone();
    let mut importer = AssetImporter::new(source_base, project_root);
    for font in &mut template.fonts {
        importer.import(&mut font.regular, AssetDirectory::Fonts)?;
        for source in [&mut font.bold, &mut font.italic, &mut font.bold_italic]
            .into_iter()
            .flatten()
        {
            importer.import(source, AssetDirectory::Fonts)?;
        }
    }
    for page in &mut template.pages {
        for element in page
            .header
            .iter_mut()
            .chain(page.elements.iter_mut())
            .chain(page.footer.iter_mut())
        {
            importer.import_element(element)?;
        }
    }

    let manifest = project_root.join(PROJECT_MANIFEST);
    let mut json = serde_json::to_string_pretty(&template).map_err(|error| error.to_string())?;
    json.push('\n');
    fs::write(&manifest, json)
        .map_err(|error| format!("could not write {}: {error}", manifest.display()))?;

    Ok(ProjectSave {
        template,
        manifest,
        copied_assets: importer.copied_assets,
        missing_assets: importer.missing_assets,
    })
}

pub fn list_managed_assets(project_root: &Path) -> Result<Vec<ManagedAsset>, String> {
    let mut assets = Vec::new();
    collect_assets(
        project_root,
        IMAGES_DIR,
        |extension| match extension {
            "svg" => Some(ManagedAssetKind::Svg),
            "png" | "jpg" | "jpeg" => Some(ManagedAssetKind::RasterImage),
            _ => None,
        },
        &mut assets,
    )?;
    collect_assets(
        project_root,
        FONTS_DIR,
        |extension| match extension {
            "ttf" | "otf" => Some(ManagedAssetKind::Font),
            _ => None,
        },
        &mut assets,
    )?;
    assets.sort_by(|left, right| {
        left.kind.label().cmp(right.kind.label()).then_with(|| {
            left.file_name
                .to_lowercase()
                .cmp(&right.file_name.to_lowercase())
        })
    });
    Ok(assets)
}

pub fn import_visual_asset(source: &Path, project_root: &Path) -> Result<ManagedAsset, String> {
    let extension = extension(source);
    let kind = match extension.as_str() {
        "svg" => ManagedAssetKind::Svg,
        "png" | "jpg" | "jpeg" => ManagedAssetKind::RasterImage,
        _ => {
            return Err(format!(
                "unsupported visual asset {}; choose PNG, JPEG, or SVG",
                source.display()
            ));
        }
    };
    let target = copy_managed_file(source, project_root, AssetDirectory::Images)?;
    managed_asset(project_root, target, kind)
}

pub fn import_font_assets(
    sources: &[PathBuf],
    project_root: &Path,
    families: &mut Vec<FontFamily>,
) -> Result<Vec<ImportedFont>, String> {
    let mut pending = sources
        .iter()
        .map(|source| {
            let extension = extension(source);
            if !matches!(extension.as_str(), "ttf" | "otf") {
                return Err(format!(
                    "unsupported font {}; choose a TTF or OTF file",
                    source.display()
                ));
            }
            let (family, style) = font_identity(source)?;
            Ok((source, family, style))
        })
        .collect::<Result<Vec<_>, String>>()?;
    pending.sort_by_key(|(_, _, style)| match style {
        ImportedFontStyle::Regular => 0,
        ImportedFontStyle::Bold => 1,
        ImportedFontStyle::Italic => 2,
        ImportedFontStyle::BoldItalic => 3,
    });
    let mut imported = Vec::new();
    for (source, family, style) in pending {
        if style != ImportedFontStyle::Regular
            && !families.iter().any(|candidate| candidate.name == family)
        {
            return Err(format!(
                "{} is a {} face; import the regular {family} face first",
                source.display(),
                font_style_label(style)
            ));
        }
        let target = copy_managed_file(source, project_root, AssetDirectory::Fonts)?;
        let asset = managed_asset(project_root, target, ManagedAssetKind::Font)?;
        let existing = families
            .iter_mut()
            .find(|candidate| candidate.name == family);
        if let Some(existing) = existing {
            set_font_variant(existing, style, asset.relative_path.clone());
        } else if style == ImportedFontStyle::Regular {
            families.push(FontFamily {
                name: family.clone(),
                regular: asset.relative_path.clone(),
                bold: None,
                italic: None,
                bold_italic: None,
            });
        }
        imported.push(ImportedFont {
            family,
            style,
            relative_path: asset.relative_path,
        });
    }
    Ok(imported)
}

pub fn missing_local_assets(template: &Template, project_root: &Path) -> Vec<String> {
    let mut sources = Vec::new();
    for family in &template.fonts {
        sources.push(family.regular.as_str());
        sources.extend(
            [
                family.bold.as_deref(),
                family.italic.as_deref(),
                family.bold_italic.as_deref(),
            ]
            .into_iter()
            .flatten(),
        );
    }
    for page in &template.pages {
        for element in page
            .header
            .iter()
            .chain(page.elements.iter())
            .chain(page.footer.iter())
        {
            collect_element_sources(element, &mut sources);
        }
    }
    let mut missing = sources
        .into_iter()
        .filter(|source| !is_external_source(source))
        .filter(|source| !project_root.join(source).is_file())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    missing.sort();
    missing.dedup();
    missing
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum AssetDirectory {
    Images,
    Fonts,
}

impl AssetDirectory {
    const fn relative(self) -> &'static str {
        match self {
            Self::Images => IMAGES_DIR,
            Self::Fonts => FONTS_DIR,
        }
    }
}

fn collect_assets(
    project_root: &Path,
    relative_directory: &str,
    classify: impl Fn(&str) -> Option<ManagedAssetKind>,
    assets: &mut Vec<ManagedAsset>,
) -> Result<(), String> {
    let directory = project_root.join(relative_directory);
    if !directory.exists() {
        return Ok(());
    }
    let entries = fs::read_dir(&directory)
        .map_err(|error| format!("could not read {}: {error}", directory.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let extension = extension(&path);
        let Some(kind) = classify(&extension) else {
            continue;
        };
        assets.push(managed_asset(project_root, path, kind)?);
    }
    Ok(())
}

fn managed_asset(
    project_root: &Path,
    absolute_path: PathBuf,
    kind: ManagedAssetKind,
) -> Result<ManagedAsset, String> {
    let relative_path = absolute_path
        .strip_prefix(project_root)
        .map_err(|_| {
            format!(
                "asset is outside project folder: {}",
                absolute_path.display()
            )
        })?
        .to_string_lossy()
        .replace('\\', "/");
    let file_name = absolute_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("asset has no usable filename: {}", absolute_path.display()))?
        .to_owned();
    let bytes = fs::metadata(&absolute_path)
        .map_err(|error| error.to_string())?
        .len();
    Ok(ManagedAsset {
        kind,
        relative_path,
        absolute_path,
        file_name,
        bytes,
    })
}

fn copy_managed_file(
    source: &Path,
    project_root: &Path,
    directory: AssetDirectory,
) -> Result<PathBuf, String> {
    if !source.is_file() {
        return Err(format!("asset does not exist: {}", source.display()));
    }
    let target_directory = project_root.join(directory.relative());
    fs::create_dir_all(&target_directory)
        .map_err(|error| format!("could not create {}: {error}", target_directory.display()))?;
    let file_name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("asset has no usable filename: {}", source.display()))?;
    let target = available_target(source, &target_directory, file_name)?;
    if !same_file(source, &target) {
        fs::copy(source, &target).map_err(|error| {
            format!(
                "could not copy {} to {}: {error}",
                source.display(),
                target.display()
            )
        })?;
    }
    Ok(target)
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn font_identity(path: &Path) -> Result<(String, ImportedFontStyle), String> {
    let bytes =
        fs::read(path).map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let face = ttf_parser::Face::parse(&bytes, 0)
        .map_err(|error| format!("could not parse font {}: {error:?}", path.display()))?;
    let mut family = face
        .names()
        .into_iter()
        .find(|name| name.name_id == ttf_parser::name_id::TYPOGRAPHIC_FAMILY)
        .and_then(|name| name.to_string())
        .or_else(|| {
            face.names()
                .into_iter()
                .find(|name| name.name_id == ttf_parser::name_id::FAMILY)
                .and_then(|name| name.to_string())
        })
        .or_else(|| {
            path.file_stem()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
        })
        .ok_or_else(|| format!("font has no family name: {}", path.display()))?;
    let subfamily = face
        .names()
        .into_iter()
        .find(|name| name.name_id == ttf_parser::name_id::TYPOGRAPHIC_SUBFAMILY)
        .and_then(|name| name.to_string())
        .or_else(|| {
            face.names()
                .into_iter()
                .find(|name| name.name_id == ttf_parser::name_id::SUBFAMILY)
                .and_then(|name| name.to_string())
        })
        .unwrap_or_default()
        .to_ascii_lowercase();
    let file_stem = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let bold = face.is_bold() || subfamily.contains("bold") || file_stem.contains("bold");
    let italic = face.is_italic()
        || subfamily.contains("italic")
        || subfamily.contains("oblique")
        || file_stem.contains("italic")
        || file_stem.contains("oblique");
    let style = match (bold, italic) {
        (false, false) => ImportedFontStyle::Regular,
        (true, false) => ImportedFontStyle::Bold,
        (false, true) => ImportedFontStyle::Italic,
        (true, true) => ImportedFontStyle::BoldItalic,
    };
    normalize_font_family(&mut family, style);
    Ok((family, style))
}

fn normalize_font_family(family: &mut String, style: ImportedFontStyle) {
    let suffixes: &[&str] = match style {
        ImportedFontStyle::Regular => &[" regular", "-regular", "_regular"],
        ImportedFontStyle::Bold => &[" bold", "-bold", "_bold"],
        ImportedFontStyle::Italic => &[
            " italic", "-italic", "_italic", " oblique", "-oblique", "_oblique",
        ],
        ImportedFontStyle::BoldItalic => &[
            " bold italic",
            "-bold-italic",
            "_bold_italic",
            " bold oblique",
            "-bold-oblique",
            "_bold_oblique",
            "-bolditalic",
            "_bolditalic",
            "-boldoblique",
            "_boldoblique",
        ],
    };
    let lowercase = family.to_ascii_lowercase();
    if let Some(suffix) = suffixes.iter().find(|suffix| lowercase.ends_with(**suffix)) {
        family.truncate(family.len() - suffix.len());
        *family = family.trim_end_matches([' ', '-', '_']).to_owned();
    }
}

fn set_font_variant(family: &mut FontFamily, style: ImportedFontStyle, source: String) {
    match style {
        ImportedFontStyle::Regular => family.regular = source,
        ImportedFontStyle::Bold => family.bold = Some(source),
        ImportedFontStyle::Italic => family.italic = Some(source),
        ImportedFontStyle::BoldItalic => family.bold_italic = Some(source),
    }
}

pub const fn font_style_label(style: ImportedFontStyle) -> &'static str {
    match style {
        ImportedFontStyle::Regular => "regular",
        ImportedFontStyle::Bold => "bold",
        ImportedFontStyle::Italic => "italic",
        ImportedFontStyle::BoldItalic => "bold italic",
    }
}

fn collect_element_sources<'a>(element: &'a Element, sources: &mut Vec<&'a str>) {
    match element {
        Element::Image(image) => sources.push(&image.source),
        Element::Svg(svg) => sources.push(&svg.source),
        Element::Group(group) => {
            for child in &group.children {
                collect_element_sources(child, sources);
            }
        }
        Element::Stack(stack) => {
            for child in &stack.children {
                collect_element_sources(child, sources);
            }
        }
        Element::Repeater(repeater) => collect_element_sources(&repeater.template, sources),
        Element::Text(_)
        | Element::Rectangle(_)
        | Element::Line(_)
        | Element::QrCode(_)
        | Element::Barcode(_)
        | Element::Table(_)
        | Element::PageBreak => {}
    }
}

struct AssetImporter<'a> {
    source_base: &'a Path,
    project_root: &'a Path,
    imported: HashMap<(PathBuf, AssetDirectory), String>,
    copied_assets: usize,
    missing_assets: Vec<String>,
}

impl<'a> AssetImporter<'a> {
    fn new(source_base: &'a Path, project_root: &'a Path) -> Self {
        Self {
            source_base,
            project_root,
            imported: HashMap::new(),
            copied_assets: 0,
            missing_assets: Vec::new(),
        }
    }

    fn import_element(&mut self, element: &mut Element) -> Result<(), String> {
        match element {
            Element::Image(image) => self.import(&mut image.source, AssetDirectory::Images)?,
            Element::Svg(svg) => self.import(&mut svg.source, AssetDirectory::Images)?,
            Element::Group(group) => {
                for child in &mut group.children {
                    self.import_element(child)?;
                }
            }
            Element::Stack(stack) => {
                for child in &mut stack.children {
                    self.import_element(child)?;
                }
            }
            Element::Repeater(repeater) => self.import_element(&mut repeater.template)?,
            Element::Text(_)
            | Element::Rectangle(_)
            | Element::Line(_)
            | Element::QrCode(_)
            | Element::Barcode(_)
            | Element::Table(_)
            | Element::PageBreak => {}
        }
        Ok(())
    }

    fn import(&mut self, source: &mut String, directory: AssetDirectory) -> Result<(), String> {
        if is_external_source(source) {
            return Ok(());
        }
        let source_path = Path::new(source);
        let resolved = if source_path.is_absolute() {
            source_path.to_owned()
        } else {
            self.source_base.join(source_path)
        };
        if !resolved.is_file() {
            if !self.missing_assets.contains(source) {
                self.missing_assets.push(source.clone());
            }
            return Ok(());
        }

        let key = resolved.canonicalize().unwrap_or_else(|_| resolved.clone());
        if let Some(relative) = self.imported.get(&(key.clone(), directory)) {
            *source = relative.clone();
            return Ok(());
        }

        let file_name = resolved
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .ok_or_else(|| format!("asset path has no usable filename: {}", resolved.display()))?;
        let target_directory = self.project_root.join(directory.relative());
        let target = available_target(&resolved, &target_directory, file_name)?;
        if !same_file(&resolved, &target) {
            fs::copy(&resolved, &target).map_err(|error| {
                format!(
                    "could not copy {} to {}: {error}",
                    resolved.display(),
                    target.display()
                )
            })?;
            self.copied_assets += 1;
        }
        let target_name = target
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("asset target has no usable filename: {}", target.display()))?;
        let relative = format!("{}/{target_name}", directory.relative());
        self.imported.insert((key, directory), relative.clone());
        *source = relative;
        Ok(())
    }
}

fn is_external_source(source: &str) -> bool {
    let source = source.trim();
    source.is_empty()
        || source.contains("{{")
        || source.contains("://")
        || source.starts_with("data:")
}

fn available_target(source: &Path, directory: &Path, file_name: &str) -> Result<PathBuf, String> {
    let direct = directory.join(file_name);
    if !direct.exists() || same_contents(source, &direct)? {
        return Ok(direct);
    }
    let file_name = Path::new(file_name);
    let stem = file_name
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("asset");
    let extension = file_name.extension().and_then(|value| value.to_str());
    for suffix in 2..=10_000 {
        let candidate = extension.map_or_else(
            || format!("{stem}-{suffix}"),
            |extension| format!("{stem}-{suffix}.{extension}"),
        );
        let candidate = directory.join(candidate);
        if !candidate.exists() || same_contents(source, &candidate)? {
            return Ok(candidate);
        }
    }
    Err(format!(
        "could not find an available asset filename for {}",
        source.display()
    ))
}

fn same_contents(left: &Path, right: &Path) -> Result<bool, String> {
    let left_metadata = fs::metadata(left).map_err(|error| error.to_string())?;
    let right_metadata = fs::metadata(right).map_err(|error| error.to_string())?;
    if left_metadata.len() != right_metadata.len() {
        return Ok(false);
    }
    let left = fs::read(left).map_err(|error| error.to_string())?;
    let right = fs::read(right).map_err(|error| error.to_string())?;
    Ok(left == right)
}

fn same_file(left: &Path, right: &Path) -> bool {
    left.canonicalize().ok() == right.canonicalize().ok()
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf, time::SystemTime};

    use print_forge_template::{Element, FontFamily};

    use super::{
        FONTS_DIR, IMAGES_DIR, ImportedFontStyle, ManagedAssetKind, PROJECT_MANIFEST,
        import_font_assets, import_visual_asset, list_managed_assets, missing_local_assets,
        save_project_folder,
    };
    use crate::model::{ElementKind, new_element, starter_template};

    fn temporary_root(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("print-forge-{label}-{unique}"))
    }

    #[test]
    fn project_save_copies_local_assets_and_preserves_external_sources() {
        let root = temporary_root("project-save-test");
        let source = root.join("source");
        let project = root.join("project");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("mark.svg"), "<svg/>").unwrap();
        fs::write(source.join("body.ttf"), "font").unwrap();

        let mut template = starter_template();
        let mut svg = new_element(ElementKind::Svg, 0.0);
        let Element::Svg(element) = &mut svg else {
            unreachable!();
        };
        element.source = "mark.svg".to_owned();
        let mut image = new_element(ElementKind::Image, 0.0);
        let Element::Image(element) = &mut image else {
            unreachable!();
        };
        element.source = "https://example.com/photo.png".to_owned();
        let mut missing = new_element(ElementKind::Image, 0.0);
        let Element::Image(element) = &mut missing else {
            unreachable!();
        };
        element.source = "missing.png".to_owned();
        template.pages[0].elements.extend([svg, image, missing]);
        template.fonts.push(FontFamily {
            name: "Body".to_owned(),
            regular: "body.ttf".to_owned(),
            bold: None,
            italic: None,
            bold_italic: None,
        });

        let saved = save_project_folder(&template, &source, &project).unwrap();
        assert!(saved.manifest.ends_with(PROJECT_MANIFEST));
        assert!(project.join(IMAGES_DIR).join("mark.svg").is_file());
        assert!(project.join(FONTS_DIR).join("body.ttf").is_file());
        assert_eq!(saved.copied_assets, 2);
        let Element::Svg(svg) = &saved.template.pages[0].elements[1] else {
            panic!("expected SVG");
        };
        assert_eq!(svg.source, "assets/images/mark.svg");
        let Element::Image(image) = &saved.template.pages[0].elements[2] else {
            panic!("expected image");
        };
        assert_eq!(image.source, "https://example.com/photo.png");
        assert_eq!(saved.template.fonts[0].regular, "assets/fonts/body.ttf");
        assert_eq!(saved.missing_assets, ["missing.png"]);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn managed_visual_assets_are_copied_listed_and_collision_safe() {
        let root = temporary_root("managed-assets-test");
        let first_source = root.join("first");
        let second_source = root.join("second");
        let project = root.join("project");
        fs::create_dir_all(&first_source).unwrap();
        fs::create_dir_all(&second_source).unwrap();
        fs::write(first_source.join("mark.svg"), "<svg id=\"first\"/>").unwrap();
        fs::write(second_source.join("mark.svg"), "<svg id=\"second\"/>").unwrap();
        fs::write(first_source.join("photo.PNG"), [137, 80, 78, 71]).unwrap();

        let first = import_visual_asset(&first_source.join("mark.svg"), &project).unwrap();
        let second = import_visual_asset(&second_source.join("mark.svg"), &project).unwrap();
        let image = import_visual_asset(&first_source.join("photo.PNG"), &project).unwrap();

        assert_eq!(first.relative_path, "assets/images/mark.svg");
        assert_eq!(second.relative_path, "assets/images/mark-2.svg");
        assert_eq!(image.kind, ManagedAssetKind::RasterImage);
        let assets = list_managed_assets(&project).unwrap();
        assert_eq!(assets.len(), 3);
        assert_eq!(
            assets
                .iter()
                .filter(|asset| asset.kind == ManagedAssetKind::Svg)
                .count(),
            2
        );
        assert!(assets.iter().all(|asset| asset.absolute_path.is_file()));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_assets_ignore_external_and_variable_sources() {
        let root = temporary_root("missing-assets-test");
        let project = root.join("project");
        fs::create_dir_all(project.join(IMAGES_DIR)).unwrap();
        fs::write(project.join(IMAGES_DIR).join("present.svg"), "<svg/>").unwrap();
        let mut template = starter_template();
        for source in [
            "assets/images/present.svg",
            "assets/images/missing.svg",
            "https://example.com/remote.png",
            "{{dynamic_asset}}",
        ] {
            let mut svg = new_element(ElementKind::Svg, 0.0);
            let Element::Svg(element) = &mut svg else {
                unreachable!();
            };
            element.source = source.to_owned();
            template.pages[0].elements.push(svg);
        }

        assert_eq!(
            missing_local_assets(&template, &project),
            ["assets/images/missing.svg"]
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn font_import_uses_embedded_family_and_style_metadata() {
        let root = temporary_root("font-import-test");
        let project = root.join("project");
        let font_root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../crates/pdf/assets/fonts");
        let sources = [
            font_root.join("Helvetica-Bold.ttf"),
            font_root.join("Helvetica.ttf"),
        ];
        let mut families = Vec::new();

        let imported = import_font_assets(&sources, &project, &mut families).unwrap();

        assert_eq!(imported.len(), 2);
        assert_eq!(imported[0].style, ImportedFontStyle::Regular);
        assert_eq!(imported[1].style, ImportedFontStyle::Bold);
        assert_eq!(families.len(), 1);
        assert_eq!(families[0].name, "Helvetica");
        assert_eq!(families[0].regular, "assets/fonts/Helvetica.ttf");
        assert_eq!(
            families[0].bold.as_deref(),
            Some("assets/fonts/Helvetica-Bold.ttf")
        );
        assert!(project.join(&families[0].regular).is_file());

        fs::remove_dir_all(root).unwrap();
    }
}
