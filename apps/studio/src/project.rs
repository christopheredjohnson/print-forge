use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use print_forge_template::{Element, PROJECT_MANIFEST_FILE, Template};

pub const PROJECT_MANIFEST: &str = PROJECT_MANIFEST_FILE;
pub const IMAGES_DIR: &str = "assets/images";
pub const FONTS_DIR: &str = "assets/fonts";

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
    use std::{fs, time::SystemTime};

    use print_forge_template::{Element, FontFamily};

    use super::{FONTS_DIR, IMAGES_DIR, PROJECT_MANIFEST, save_project_folder};
    use crate::model::{ElementKind, new_element, starter_template};

    #[test]
    fn project_save_copies_local_assets_and_preserves_external_sources() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("print-forge-project-test-{unique}"));
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
}
