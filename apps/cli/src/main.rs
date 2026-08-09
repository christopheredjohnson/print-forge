use std::{
    fs::File,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use print_forge_dataset::Dataset;
use print_forge_template::Template;
use print_forge_validation::{ValidationReport, validate_job, validate_template};

mod job;

#[derive(Debug, Parser)]
#[command(name = "print-forge", version, about = "Data-driven print generation")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Parse a template and report its basic structure.
    Validate {
        template: PathBuf,
        /// Also validate template fields against every dataset row.
        #[arg(long)]
        dataset: Option<PathBuf>,
    },
    /// Parse a CSV or JSON dataset and report its row count.
    InspectData { dataset: PathBuf },
    /// Render a variable-data job to one combined PDF or one PDF per row.
    Render(job::RenderArgs),
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Validate { template, dataset } => validate_inputs(&template, dataset.as_deref()),
        Command::InspectData { dataset } => inspect_data(&dataset),
        Command::Render(arguments) => job::render(&arguments),
    }
}

fn validate_inputs(path: &Path, dataset_path: Option<&Path>) -> Result<()> {
    let template = load_template(path)?;
    let element_count: usize = template.pages.iter().map(|page| page.elements.len()).sum();
    let report = if let Some(dataset_path) = dataset_path {
        let dataset = load_dataset(dataset_path)?;
        validate_job(&template, &dataset)
    } else {
        validate_template(&template)
    };

    print_diagnostics(&report);
    require_valid(&report)?;

    println!(
        "valid template: {} ({} page(s), {} top-level element(s), {} warning(s))",
        template.name,
        template.pages.len(),
        element_count,
        report.warning_count()
    );

    Ok(())
}

pub(crate) fn load_template(path: &Path) -> Result<Template> {
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

pub(crate) fn load_dataset(path: &Path) -> Result<Dataset> {
    let dataset = match path.extension().and_then(|extension| extension.to_str()) {
        Some("csv") => Dataset::from_csv(path)?,
        Some("json") => Dataset::from_json(path)?,
        _ => anyhow::bail!("dataset must have a .csv or .json extension"),
    };

    Ok(dataset)
}

pub(crate) fn print_diagnostics(report: &ValidationReport) {
    for diagnostic in report.diagnostics() {
        eprintln!("{diagnostic}");
    }
}

pub(crate) fn require_valid(report: &ValidationReport) -> Result<()> {
    if report.is_valid() {
        return Ok(());
    }

    anyhow::bail!(
        "validation failed with {} error(s) and {} warning(s)",
        report.error_count(),
        report.warning_count()
    )
}
