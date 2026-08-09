use std::{
    fs::{self, File},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use print_forge_dataset::Dataset;
use print_forge_engine::{BasicLayoutEngine, LayoutEngine};
use print_forge_pdf::{DocumentRenderer, PdfRenderer};
use print_forge_template::Template;

#[derive(Debug, Parser)]
#[command(name = "print-forge", version, about = "Data-driven print generation")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Parse a template and report its basic structure.
    Validate { template: PathBuf },
    /// Parse a CSV or JSON dataset and report its row count.
    InspectData { dataset: PathBuf },
    /// Render the first dataset row to a PDF file.
    Render {
        template: PathBuf,
        dataset: PathBuf,
        output: PathBuf,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Validate { template } => validate_template(&template),
        Command::InspectData { dataset } => inspect_data(&dataset),
        Command::Render {
            template,
            dataset,
            output,
        } => render(&template, &dataset, &output),
    }
}

fn validate_template(path: &Path) -> Result<()> {
    let template = load_template(path)?;
    let element_count: usize = template.pages.iter().map(|page| page.elements.len()).sum();

    println!(
        "valid template: {} ({} page(s), {} top-level element(s))",
        template.name,
        template.pages.len(),
        element_count
    );

    Ok(())
}

fn load_template(path: &Path) -> Result<Template> {
    let file =
        File::open(path).with_context(|| format!("failed to open template {}", path.display()))?;
    serde_json::from_reader(file)
        .with_context(|| format!("failed to parse template {}", path.display()))
}

fn inspect_data(path: &Path) -> Result<()> {
    let dataset = load_dataset(path)?;

    println!("valid dataset: {} row(s)", dataset.rows.len());
    Ok(())
}

fn load_dataset(path: &Path) -> Result<Dataset> {
    let dataset = match path.extension().and_then(|extension| extension.to_str()) {
        Some("csv") => Dataset::from_csv(path)?,
        Some("json") => Dataset::from_json(path)?,
        _ => anyhow::bail!("dataset must have a .csv or .json extension"),
    };

    Ok(dataset)
}

fn render(template_path: &Path, dataset_path: &Path, output_path: &Path) -> Result<()> {
    let template = load_template(template_path)?;
    let dataset = load_dataset(dataset_path)?;
    let row = dataset
        .rows
        .first()
        .context("cannot render an empty dataset")?;
    let document = BasicLayoutEngine.layout(&template, row)?;
    let pdf = PdfRenderer.render(&document)?;

    if let Some(parent) = output_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create output directory {}", parent.display()))?;
    }

    fs::write(output_path, &pdf)
        .with_context(|| format!("failed to write PDF {}", output_path.display()))?;

    println!(
        "rendered {} page(s) to {} ({} bytes)",
        document.pages.len(),
        output_path.display(),
        pdf.len()
    );
    Ok(())
}
