use std::{
    collections::HashSet,
    fs,
    ops::ControlFlow,
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use print_forge_application::{
    AssetSource, BoxError, DatasetSource, DiagnosticPhase, JobSummary, OutputArtifact, OutputMode,
    OutputWriter, PdfRenderOptions, PdfXStandard, ProgressEvent, RenderOptions, RenderRequest,
    RowRange as ApplicationRowRange, RowSource, WarningPolicy as ApplicationWarningPolicy,
    render_with_progress,
};
use print_forge_dataset::{DataRow, visit_path};

use crate::{OutputOptions, WarningPolicy, load_template, write_json};

#[derive(Debug, Args)]
pub(crate) struct RenderArgs {
    pub(crate) template: PathBuf,
    pub(crate) dataset: PathBuf,
    /// PDF path in combined mode; output directory in separate mode.
    pub(crate) output: PathBuf,
    /// Write one combined PDF or one PDF for each selected row.
    #[arg(long, value_enum, default_value_t = CliOutputMode::Combined)]
    pub(crate) output_mode: CliOutputMode,
    /// One-based inclusive dataset row range, for example 2-5.
    #[arg(long, value_name = "START-END")]
    pub(crate) rows: Option<RowRange>,
    /// Render at most this many rows after applying --rows.
    #[arg(long, value_name = "COUNT")]
    pub(crate) limit: Option<usize>,
    /// Continue processing later rows after a row fails.
    #[arg(long)]
    pub(crate) continue_on_error: bool,
    /// Filename pattern for separate mode. Supports dotted fields and {{row}}.
    #[arg(long, default_value = "row-{{row}}", value_name = "PATTERN")]
    pub(crate) output_name: String,
    /// Write a machine-readable JSON job summary to this path.
    #[arg(long, value_name = "PATH")]
    pub(crate) summary: Option<PathBuf>,
    /// Enable PDF/X-4, 300 DPI images, and mandatory embedded fonts.
    #[arg(long)]
    pub(crate) print_ready: bool,
    /// Reject images below this effective output resolution.
    #[arg(long, value_name = "DPI")]
    pub(crate) min_image_dpi: Option<f32>,
    /// Reject built-in PDF fonts instead of accepting unembedded text fonts.
    #[arg(long)]
    pub(crate) require_embedded_fonts: bool,
    /// Generate and validate against a PDF/X conformance target.
    #[arg(long, value_enum, value_name = "TARGET")]
    pub(crate) pdf_x: Option<PdfXTarget>,
    /// Validate, lay out, and preflight every selected row without writing files.
    #[arg(long)]
    pub(crate) dry_run: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum CliOutputMode {
    Combined,
    Separate,
}

impl From<CliOutputMode> for OutputMode {
    fn from(value: CliOutputMode) -> Self {
        match value {
            CliOutputMode::Combined => Self::Combined,
            CliOutputMode::Separate => Self::Separate,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum PdfXTarget {
    X4,
}

impl From<PdfXTarget> for PdfXStandard {
    fn from(value: PdfXTarget) -> Self {
        match value {
            PdfXTarget::X4 => Self::X4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RowRange(ApplicationRowRange);

impl FromStr for RowRange {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (start, end) = value
            .split_once('-')
            .ok_or_else(|| "row range must use START-END, for example 2-5".to_owned())?;
        let start = start
            .parse::<usize>()
            .map_err(|_| "row range start must be a positive integer".to_owned())?;
        let end = end
            .parse::<usize>()
            .map_err(|_| "row range end must be a positive integer".to_owned())?;
        ApplicationRowRange::new(start, end)
            .map(Self)
            .map_err(|error| error.to_string())
    }
}

pub(crate) fn render(arguments: &RenderArgs, output_options: OutputOptions) -> Result<()> {
    let template = load_template(&arguments.template)?;
    let mut dataset = PathRowSource(&arguments.dataset);
    let mut writer = FileOutputWriter::new(
        arguments.output_mode.into(),
        &arguments.output,
        arguments.summary.as_deref(),
    );
    let request = RenderRequest {
        template: &template,
        dataset: DatasetSource::Streaming(&mut dataset),
        assets: AssetSource::Directory(
            arguments
                .template
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .to_owned(),
        ),
        options: RenderOptions {
            output_mode: arguments.output_mode.into(),
            rows: arguments.rows.map(|range| range.0),
            limit: arguments.limit,
            continue_on_error: arguments.continue_on_error,
            output_name: arguments.output_name.clone(),
            pdf: pdf_options(arguments),
            dry_run: arguments.dry_run,
            warnings: match output_options.warnings {
                WarningPolicy::Allow => ApplicationWarningPolicy::Allow,
                WarningPolicy::Deny => ApplicationWarningPolicy::Deny,
                WarningPolicy::Ignore => ApplicationWarningPolicy::Ignore,
            },
        },
    };

    let outcome = render_with_progress(request, &mut writer, |event| {
        report_progress(&event, arguments, output_options);
    })?;

    if !arguments.dry_run
        && let Some(path) = &arguments.summary
    {
        write_summary(path, &outcome.summary)?;
    }
    if outcome.summary.failures > 0 {
        bail!(
            "rendering completed with {} failed dataset row(s)",
            outcome.summary.failures
        );
    }
    if outcome.stopped_early {
        bail!("rendering stopped before producing output");
    }

    if output_options.json {
        write_json(&outcome.summary)?;
    } else {
        println!(
            "{} summary: {} success(es), {} warning(s), {} failure(s), {} ms",
            if arguments.dry_run { "dry-run" } else { "job" },
            outcome.summary.successes,
            outcome.summary.warnings,
            outcome.summary.failures,
            outcome.summary.elapsed_ms
        );
    }
    Ok(())
}

struct PathRowSource<'a>(&'a Path);

impl RowSource for PathRowSource<'_> {
    fn row_count(&mut self) -> std::result::Result<usize, BoxError> {
        visit_path(self.0, |_, _| ControlFlow::Continue(()))
            .map(|outcome| outcome.rows())
            .map_err(|error| Box::new(error) as BoxError)
    }

    fn visit(
        &mut self,
        visitor: &mut dyn FnMut(usize, DataRow) -> ControlFlow<()>,
    ) -> std::result::Result<(), BoxError> {
        visit_path(self.0, visitor)
            .map(|_| ())
            .map_err(|error| Box::new(error) as BoxError)
    }
}

fn pdf_options(arguments: &RenderArgs) -> PdfRenderOptions {
    let mut options = if arguments.print_ready {
        PdfRenderOptions::print_ready()
    } else {
        PdfRenderOptions::default()
    };
    if let Some(minimum) = arguments.min_image_dpi {
        options.min_image_dpi = Some(minimum);
    }
    options.require_embedded_fonts |= arguments.require_embedded_fonts;
    if let Some(target) = arguments.pdf_x {
        options.pdf_x = Some(target.into());
    }
    options
}

fn report_progress(event: &ProgressEvent, arguments: &RenderArgs, output: OutputOptions) {
    if output.json {
        return;
    }
    match event {
        ProgressEvent::Started {
            selected_rows,
            dry_run,
        } if output.verbose > 0 => eprintln!(
            "processing {selected_rows} selected row(s) from {}{}",
            arguments.dataset.display(),
            if *dry_run { " (dry run)" } else { "" }
        ),
        ProgressEvent::Diagnostic(diagnostic)
            if diagnostic.phase == DiagnosticPhase::Validation =>
        {
            eprintln!("{diagnostic}");
        }
        ProgressEvent::RowStarted { row_index } if output.verbose > 1 => {
            eprintln!("processing dataset row {row_index}");
        }
        ProgressEvent::RowSucceeded {
            row_index,
            output,
            pages,
            bytes,
            dry_run,
        } => println!(
            "{} dataset row {row_index} to {output} ({pages} page(s), {bytes} bytes)",
            if *dry_run { "checked" } else { "rendered" }
        ),
        ProgressEvent::RowFailed { row_index, error } => {
            eprintln!("dataset row {row_index} failed: {error}");
        }
        ProgressEvent::CombinedSucceeded {
            rows,
            output,
            pages,
            bytes,
            dry_run,
        } => println!(
            "{} {rows} dataset row(s) to {output} ({pages} page(s), {bytes} bytes)",
            if *dry_run { "checked" } else { "rendered" }
        ),
        ProgressEvent::Started { .. }
        | ProgressEvent::Diagnostic(_)
        | ProgressEvent::RowStarted { .. }
        | ProgressEvent::Finished { .. } => {}
    }
}

struct FileOutputWriter {
    mode: OutputMode,
    output: PathBuf,
    summary_conflicts: bool,
    reserved: HashSet<String>,
}

impl FileOutputWriter {
    fn new(mode: OutputMode, output: &Path, summary: Option<&Path>) -> Self {
        Self {
            mode,
            output: output.to_owned(),
            summary_conflicts: summary == Some(output),
            reserved: HashSet::new(),
        }
    }

    fn separate_path(&mut self, stem: &str) -> PathBuf {
        for suffix in 1.. {
            let candidate_stem = if suffix == 1 {
                stem.to_owned()
            } else {
                format!("{stem}-{suffix}")
            };
            let reservation = candidate_stem.to_ascii_lowercase();
            let candidate = self.output.join(format!("{candidate_stem}.pdf"));
            if !self.reserved.contains(&reservation) && !candidate.exists() {
                self.reserved.insert(reservation);
                return candidate;
            }
        }
        unreachable!("the collision suffix is unbounded")
    }
}

impl OutputWriter for FileOutputWriter {
    fn validate(&mut self, _mode: OutputMode) -> Result<(), BoxError> {
        if self.summary_conflicts {
            return Err(std::io::Error::other(
                "summary path must be different from the PDF output path",
            )
            .into());
        }
        Ok(())
    }

    fn prepare(&mut self, mode: OutputMode, dry_run: bool) -> Result<(), BoxError> {
        if dry_run {
            return Ok(());
        }
        match mode {
            OutputMode::Combined => ensure_parent(&self.output),
            OutputMode::Separate => fs::create_dir_all(&self.output).map_err(|source| {
                std::io::Error::new(
                    source.kind(),
                    format!(
                        "failed to create output directory {}: {source}",
                        self.output.display()
                    ),
                )
            }),
        }
        .map_err(|error| Box::new(error) as BoxError)
    }

    fn write(&mut self, artifact: OutputArtifact<'_>, dry_run: bool) -> Result<String, BoxError> {
        let path = match self.mode {
            OutputMode::Combined => self.output.clone(),
            OutputMode::Separate => self.separate_path(artifact.suggested_name),
        };
        if !dry_run {
            ensure_parent(&path).map_err(|error| -> BoxError { Box::new(error) })?;
            fs::write(&path, artifact.bytes).map_err(|source| -> BoxError {
                Box::new(std::io::Error::new(
                    source.kind(),
                    format!("failed to write PDF {}: {source}", path.display()),
                ))
            })?;
        }
        Ok(path.display().to_string())
    }
}

fn write_summary(path: &Path, summary: &JobSummary) -> Result<()> {
    ensure_parent(path)?;
    let mut json = serde_json::to_vec_pretty(summary).context("failed to serialize job summary")?;
    json.push(b'\n');
    fs::write(path, json).with_context(|| format!("failed to write job summary {}", path.display()))
}

fn ensure_parent(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|source| {
            std::io::Error::new(
                source.kind(),
                format!(
                    "failed to create output directory {}: {source}",
                    parent.display()
                ),
            )
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::RowRange;

    #[test]
    fn parses_one_based_inclusive_ranges() {
        let range = "2-5".parse::<RowRange>().unwrap();
        assert_eq!(range.0.start(), 2);
        assert_eq!(range.0.end(), 5);
    }
}
