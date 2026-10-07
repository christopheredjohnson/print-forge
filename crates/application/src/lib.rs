//! Reusable in-process orchestration for Print Forge render jobs.

use std::{
    collections::{BTreeMap, HashSet},
    error::Error,
    fs,
    ops::{ControlFlow, Range},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use print_forge_dataset::{DataRow, Dataset};
use print_forge_engine::{BasicLayoutEngine, LayoutError, LayoutOptions, ResolvedDocument};
use print_forge_pdf::{PdfError, PdfRenderer};
use print_forge_template::Template;
use print_forge_validation::{
    Diagnostic, Severity, ValidationReport, validate_data_row, validate_dataset, validate_template,
};
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

pub use print_forge_pdf::{PdfRenderOptions, PdfXStandard};

pub type BoxError = Box<dyn Error + Send + Sync + 'static>;

/// Data and options needed to run a render job without invoking the CLI.
pub struct RenderRequest<'a> {
    pub template: &'a Template,
    pub dataset: DatasetSource<'a>,
    pub assets: AssetSource,
    pub options: RenderOptions,
}

impl<'a> RenderRequest<'a> {
    #[must_use]
    pub fn new(template: &'a Template, dataset: &'a Dataset) -> Self {
        Self {
            template,
            dataset: DatasetSource::InMemory(dataset),
            assets: AssetSource::default(),
            options: RenderOptions::default(),
        }
    }
}

/// Repeatable source of dataset rows for bounded-memory callers.
pub trait RowSource {
    fn row_count(&mut self) -> Result<usize, BoxError>;

    fn visit(
        &mut self,
        visitor: &mut dyn FnMut(usize, DataRow) -> ControlFlow<()>,
    ) -> Result<(), BoxError>;
}

/// In-memory or caller-provided streaming data for a render request.
pub enum DatasetSource<'a> {
    InMemory(&'a Dataset),
    Streaming(&'a mut dyn RowSource),
}

impl DatasetSource<'_> {
    fn row_count(&mut self) -> Result<usize, BoxError> {
        match self {
            Self::InMemory(dataset) => Ok(dataset.rows.len()),
            Self::Streaming(source) => source.row_count(),
        }
    }

    fn visit(
        &mut self,
        visitor: &mut dyn FnMut(usize, DataRow) -> ControlFlow<()>,
    ) -> Result<(), BoxError> {
        match self {
            Self::InMemory(dataset) => {
                for (row_index, row) in dataset.rows.iter().cloned().enumerate() {
                    if visitor(row_index, row).is_break() {
                        break;
                    }
                }
                Ok(())
            }
            Self::Streaming(source) => source.visit(visitor),
        }
    }
}

/// Assets referenced by relative paths in a template.
#[derive(Debug, Clone, Default)]
pub enum AssetSource {
    /// Resolve template assets relative to this directory.
    #[default]
    CurrentDirectory,
    Directory(PathBuf),
    /// Materialize caller-provided bytes in an isolated temporary directory.
    Bundled(AssetBundle),
}

/// An in-memory set of assets keyed by the relative paths used by a template.
#[derive(Debug, Clone, Default)]
pub struct AssetBundle {
    assets: BTreeMap<PathBuf, Vec<u8>>,
}

impl AssetBundle {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, path: impl Into<PathBuf>, bytes: impl Into<Vec<u8>>) {
        self.assets.insert(path.into(), bytes.into());
    }

    #[must_use]
    pub fn with_asset(mut self, path: impl Into<PathBuf>, bytes: impl Into<Vec<u8>>) -> Self {
        self.insert(path, bytes);
        self
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.assets.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.assets.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RenderOptions {
    pub output_mode: OutputMode,
    pub rows: Option<RowRange>,
    pub limit: Option<usize>,
    pub continue_on_error: bool,
    pub output_name: String,
    pub pdf: PdfRenderOptions,
    pub dry_run: bool,
    pub warnings: WarningPolicy,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            output_mode: OutputMode::Combined,
            rows: None,
            limit: None,
            continue_on_error: false,
            output_name: "row-{{row}}".to_owned(),
            pdf: PdfRenderOptions::default(),
            dry_run: false,
            warnings: WarningPolicy::Allow,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputMode {
    Combined,
    Separate,
}

impl OutputMode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Combined => "combined",
            Self::Separate => "separate",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarningPolicy {
    Allow,
    Deny,
    Ignore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowRange {
    start: usize,
    end: usize,
}

impl RowRange {
    pub fn new(start: usize, end: usize) -> Result<Self, JobError> {
        if start == 0 || end == 0 {
            return Err(JobError::InvalidRequest {
                message: "row ranges are one-based; both values must be at least 1".to_owned(),
            });
        }
        if start > end {
            return Err(JobError::InvalidRequest {
                message: "row range start cannot be greater than its end".to_owned(),
            });
        }
        Ok(Self { start, end })
    }

    #[must_use]
    pub const fn start(self) -> usize {
        self.start
    }

    #[must_use]
    pub const fn end(self) -> usize {
        self.end
    }
}

/// A rendered PDF offered to a caller-controlled destination.
#[derive(Debug)]
pub struct OutputArtifact<'a> {
    pub row_index: Option<usize>,
    pub suggested_name: &'a str,
    pub bytes: &'a [u8],
}

/// Receives rendered artifacts and returns their stable caller-facing identifiers.
pub trait OutputWriter {
    fn validate(&mut self, _mode: OutputMode) -> Result<(), BoxError> {
        Ok(())
    }

    fn prepare(&mut self, _mode: OutputMode, _dry_run: bool) -> Result<(), BoxError> {
        Ok(())
    }

    fn write(&mut self, artifact: OutputArtifact<'_>, dry_run: bool) -> Result<String, BoxError>;
}

/// In-memory output destination suitable for workers, services, and tests.
#[derive(Debug, Default)]
pub struct MemoryOutputWriter {
    artifacts: BTreeMap<String, Vec<u8>>,
    reserved: HashSet<String>,
}

impl MemoryOutputWriter {
    #[must_use]
    pub fn artifacts(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.artifacts
    }

    #[must_use]
    pub fn into_artifacts(self) -> BTreeMap<String, Vec<u8>> {
        self.artifacts
    }
}

impl OutputWriter for MemoryOutputWriter {
    fn write(&mut self, artifact: OutputArtifact<'_>, dry_run: bool) -> Result<String, BoxError> {
        let base = artifact.suggested_name;
        let mut suffix = 1;
        loop {
            let name = if suffix == 1 {
                format!("{base}.pdf")
            } else {
                format!("{base}-{suffix}.pdf")
            };
            if !self.reserved.contains(&name) {
                self.reserved.insert(name.clone());
                if !dry_run {
                    self.artifacts.insert(name.clone(), artifact.bytes.to_vec());
                }
                return Ok(name);
            }
            suffix += 1;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Succeeded,
    Partial,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordStatus {
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticPhase {
    Validation,
    Layout,
    Preflight,
    Rendering,
    Output,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JobDiagnostic {
    pub severity: DiagnosticSeverity,
    pub phase: DiagnosticPhase,
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element_path: Option<String>,
}

impl std::fmt::Display for JobDiagnostic {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.phase == DiagnosticPhase::Validation {
            write!(
                formatter,
                "{}[{}] {}: {}",
                match self.severity {
                    DiagnosticSeverity::Error => "error",
                    DiagnosticSeverity::Warning => "warning",
                },
                self.code,
                self.path.as_deref().unwrap_or("job"),
                self.message
            )
        } else {
            formatter.write_str(&self.message)
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RecordSummary {
    pub row_index: usize,
    pub row_number: usize,
    pub status: RecordStatus,
    pub warnings: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<JobDiagnostic>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JobSummary {
    pub status: JobStatus,
    pub output_mode: &'static str,
    pub selected_rows: usize,
    pub successes: usize,
    pub warnings: usize,
    pub failures: usize,
    pub outputs: Vec<String>,
    pub records: Vec<RecordSummary>,
    pub elapsed_ms: u128,
    pub dry_run: bool,
}

impl JobSummary {
    fn new(output_mode: OutputMode, selected_rows: usize, warnings: usize, dry_run: bool) -> Self {
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
            dry_run,
        }
    }

    fn success(&mut self, row_index: usize, warnings: usize, output: &str) {
        if !self.outputs.iter().any(|candidate| candidate == output) {
            self.outputs.push(output.to_owned());
        }
        self.successes += 1;
        self.warnings += warnings;
        self.records.push(RecordSummary {
            row_index,
            row_number: row_index + 1,
            status: RecordStatus::Succeeded,
            warnings,
            output: Some(output.to_owned()),
            error: None,
            diagnostics: Vec::new(),
        });
    }

    fn failure(
        &mut self,
        row_index: usize,
        warnings: usize,
        error: String,
        diagnostics: Vec<JobDiagnostic>,
    ) {
        self.failures += 1;
        self.warnings += warnings;
        self.records.push(RecordSummary {
            row_index,
            row_number: row_index + 1,
            status: RecordStatus::Failed,
            warnings,
            output: None,
            error: Some(error),
            diagnostics,
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

#[derive(Debug, Clone)]
pub struct RenderOutcome {
    pub summary: JobSummary,
    pub diagnostics: Vec<JobDiagnostic>,
    pub stopped_early: bool,
}

impl RenderOutcome {
    #[must_use]
    pub fn is_success(&self) -> bool {
        self.summary.failures == 0 && !self.stopped_early
    }
}

#[derive(Debug, Clone)]
pub enum ProgressEvent {
    Started {
        selected_rows: usize,
        dry_run: bool,
    },
    Diagnostic(JobDiagnostic),
    RowStarted {
        row_index: usize,
    },
    RowSucceeded {
        row_index: usize,
        output: String,
        pages: usize,
        bytes: usize,
        dry_run: bool,
    },
    RowFailed {
        row_index: usize,
        error: String,
    },
    CombinedSucceeded {
        rows: usize,
        output: String,
        pages: usize,
        bytes: usize,
        dry_run: bool,
    },
    Finished {
        status: JobStatus,
    },
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum JobError {
    #[error("{message}")]
    InvalidRequest { message: String },
    #[error("{message}")]
    Validation {
        message: String,
        diagnostics: Vec<JobDiagnostic>,
    },
    #[error("dataset source failed: {source}")]
    Dataset {
        #[source]
        source: BoxError,
    },
    #[error("{source}")]
    Output {
        #[source]
        source: BoxError,
    },
    #[error(transparent)]
    Assets(#[from] AssetError),
}

impl JobError {
    #[must_use]
    pub fn diagnostics(&self) -> &[JobDiagnostic] {
        match self {
            Self::Validation { diagnostics, .. } => diagnostics,
            Self::InvalidRequest { .. }
            | Self::Dataset { .. }
            | Self::Output { .. }
            | Self::Assets(_) => &[],
        }
    }
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AssetError {
    #[error("asset path must be a nonempty relative path without parent traversal: {path}")]
    InvalidPath { path: PathBuf },
    #[error("failed to create temporary asset directory {path}")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write temporary asset {path}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

struct PreparedRow {
    row_index: usize,
    warnings: usize,
    document: ResolvedDocument,
}

/// Runs a render job with no progress observer.
pub fn render(
    request: RenderRequest<'_>,
    output: &mut impl OutputWriter,
) -> Result<RenderOutcome, JobError> {
    render_with_progress(request, output, |_| {})
}

/// Runs a render job and reports structured progress synchronously.
pub fn render_with_progress(
    mut request: RenderRequest<'_>,
    output: &mut impl OutputWriter,
    mut progress: impl FnMut(ProgressEvent),
) -> Result<RenderOutcome, JobError> {
    let started = Instant::now();

    let template_report = validate_template(request.template);
    let mut diagnostics = report_diagnostics(&template_report);
    emit_visible_diagnostics(&diagnostics, request.options.warnings, &mut progress);
    require_acceptable(&template_report, request.options.warnings, &diagnostics)?;

    let row_count = request
        .dataset
        .row_count()
        .map_err(|source| JobError::Dataset { source })?;
    if row_count == 0 {
        let report = validate_dataset(request.template, &Dataset::default());
        let empty_diagnostics = report_diagnostics(&report);
        emit_visible_diagnostics(&empty_diagnostics, request.options.warnings, &mut progress);
        require_acceptable(&report, request.options.warnings, &empty_diagnostics)?;
    }

    let row_indices = select_rows(row_count, request.options.rows, request.options.limit)?;
    if request.options.output_mode == OutputMode::Separate {
        validate_name_pattern(&request.options.output_name)
            .map_err(|message| JobError::InvalidRequest { message })?;
    }
    output
        .validate(request.options.output_mode)
        .map_err(|source| JobError::Output { source })?;
    validate_pdf_options(&request.options)?;
    let selected_rows = row_indices.len();
    progress(ProgressEvent::Started {
        selected_rows,
        dry_run: request.options.dry_run,
    });

    let resolved_assets = ResolvedAssets::prepare(&request.assets)?;
    output
        .prepare(request.options.output_mode, request.options.dry_run)
        .map_err(|source| JobError::Output { source })?;
    let mut summary = JobSummary::new(
        request.options.output_mode,
        selected_rows,
        template_report.warning_count(),
        request.options.dry_run,
    );
    let mut prepared = Vec::new();
    let mut stopped = false;

    let selected_start = row_indices.start;
    let selected_end = row_indices.end;
    request
        .dataset
        .visit(&mut |row_index, row| {
            if row_index < selected_start {
                return ControlFlow::Continue(());
            }
            if row_index >= selected_end {
                return ControlFlow::Break(());
            }
        progress(ProgressEvent::RowStarted { row_index });
        let row_report = validate_data_row(request.template, &row, row_index);
        let row_diagnostics = report_diagnostics(&row_report);
        emit_visible_diagnostics(&row_diagnostics, request.options.warnings, &mut progress);
        diagnostics.extend(row_diagnostics.iter().cloned());
        let warnings = row_report.warning_count();

        if !is_acceptable(&row_report, request.options.warnings) {
            let error = format!(
                "validation failed for dataset row {row_index} with {} error(s) and {} warning(s)",
                row_report.error_count(),
                row_report.warning_count()
            );
            progress(ProgressEvent::RowFailed {
                row_index,
                error: error.clone(),
            });
            summary.failure(row_index, warnings, error, row_diagnostics);
            if !request.options.continue_on_error {
                stopped = true;
                return ControlFlow::Break(());
            }
            return ControlFlow::Continue(());
        }

        let (document, pdf) = match render_row(
            request.template,
            &row,
            row_index,
            resolved_assets.path(),
            &request.options.pdf,
        ) {
            Ok(rendered) => rendered,
            Err(diagnostic) => {
                let diagnostic = *diagnostic;
                let error = diagnostic.message.clone();
                progress(ProgressEvent::Diagnostic(diagnostic.clone()));
                progress(ProgressEvent::RowFailed {
                    row_index,
                    error: error.clone(),
                });
                diagnostics.push(diagnostic.clone());
                summary.failure(row_index, warnings, error, vec![diagnostic]);
                if !request.options.continue_on_error {
                    stopped = true;
                    return ControlFlow::Break(());
                }
                return ControlFlow::Continue(());
            }
        };

        match request.options.output_mode {
            OutputMode::Combined => prepared.push(PreparedRow {
                row_index,
                warnings,
                document,
            }),
            OutputMode::Separate => {
                let suggested_name =
                    match resolve_output_name(&request.options.output_name, &row, row_index) {
                        Ok(name) => name,
                        Err(error) => {
                            let diagnostic = row_diagnostic(
                                row_index,
                                DiagnosticPhase::Output,
                                "output.invalid_name",
                                error,
                                None,
                                None,
                            );
                            let message = diagnostic.message.clone();
                            progress(ProgressEvent::Diagnostic(diagnostic.clone()));
                            progress(ProgressEvent::RowFailed {
                                row_index,
                                error: message.clone(),
                            });
                            diagnostics.push(diagnostic.clone());
                            summary.failure(row_index, warnings, message, vec![diagnostic]);
                            if !request.options.continue_on_error {
                                stopped = true;
                                return ControlFlow::Break(());
                            }
                            return ControlFlow::Continue(());
                        }
                    };
                let artifact = OutputArtifact {
                    row_index: Some(row_index),
                    suggested_name: &suggested_name,
                    bytes: &pdf,
                };
                match output.write(artifact, request.options.dry_run) {
                    Ok(identifier) => {
                        progress(ProgressEvent::RowSucceeded {
                            row_index,
                            output: identifier.clone(),
                            pages: document.pages.len(),
                            bytes: pdf.len(),
                            dry_run: request.options.dry_run,
                        });
                        summary.success(row_index, warnings, &identifier);
                    }
                    Err(error) => {
                        let diagnostic = row_diagnostic(
                            row_index,
                            DiagnosticPhase::Output,
                            "output.write_failed",
                            error.to_string(),
                            None,
                            None,
                        );
                        let message = diagnostic.message.clone();
                        progress(ProgressEvent::Diagnostic(diagnostic.clone()));
                        progress(ProgressEvent::RowFailed {
                            row_index,
                            error: message.clone(),
                        });
                        diagnostics.push(diagnostic.clone());
                        summary.failure(row_index, warnings, message, vec![diagnostic]);
                        if !request.options.continue_on_error {
                            stopped = true;
                            return ControlFlow::Break(());
                        }
                    }
                }
            }
        }
            ControlFlow::Continue(())
        })
        .map_err(|source| JobError::Dataset { source })?;

    if request.options.output_mode == OutputMode::Combined && !stopped && !prepared.is_empty() {
        match render_combined(request.template, &prepared, &request.options.pdf) {
            Ok((document, pdf)) => {
                let artifact = OutputArtifact {
                    row_index: None,
                    suggested_name: "combined",
                    bytes: &pdf,
                };
                match output.write(artifact, request.options.dry_run) {
                    Ok(identifier) => {
                        progress(ProgressEvent::CombinedSucceeded {
                            rows: prepared.len(),
                            output: identifier.clone(),
                            pages: document.pages.len(),
                            bytes: pdf.len(),
                            dry_run: request.options.dry_run,
                        });
                        for row in &prepared {
                            summary.success(row.row_index, row.warnings, &identifier);
                        }
                    }
                    Err(error) => {
                        let diagnostic = JobDiagnostic {
                            severity: DiagnosticSeverity::Error,
                            phase: DiagnosticPhase::Output,
                            code: "output.write_failed".to_owned(),
                            message: error.to_string(),
                            path: None,
                            row_index: None,
                            page_index: None,
                            element_path: None,
                        };
                        fail_prepared_rows(
                            &prepared,
                            &mut summary,
                            &mut diagnostics,
                            &diagnostic,
                            &mut progress,
                        );
                    }
                }
            }
            Err(diagnostic) => {
                let diagnostic = *diagnostic;
                fail_prepared_rows(
                    &prepared,
                    &mut summary,
                    &mut diagnostics,
                    &diagnostic,
                    &mut progress,
                );
            }
        }
    }

    summary.finish(started.elapsed().as_millis());
    progress(ProgressEvent::Finished {
        status: summary.status.clone(),
    });
    Ok(RenderOutcome {
        summary,
        diagnostics,
        stopped_early: stopped,
    })
}

fn validate_pdf_options(options: &RenderOptions) -> Result<(), JobError> {
    if let Some(minimum) = options.pdf.min_image_dpi
        && (!minimum.is_finite() || minimum <= 0.0)
    {
        return Err(JobError::InvalidRequest {
            message: "--min-image-dpi must be positive and finite".to_owned(),
        });
    }
    Ok(())
}

fn is_acceptable(report: &ValidationReport, policy: WarningPolicy) -> bool {
    report.is_valid() && !(policy == WarningPolicy::Deny && report.warning_count() > 0)
}

fn require_acceptable(
    report: &ValidationReport,
    policy: WarningPolicy,
    diagnostics: &[JobDiagnostic],
) -> Result<(), JobError> {
    if is_acceptable(report, policy) {
        return Ok(());
    }
    Err(JobError::Validation {
        message: format!(
            "validation failed with {} error(s) and {} warning(s){}",
            report.error_count(),
            report.warning_count(),
            if policy == WarningPolicy::Deny {
                " (--warnings deny)"
            } else {
                ""
            }
        ),
        diagnostics: diagnostics.to_vec(),
    })
}

fn report_diagnostics(report: &ValidationReport) -> Vec<JobDiagnostic> {
    report
        .diagnostics()
        .iter()
        .map(validation_diagnostic)
        .collect()
}

fn validation_diagnostic(diagnostic: &Diagnostic) -> JobDiagnostic {
    let path = diagnostic.path.clone();
    JobDiagnostic {
        severity: match diagnostic.severity {
            Severity::Error => DiagnosticSeverity::Error,
            Severity::Warning => DiagnosticSeverity::Warning,
        },
        phase: DiagnosticPhase::Validation,
        code: diagnostic.code.to_owned(),
        message: diagnostic.message.clone(),
        row_index: context_index(&path, "rows["),
        page_index: context_index(&path, "pages["),
        element_path: path.contains("elements[").then(|| path.clone()),
        path: Some(path),
    }
}

fn emit_visible_diagnostics(
    diagnostics: &[JobDiagnostic],
    policy: WarningPolicy,
    progress: &mut impl FnMut(ProgressEvent),
) {
    for diagnostic in diagnostics {
        if policy != WarningPolicy::Ignore || diagnostic.severity == DiagnosticSeverity::Error {
            progress(ProgressEvent::Diagnostic(diagnostic.clone()));
        }
    }
}

fn select_rows(
    row_count: usize,
    range: Option<RowRange>,
    limit: Option<usize>,
) -> Result<Range<usize>, JobError> {
    if row_count == 0 {
        return Err(JobError::InvalidRequest {
            message: "cannot select rows from an empty dataset".to_owned(),
        });
    }
    if limit == Some(0) {
        return Err(JobError::InvalidRequest {
            message: "--limit must be at least 1".to_owned(),
        });
    }
    let range = range.unwrap_or(RowRange {
        start: 1,
        end: row_count,
    });
    if range.end > row_count {
        return Err(JobError::InvalidRequest {
            message: format!(
                "row range {}-{} exceeds dataset length {row_count}",
                range.start, range.end
            ),
        });
    }
    let start = range.start - 1;
    let end = limit.map_or(range.end, |limit| range.end.min(start + limit));
    Ok(start..end)
}

fn render_row(
    template: &Template,
    row: &DataRow,
    row_index: usize,
    asset_base: &Path,
    pdf_options: &PdfRenderOptions,
) -> Result<(ResolvedDocument, Vec<u8>), Box<JobDiagnostic>> {
    let mut document = BasicLayoutEngine
        .layout_with_options(
            template,
            row,
            &LayoutOptions {
                asset_base: asset_base.to_owned(),
            },
        )
        .map_err(|error| Box::new(layout_diagnostic(row_index, error)))?;

    for page in &mut document.pages {
        for command in &mut page.commands {
            command.source_path = format!("rows[{row_index}].{}", command.source_path);
        }
    }
    let pdf = PdfRenderer
        .render_with_options(&document, pdf_options)
        .map_err(|error| Box::new(pdf_diagnostic(Some(row_index), error)))?;
    Ok((document, pdf))
}

fn render_combined(
    template: &Template,
    prepared: &[PreparedRow],
    pdf_options: &PdfRenderOptions,
) -> Result<(ResolvedDocument, Vec<u8>), Box<JobDiagnostic>> {
    let mut pages = Vec::new();
    for row in prepared {
        pages.extend(row.document.pages.iter().cloned());
    }
    let document = ResolvedDocument {
        title: format!("{} variable-data job", template.name),
        width_pt: prepared[0].document.width_pt,
        height_pt: prepared[0].document.height_pt,
        bleed_pt: prepared[0].document.bleed_pt,
        metadata: template.document.metadata.clone(),
        pages,
    };
    let pdf = PdfRenderer
        .render_with_options(&document, pdf_options)
        .map_err(|error| Box::new(pdf_diagnostic(None, error)))?;
    Ok((document, pdf))
}

fn layout_diagnostic(row_index: usize, error: LayoutError) -> JobDiagnostic {
    let (page_index, element_path) = match &error {
        LayoutError::Element {
            page_index,
            element_path,
            ..
        } => (Some(*page_index), Some(element_path.clone())),
        LayoutError::Document { .. } => (None, None),
    };
    row_diagnostic(
        row_index,
        DiagnosticPhase::Layout,
        "layout.failed",
        format!("failed to lay out dataset row {row_index}: {error}"),
        page_index,
        element_path,
    )
}

fn pdf_diagnostic(row_index: Option<usize>, error: PdfError) -> JobDiagnostic {
    let message = row_index.map_or_else(
        || format!("failed to render combined PDF: {error}"),
        |row_index| format!("failed to render dataset row {row_index}: {error}"),
    );
    let phase = match error {
        PdfError::Preflight(_) => DiagnosticPhase::Preflight,
        PdfError::Render(_) | PdfError::Conformance(_) => DiagnosticPhase::Rendering,
        _ => DiagnosticPhase::Rendering,
    };
    let page_index = context_index(&message, "page ");
    let element_path = message
        .split_once(", element ")
        .map(|(_, value)| value.split(':').next().unwrap_or(value).to_owned());
    let row_index = row_index.or_else(|| {
        element_path
            .as_deref()
            .and_then(|path| context_index(path, "rows["))
    });
    JobDiagnostic {
        severity: DiagnosticSeverity::Error,
        phase,
        code: "pdf.failed".to_owned(),
        message,
        path: element_path.clone(),
        row_index,
        page_index,
        element_path,
    }
}

fn row_diagnostic(
    row_index: usize,
    phase: DiagnosticPhase,
    code: &str,
    message: impl Into<String>,
    page_index: Option<usize>,
    element_path: Option<String>,
) -> JobDiagnostic {
    JobDiagnostic {
        severity: DiagnosticSeverity::Error,
        phase,
        code: code.to_owned(),
        message: message.into(),
        path: element_path.clone(),
        row_index: Some(row_index),
        page_index,
        element_path,
    }
}

fn fail_prepared_rows(
    prepared: &[PreparedRow],
    summary: &mut JobSummary,
    diagnostics: &mut Vec<JobDiagnostic>,
    failure: &JobDiagnostic,
    progress: &mut impl FnMut(ProgressEvent),
) {
    for row in prepared {
        let context = (failure.row_index == Some(row.row_index)).then_some(failure);
        let diagnostic = row_diagnostic(
            row.row_index,
            failure.phase.clone(),
            &failure.code,
            failure.message.clone(),
            context.and_then(|diagnostic| diagnostic.page_index),
            context.and_then(|diagnostic| diagnostic.element_path.clone()),
        );
        progress(ProgressEvent::Diagnostic(diagnostic.clone()));
        progress(ProgressEvent::RowFailed {
            row_index: row.row_index,
            error: failure.message.clone(),
        });
        diagnostics.push(diagnostic.clone());
        summary.failure(
            row.row_index,
            row.warnings,
            failure.message.clone(),
            vec![diagnostic],
        );
    }
}

fn context_index(value: &str, prefix: &str) -> Option<usize> {
    let (_, tail) = value.split_once(prefix)?;
    tail.chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .ok()
}

fn validate_name_pattern(pattern: &str) -> Result<(), String> {
    let mut remaining = pattern;
    while let Some(start) = remaining.find("{{") {
        if remaining[..start].contains("}}") {
            return Err("output name contains an unmatched closing delimiter".to_owned());
        }
        let expression = &remaining[start + 2..];
        let end = expression
            .find("}}")
            .ok_or_else(|| "output name contains an unclosed variable".to_owned())?;
        if expression[..end].trim().is_empty() {
            return Err("output name variable cannot be empty".to_owned());
        }
        remaining = &expression[end + 2..];
    }
    if remaining.contains("}}") {
        return Err("output name contains an unmatched closing delimiter".to_owned());
    }
    Ok(())
}

fn resolve_output_name(pattern: &str, row: &DataRow, row_index: usize) -> Result<String, String> {
    let interpolated = interpolate_name(pattern, row, row_index)?;
    let stem = sanitize_filename(&interpolated);
    if stem.is_empty() {
        return Err(format!(
            "output name for dataset row {row_index} contains no safe filename characters"
        ));
    }
    Ok(stem)
}

fn interpolate_name(pattern: &str, row: &DataRow, row_index: usize) -> Result<String, String> {
    let mut output = String::with_capacity(pattern.len());
    let mut remaining = pattern;
    while let Some(start) = remaining.find("{{") {
        output.push_str(&remaining[..start]);
        let expression = &remaining[start + 2..];
        let end = expression
            .find("}}")
            .ok_or_else(|| "output name contains an unclosed variable".to_owned())?;
        let key = expression[..end].trim();
        if key == "row" {
            output.push_str(&(row_index + 1).to_string());
        } else {
            let value = lookup_value(row, key).ok_or_else(|| {
                format!("output name variable is missing for dataset row {row_index}: {key}")
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

fn filename_value(value: &Value, key: &str, row_index: usize) -> Result<String, String> {
    match value {
        Value::String(value) => Ok(value.clone()),
        Value::Number(value) => Ok(value.to_string()),
        Value::Bool(value) => Ok(value.to_string()),
        Value::Null => Err(format!(
            "output name variable is null for dataset row {row_index}: {key}"
        )),
        Value::Array(_) | Value::Object(_) => Err(format!(
            "output name variable must be a scalar for dataset row {row_index}: {key}"
        )),
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

enum ResolvedAssets {
    Directory(PathBuf),
    Scratch(ScratchDirectory),
}

impl ResolvedAssets {
    fn prepare(source: &AssetSource) -> Result<Self, AssetError> {
        match source {
            AssetSource::CurrentDirectory => Ok(Self::Directory(PathBuf::from("."))),
            AssetSource::Directory(path) => Ok(Self::Directory(path.clone())),
            AssetSource::Bundled(bundle) => {
                let directory = ScratchDirectory::new()?;
                for (relative, bytes) in &bundle.assets {
                    validate_asset_path(relative)?;
                    let path = directory.path.join(relative);
                    if let Some(parent) = path.parent() {
                        fs::create_dir_all(parent).map_err(|source| {
                            AssetError::CreateDirectory {
                                path: parent.to_owned(),
                                source,
                            }
                        })?;
                    }
                    fs::write(&path, bytes).map_err(|source| AssetError::Write { path, source })?;
                }
                Ok(Self::Scratch(directory))
            }
        }
    }

    fn path(&self) -> &Path {
        match self {
            Self::Directory(path) => path,
            Self::Scratch(directory) => &directory.path,
        }
    }
}

fn validate_asset_path(path: &Path) -> Result<(), AssetError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(AssetError::InvalidPath {
            path: path.to_owned(),
        });
    }
    Ok(())
}

struct ScratchDirectory {
    path: PathBuf,
}

impl ScratchDirectory {
    fn new() -> Result<Self, AssetError> {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "print-forge-assets-{}-{nonce}-{id}",
            std::process::id()
        ));
        fs::create_dir(&path).map_err(|source| AssetError::CreateDirectory {
            path: path.clone(),
            source,
        })?;
        Ok(Self { path })
    }
}

impl Drop for ScratchDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use print_forge_dataset::Dataset;
    use print_forge_template::Template;
    use serde_json::json;

    use super::{
        AssetBundle, AssetSource, DatasetSource, DiagnosticPhase, MemoryOutputWriter, OutputMode,
        ProgressEvent, RenderOptions, RenderRequest, RowRange, render, render_with_progress,
    };

    fn template(elements: &str) -> Template {
        serde_json::from_str(&format!(
            r##"{{
              "name":"Application test",
              "document":{{
                "width":{{"value":100,"unit":"points"}},
                "height":{{"value":100,"unit":"points"}}
              }},
              "pages":[{{"elements":[{elements}]}}]
            }}"##
        ))
        .unwrap()
    }

    #[test]
    fn renders_selected_rows_to_memory_without_cli_or_paths() {
        let template = template(
            r##"{
              "type":"text",
              "position":{
                "x":{"value":5,"unit":"points"},
                "y":{"value":30,"unit":"points"},
                "width":{"value":90,"unit":"points"},
                "height":{"value":20,"unit":"points"}
              },
              "value":"{{name}}",
              "font_size":{"value":12,"unit":"points"}
            }"##,
        );
        let dataset = Dataset {
            rows: vec![
                serde_json::from_value(json!({"name":"Ada"})).unwrap(),
                serde_json::from_value(json!({"name":"Grace"})).unwrap(),
            ],
        };
        let mut output = MemoryOutputWriter::default();
        let outcome = render(
            RenderRequest {
                template: &template,
                dataset: DatasetSource::InMemory(&dataset),
                assets: AssetSource::default(),
                options: RenderOptions {
                    rows: Some(RowRange::new(2, 2).unwrap()),
                    ..RenderOptions::default()
                },
            },
            &mut output,
        )
        .unwrap();

        assert!(outcome.is_success());
        assert_eq!(outcome.summary.selected_rows, 1);
        assert_eq!(outcome.summary.records[0].row_index, 1);
        assert!(output.artifacts()["combined.pdf"].starts_with(b"%PDF"));
    }

    #[test]
    fn returns_typed_row_page_and_element_context() {
        let template = template(
            r##"{
              "type":"text",
              "position":{
                "x":{"value":5,"unit":"points"},
                "y":{"value":30,"unit":"points"},
                "width":{"value":90,"unit":"points"},
                "height":{"value":20,"unit":"points"}
              },
              "value":"{{missing}}",
              "font_size":{"value":12,"unit":"points"}
            }"##,
        );
        let dataset = Dataset {
            rows: vec![serde_json::from_value(json!({})).unwrap()],
        };
        let mut output = MemoryOutputWriter::default();
        let outcome = render(
            RenderRequest {
                template: &template,
                dataset: DatasetSource::InMemory(&dataset),
                assets: AssetSource::default(),
                options: RenderOptions::default(),
            },
            &mut output,
        )
        .unwrap();

        let diagnostic = outcome
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.phase == DiagnosticPhase::Layout)
            .unwrap();
        assert_eq!(diagnostic.row_index, Some(0));
        assert_eq!(diagnostic.page_index, Some(0));
        assert_eq!(
            diagnostic.element_path.as_deref(),
            Some("pages[0].elements[0]")
        );
    }

    #[test]
    fn materializes_bundled_assets_for_rendering() {
        let template = template(
            r##"{
              "type":"svg",
              "position":{
                "x":{"value":5,"unit":"points"},
                "y":{"value":5,"unit":"points"},
                "width":{"value":40,"unit":"points"},
                "height":{"value":40,"unit":"points"}
              },
              "source":"assets/mark.svg"
            }"##,
        );
        let dataset = Dataset {
            rows: vec![serde_json::from_value(json!({})).unwrap()],
        };
        let assets = AssetBundle::new().with_asset(
            "assets/mark.svg",
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="black"/></svg>"#,
        );
        let mut output = MemoryOutputWriter::default();
        let outcome = render(
            RenderRequest {
                template: &template,
                dataset: DatasetSource::InMemory(&dataset),
                assets: AssetSource::Bundled(assets),
                options: RenderOptions::default(),
            },
            &mut output,
        )
        .unwrap();

        assert!(outcome.is_success());
        assert!(output.artifacts()["combined.pdf"].starts_with(b"%PDF"));
    }

    #[test]
    fn reports_structured_progress_for_separate_outputs() {
        let template = template("");
        let dataset = Dataset {
            rows: vec![
                serde_json::from_value(json!({"name":"same"})).unwrap(),
                serde_json::from_value(json!({"name":"same"})).unwrap(),
            ],
        };
        let mut output = MemoryOutputWriter::default();
        let mut events = Vec::new();
        let outcome = render_with_progress(
            RenderRequest {
                template: &template,
                dataset: DatasetSource::InMemory(&dataset),
                assets: AssetSource::default(),
                options: RenderOptions {
                    output_mode: OutputMode::Separate,
                    output_name: "{{name}}".to_owned(),
                    ..RenderOptions::default()
                },
            },
            &mut output,
            |event| events.push(event),
        )
        .unwrap();

        assert!(outcome.is_success());
        assert!(output.artifacts().contains_key("same.pdf"));
        assert!(output.artifacts().contains_key("same-2.pdf"));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, ProgressEvent::RowSucceeded { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn combined_pdf_diagnostics_recover_typed_row_page_and_element_context() {
        let diagnostic = super::pdf_diagnostic(
            None,
            super::PdfError::Render(anyhow::anyhow!(
                "page 3, element rows[7].pages[1].elements[2]: fixture failure"
            )),
        );

        assert_eq!(diagnostic.row_index, Some(7));
        assert_eq!(diagnostic.page_index, Some(3));
        assert_eq!(
            diagnostic.element_path.as_deref(),
            Some("rows[7].pages[1].elements[2]")
        );
    }
}
