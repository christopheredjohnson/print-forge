use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    str::FromStr,
    time::Instant,
};

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, ValueEnum};
use print_forge_dataset::DataRow;
use print_forge_engine::{BasicLayoutEngine, LayoutOptions, ResolvedDocument};
use print_forge_pdf::{PdfRenderOptions, PdfRenderer, PdfXStandard};
use print_forge_template::Template;
use print_forge_validation::{validate_data_row, validate_dataset, validate_template};
use serde::Serialize;
use serde_json::Value;

use crate::{load_dataset, load_template, print_diagnostics, require_valid};

#[derive(Debug, Args)]
pub(crate) struct RenderArgs {
    pub(crate) template: PathBuf,
    pub(crate) dataset: PathBuf,
    /// PDF path in combined mode; output directory in separate mode.
    pub(crate) output: PathBuf,
    /// Write one combined PDF or one PDF for each selected row.
    #[arg(long, value_enum, default_value_t = OutputMode::Combined)]
    pub(crate) output_mode: OutputMode,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum OutputMode {
    Combined,
    Separate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum PdfXTarget {
    X4,
}

impl From<PdfXTarget> for PdfXStandard {
    fn from(target: PdfXTarget) -> Self {
        match target {
            PdfXTarget::X4 => Self::X4,
        }
    }
}

impl OutputMode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Combined => "combined",
            Self::Separate => "separate",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RowRange {
    start: usize,
    end: usize,
}

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

        if start == 0 || end == 0 {
            return Err("row ranges are one-based; both values must be at least 1".to_owned());
        }
        if start > end {
            return Err("row range start cannot be greater than its end".to_owned());
        }

        Ok(Self { start, end })
    }
}

#[derive(Debug, Serialize)]
struct JobSummary {
    status: JobStatus,
    output_mode: &'static str,
    selected_rows: usize,
    successes: usize,
    warnings: usize,
    failures: usize,
    outputs: Vec<String>,
    records: Vec<RecordSummary>,
    elapsed_ms: u128,
}

impl JobSummary {
    fn new(output_mode: OutputMode, selected_rows: usize, warnings: usize) -> Self {
        Self {
            status: JobStatus::Succeeded,
            output_mode: output_mode.as_str(),
            selected_rows,
            successes: 0,
            warnings,
            failures: 0,
            outputs: Vec::new(),
            records: Vec::new(),
            elapsed_ms: 0,
        }
    }

    fn success(&mut self, row_index: usize, warnings: usize, output: &Path) {
        let output = output.display().to_string();
        if !self.outputs.contains(&output) {
            self.outputs.push(output.clone());
        }
        self.successes += 1;
        self.warnings += warnings;
        self.records.push(RecordSummary {
            row_index,
            row_number: row_index + 1,
            status: RecordStatus::Succeeded,
            warnings,
            output: Some(output),
            error: None,
        });
    }

    fn failure(&mut self, row_index: usize, warnings: usize, error: &anyhow::Error) {
        self.failures += 1;
        self.warnings += warnings;
        self.records.push(RecordSummary {
            row_index,
            row_number: row_index + 1,
            status: RecordStatus::Failed,
            warnings,
            output: None,
            error: Some(format!("{error:#}")),
        });
    }

    fn finish(&mut self, elapsed_ms: u128) {
        self.records.sort_by_key(|record| record.row_index);
        self.elapsed_ms = elapsed_ms;
        self.status = match (self.successes, self.failures) {
            (_, 0) => JobStatus::Succeeded,
            (0, _) => JobStatus::Failed,
            _ => JobStatus::Partial,
        };
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum JobStatus {
    Succeeded,
    Partial,
    Failed,
}

#[derive(Debug, Serialize)]
struct RecordSummary {
    row_index: usize,
    row_number: usize,
    status: RecordStatus,
    warnings: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum RecordStatus {
    Succeeded,
    Failed,
}

struct PreparedRow {
    row_index: usize,
    warnings: usize,
    document: ResolvedDocument,
}

pub(crate) fn render(arguments: &RenderArgs) -> Result<()> {
    let started = Instant::now();
    let template = load_template(&arguments.template)?;
    let dataset = load_dataset(&arguments.dataset)?;

    let template_report = validate_template(&template);
    print_diagnostics(&template_report);
    require_valid(&template_report)?;

    if dataset.rows.is_empty() {
        let report = validate_dataset(&template, &dataset);
        print_diagnostics(&report);
        require_valid(&report)?;
    }

    let row_indices = select_rows(dataset.rows.len(), arguments.rows, arguments.limit)?;
    if arguments.output_mode == OutputMode::Separate {
        validate_name_pattern(&arguments.output_name)?;
    }
    if arguments.summary.as_deref() == Some(arguments.output.as_path()) {
        bail!("summary path must be different from the PDF output path");
    }
    let pdf_options = render_options(arguments)?;

    prepare_output(arguments.output_mode, &arguments.output)?;

    let mut summary = JobSummary::new(
        arguments.output_mode,
        row_indices.len(),
        template_report.warning_count(),
    );
    let asset_base = arguments
        .template
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_owned();
    let mut prepared = Vec::new();
    let mut reserved_names = HashSet::new();
    let mut stopped = false;

    for row_index in row_indices {
        let row = &dataset.rows[row_index];
        let row_report = validate_data_row(&template, row, row_index);
        print_diagnostics(&row_report);
        let warnings = row_report.warning_count();

        if !row_report.is_valid() {
            let error = anyhow!(
                "validation failed for dataset row {row_index} with {} error(s)",
                row_report.error_count()
            );
            report_row_failure(&mut summary, row_index, warnings, &error);
            if !arguments.continue_on_error {
                stopped = true;
                break;
            }
            continue;
        }

        let rendered = render_row(&template, row, row_index, &asset_base, &pdf_options);
        let (document, pdf) = match rendered {
            Ok(rendered) => rendered,
            Err(error) => {
                report_row_failure(&mut summary, row_index, warnings, &error);
                if !arguments.continue_on_error {
                    stopped = true;
                    break;
                }
                continue;
            }
        };

        match arguments.output_mode {
            OutputMode::Combined => prepared.push(PreparedRow {
                row_index,
                warnings,
                document,
            }),
            OutputMode::Separate => {
                let output = resolve_separate_output(
                    &arguments.output,
                    &arguments.output_name,
                    row,
                    row_index,
                    &mut reserved_names,
                )
                .and_then(|output| {
                    write_pdf(&output, &pdf)?;
                    Ok(output)
                });

                match output {
                    Ok(output) => {
                        println!(
                            "rendered dataset row {row_index} to {} ({} page(s), {} bytes)",
                            output.display(),
                            document.pages.len(),
                            pdf.len()
                        );
                        summary.success(row_index, warnings, &output);
                    }
                    Err(error) => {
                        report_row_failure(&mut summary, row_index, warnings, &error);
                        if !arguments.continue_on_error {
                            stopped = true;
                            break;
                        }
                    }
                }
            }
        }
    }

    if arguments.output_mode == OutputMode::Combined && !stopped && !prepared.is_empty() {
        if let Err(error) = write_combined_output(
            &template,
            &prepared,
            &arguments.output,
            &pdf_options,
            &mut summary,
        ) {
            eprintln!("combined output failed: {error:#}");
            for row in &prepared {
                summary.failure(row.row_index, row.warnings, &error);
            }
        }
    }

    summary.finish(started.elapsed().as_millis());
    if let Some(path) = &arguments.summary {
        write_summary(path, &summary)?;
    }

    println!(
        "job summary: {} success(es), {} warning(s), {} failure(s), {} ms",
        summary.successes, summary.warnings, summary.failures, summary.elapsed_ms
    );

    if summary.failures > 0 {
        bail!(
            "rendering completed with {} failed dataset row(s)",
            summary.failures
        );
    }
    if stopped {
        bail!("rendering stopped before producing output");
    }

    Ok(())
}

fn render_options(arguments: &RenderArgs) -> Result<PdfRenderOptions> {
    let mut options = if arguments.print_ready {
        PdfRenderOptions::print_ready()
    } else {
        PdfRenderOptions::default()
    };
    if let Some(minimum) = arguments.min_image_dpi {
        if !minimum.is_finite() || minimum <= 0.0 {
            bail!("--min-image-dpi must be positive and finite");
        }
        options.min_image_dpi = Some(minimum);
    }
    options.require_embedded_fonts |= arguments.require_embedded_fonts;
    if let Some(target) = arguments.pdf_x {
        options.pdf_x = Some(target.into());
    }
    Ok(options)
}

fn select_rows(
    row_count: usize,
    range: Option<RowRange>,
    limit: Option<usize>,
) -> Result<Vec<usize>> {
    if row_count == 0 {
        bail!("cannot select rows from an empty dataset");
    }
    if limit == Some(0) {
        bail!("--limit must be at least 1");
    }

    let range = range.unwrap_or(RowRange {
        start: 1,
        end: row_count,
    });
    if range.end > row_count {
        bail!(
            "row range {}-{} exceeds dataset length {row_count}",
            range.start,
            range.end
        );
    }

    let rows = (range.start - 1)..range.end;
    Ok(match limit {
        Some(limit) => rows.take(limit).collect(),
        None => rows.collect(),
    })
}

fn prepare_output(mode: OutputMode, output: &Path) -> Result<()> {
    match mode {
        OutputMode::Combined => ensure_parent(output),
        OutputMode::Separate => fs::create_dir_all(output)
            .with_context(|| format!("failed to create output directory {}", output.display())),
    }
}

fn render_row(
    template: &Template,
    row: &DataRow,
    row_index: usize,
    asset_base: &Path,
    pdf_options: &PdfRenderOptions,
) -> Result<(ResolvedDocument, Vec<u8>)> {
    let mut document = BasicLayoutEngine
        .layout_with_options(
            template,
            row,
            &LayoutOptions {
                asset_base: asset_base.to_owned(),
            },
        )
        .with_context(|| format!("failed to lay out dataset row {row_index}"))?;

    for page in &mut document.pages {
        for command in &mut page.commands {
            command.source_path = format!("rows[{row_index}].{}", command.source_path);
        }
    }

    let pdf = PdfRenderer
        .render_with_options(&document, pdf_options)
        .with_context(|| format!("failed to render dataset row {row_index}"))?;
    Ok((document, pdf))
}

fn write_combined_output(
    template: &Template,
    prepared: &[PreparedRow],
    output: &Path,
    pdf_options: &PdfRenderOptions,
    summary: &mut JobSummary,
) -> Result<()> {
    let width_pt = prepared[0].document.width_pt;
    let height_pt = prepared[0].document.height_pt;
    let mut pages = Vec::new();
    for row in prepared {
        pages.extend(row.document.pages.iter().cloned());
    }
    let document = ResolvedDocument {
        title: format!("{} variable-data job", template.name),
        width_pt,
        height_pt,
        bleed_pt: prepared[0].document.bleed_pt,
        metadata: template.document.metadata.clone(),
        pages,
    };
    let pdf = PdfRenderer
        .render_with_options(&document, pdf_options)
        .context("failed to render combined PDF")?;
    write_pdf(output, &pdf)?;

    println!(
        "rendered {} dataset row(s) to {} ({} page(s), {} bytes)",
        prepared.len(),
        output.display(),
        document.pages.len(),
        pdf.len()
    );
    for row in prepared {
        summary.success(row.row_index, row.warnings, output);
    }
    Ok(())
}

fn resolve_separate_output(
    directory: &Path,
    pattern: &str,
    row: &DataRow,
    row_index: usize,
    reserved: &mut HashSet<String>,
) -> Result<PathBuf> {
    let interpolated = interpolate_name(pattern, row, row_index)?;
    let stem = sanitize_filename(&interpolated);
    if stem.is_empty() {
        bail!("output name for dataset row {row_index} contains no safe filename characters");
    }

    for suffix in 1.. {
        let candidate_stem = if suffix == 1 {
            stem.clone()
        } else {
            format!("{stem}-{suffix}")
        };
        let reservation = candidate_stem.to_ascii_lowercase();
        let candidate = directory.join(format!("{candidate_stem}.pdf"));
        if !reserved.contains(&reservation) && !candidate.exists() {
            reserved.insert(reservation);
            return Ok(candidate);
        }
    }

    unreachable!("the collision suffix is unbounded")
}

fn validate_name_pattern(pattern: &str) -> Result<()> {
    let mut remaining = pattern;
    while let Some(start) = remaining.find("{{") {
        if remaining[..start].contains("}}") {
            bail!("output name contains an unmatched closing delimiter");
        }
        let expression = &remaining[start + 2..];
        let end = expression
            .find("}}")
            .ok_or_else(|| anyhow!("output name contains an unclosed variable"))?;
        if expression[..end].trim().is_empty() {
            bail!("output name variable cannot be empty");
        }
        remaining = &expression[end + 2..];
    }
    if remaining.contains("}}") {
        bail!("output name contains an unmatched closing delimiter");
    }
    Ok(())
}

fn interpolate_name(pattern: &str, row: &DataRow, row_index: usize) -> Result<String> {
    let mut output = String::with_capacity(pattern.len());
    let mut remaining = pattern;

    while let Some(start) = remaining.find("{{") {
        output.push_str(&remaining[..start]);
        let expression = &remaining[start + 2..];
        let end = expression
            .find("}}")
            .ok_or_else(|| anyhow!("output name contains an unclosed variable"))?;
        let key = expression[..end].trim();

        if key == "row" {
            output.push_str(&(row_index + 1).to_string());
        } else {
            let value = lookup_value(row, key).ok_or_else(|| {
                anyhow!("output name variable is missing for dataset row {row_index}: {key}")
            })?;
            output.push_str(&filename_value(value, key, row_index)?);
        }
        remaining = &expression[end + 2..];
    }

    output.push_str(remaining);
    Ok(output)
}

fn lookup_value<'a>(row: &'a DataRow, path: &str) -> Option<&'a Value> {
    let mut segments = path.split('.');
    let mut value = row.get(segments.next()?)?;
    for segment in segments {
        value = value.get(segment)?;
    }
    Some(value)
}

fn filename_value(value: &Value, key: &str, row_index: usize) -> Result<String> {
    match value {
        Value::String(value) => Ok(value.clone()),
        Value::Number(value) => Ok(value.to_string()),
        Value::Bool(value) => Ok(value.to_string()),
        Value::Null => bail!("output name variable is null for dataset row {row_index}: {key}"),
        Value::Array(_) | Value::Object(_) => {
            bail!("output name variable must be a scalar for dataset row {row_index}: {key}")
        }
    }
}

fn sanitize_filename(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut needs_separator = false;

    for character in value.trim().chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
            if needs_separator && !output.is_empty() && !output.ends_with(['-', '_']) {
                output.push('-');
            }
            output.push(character);
            needs_separator = false;
        } else {
            needs_separator = true;
        }
    }

    output
        .trim_matches(|character| matches!(character, '-' | '_'))
        .to_owned()
}

fn report_row_failure(
    summary: &mut JobSummary,
    row_index: usize,
    warnings: usize,
    error: &anyhow::Error,
) {
    eprintln!("dataset row {row_index} failed: {error:#}");
    summary.failure(row_index, warnings, error);
}

fn write_pdf(path: &Path, pdf: &[u8]) -> Result<()> {
    ensure_parent(path)?;
    fs::write(path, pdf).with_context(|| format!("failed to write PDF {}", path.display()))
}

fn write_summary(path: &Path, summary: &JobSummary) -> Result<()> {
    ensure_parent(path)?;
    let mut json = serde_json::to_vec_pretty(summary).context("failed to serialize job summary")?;
    json.push(b'\n');
    fs::write(path, json).with_context(|| format!("failed to write job summary {}", path.display()))
}

fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create output directory {}", parent.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use print_forge_dataset::DataRow;
    use serde_json::json;

    use super::{RowRange, interpolate_name, sanitize_filename, select_rows};

    #[test]
    fn parses_and_selects_one_based_inclusive_ranges_before_limiting() {
        let range = "2-5".parse::<RowRange>().unwrap();

        assert_eq!(select_rows(8, Some(range), Some(2)).unwrap(), vec![1, 2]);
    }

    #[test]
    fn interpolates_dotted_values_and_the_one_based_row_number() {
        let row: DataRow = serde_json::from_value(json!({
            "customer": { "invoice": "INV-42" }
        }))
        .unwrap();

        assert_eq!(
            interpolate_name("{{customer.invoice}}-{{row}}", &row, 2).unwrap(),
            "INV-42-3"
        );
    }

    #[test]
    fn sanitizes_path_components_from_output_names() {
        assert_eq!(
            sanitize_filename(" ../../ACME / Invoice 7 "),
            "ACME-Invoice-7"
        );
    }
}
