use std::{
    fs::File,
    path::{Path, PathBuf},
    process::ExitCode,
};

use anyhow::{Context, Result};
use clap::{ArgAction, Parser, Subcommand, ValueEnum};
use print_forge_dataset::Dataset;
use print_forge_template::Template;
use print_forge_validation::{Diagnostic, ValidationReport, validate_job, validate_template};
use serde::Serialize;

mod job;

const EXIT_INPUT: u8 = 3;
const EXIT_JOB: u8 = 4;

#[derive(Debug, Parser)]
#[command(name = "print-forge", version, about = "Data-driven print generation")]
struct Cli {
    /// Emit one machine-readable JSON document on stdout.
    #[arg(long, global = true)]
    json: bool,
    /// Include operational details in text diagnostics; repeat for more detail.
    #[arg(short, long, global = true, action = ArgAction::Count)]
    verbose: u8,
    /// Decide whether validation warnings are accepted, rejected, or hidden.
    #[arg(long, global = true, value_enum, default_value_t = WarningPolicy::Allow)]
    warnings: WarningPolicy,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum WarningPolicy {
    Allow,
    Deny,
    Ignore,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct OutputOptions {
    pub(crate) json: bool,
    pub(crate) verbose: u8,
    pub(crate) warnings: WarningPolicy,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Parse and semantically validate a template and optional dataset.
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

#[derive(Debug)]
struct AppError {
    exit_code: u8,
    source: anyhow::Error,
}

impl AppError {
    fn input(error: impl Into<anyhow::Error>) -> Self {
        Self {
            exit_code: EXIT_INPUT,
            source: error.into(),
        }
    }

    fn job(error: impl Into<anyhow::Error>) -> Self {
        Self {
            exit_code: EXIT_JOB,
            source: error.into(),
        }
    }
}

#[derive(Serialize)]
struct ErrorOutput {
    status: &'static str,
    exit_code: u8,
    error: String,
}

#[derive(Serialize)]
struct ValidationOutput<'a> {
    status: &'static str,
    template: &'a str,
    pages: usize,
    elements: usize,
    warnings: usize,
    diagnostics: Vec<&'a Diagnostic>,
}

#[derive(Serialize)]
struct DatasetOutput {
    status: &'static str,
    rows: usize,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let output = OutputOptions {
        json: cli.json,
        verbose: cli.verbose,
        warnings: cli.warnings,
    };
    let result = match cli.command {
        Command::Validate { template, dataset } => {
            validate_inputs(&template, dataset.as_deref(), output).map_err(AppError::input)
        }
        Command::InspectData { dataset } => inspect_data(&dataset, output).map_err(AppError::input),
        Command::Render(arguments) => job::render(&arguments, output).map_err(AppError::job),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if output.json {
                let value = ErrorOutput {
                    status: "error",
                    exit_code: error.exit_code,
                    error: format!("{:#}", error.source),
                };
                if serde_json::to_writer(std::io::stdout(), &value).is_ok() {
                    println!();
                }
            } else {
                eprintln!("error: {:#}", error.source);
                if output.verbose > 0 {
                    eprintln!("exit code: {}", error.exit_code);
                }
            }
            ExitCode::from(error.exit_code)
        }
    }
}

fn validate_inputs(path: &Path, dataset_path: Option<&Path>, output: OutputOptions) -> Result<()> {
    if output.verbose > 0 && !output.json {
        eprintln!("validating template {}", path.display());
        if let Some(dataset_path) = dataset_path {
            eprintln!("validating dataset {}", dataset_path.display());
        }
    }
    let template = load_template(path)?;
    let element_count: usize = template.pages.iter().map(|page| page.elements.len()).sum();
    let report = if let Some(dataset_path) = dataset_path {
        let dataset = load_dataset(dataset_path)?;
        validate_job(&template, &dataset)
    } else {
        validate_template(&template)
    };

    print_diagnostics(&report, output);
    require_acceptable(&report, output.warnings)?;

    if output.json {
        write_json(&ValidationOutput {
            status: "valid",
            template: &template.name,
            pages: template.pages.len(),
            elements: element_count,
            warnings: report.warning_count(),
            diagnostics: visible_diagnostics(&report, output.warnings),
        })?;
    } else {
        println!(
            "valid template: {} ({} page(s), {} top-level element(s), {} warning(s))",
            template.name,
            template.pages.len(),
            element_count,
            report.warning_count()
        );
    }
    Ok(())
}

pub(crate) fn load_template(path: &Path) -> Result<Template> {
    let file =
        File::open(path).with_context(|| format!("failed to open template {}", path.display()))?;
    serde_json::from_reader(file)
        .with_context(|| format!("failed to parse template {}", path.display()))
}

fn inspect_data(path: &Path, output: OutputOptions) -> Result<()> {
    if output.verbose > 0 && !output.json {
        eprintln!("inspecting dataset {}", path.display());
    }
    let dataset = load_dataset(path)?;
    if output.json {
        write_json(&DatasetOutput {
            status: "valid",
            rows: dataset.rows.len(),
        })?;
    } else {
        println!("valid dataset: {} row(s)", dataset.rows.len());
    }
    Ok(())
}

pub(crate) fn load_dataset(path: &Path) -> Result<Dataset> {
    Dataset::from_path(path).map_err(Into::into)
}

pub(crate) fn visible_diagnostics(
    report: &ValidationReport,
    policy: WarningPolicy,
) -> Vec<&Diagnostic> {
    report
        .diagnostics()
        .iter()
        .filter(|diagnostic| {
            policy != WarningPolicy::Ignore
                || diagnostic.severity == print_forge_validation::Severity::Error
        })
        .collect()
}

pub(crate) fn print_diagnostics(report: &ValidationReport, output: OutputOptions) {
    if output.json {
        return;
    }
    for diagnostic in visible_diagnostics(report, output.warnings) {
        eprintln!("{diagnostic}");
    }
}

pub(crate) fn require_acceptable(report: &ValidationReport, policy: WarningPolicy) -> Result<()> {
    if report.is_valid() && !(policy == WarningPolicy::Deny && report.warning_count() > 0) {
        return Ok(());
    }

    anyhow::bail!(
        "validation failed with {} error(s) and {} warning(s){}",
        report.error_count(),
        report.warning_count(),
        if policy == WarningPolicy::Deny {
            " (--warnings deny)"
        } else {
            ""
        }
    )
}

pub(crate) fn write_json(value: &impl Serialize) -> Result<()> {
    serde_json::to_writer(std::io::stdout(), value).context("failed to serialize JSON output")?;
    println!();
    Ok(())
}
