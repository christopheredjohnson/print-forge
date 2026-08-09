<p align="center">
  <img src="assets/brand/print-forge-logo.png" alt="Print Forge logo" width="720">
</p>

# Print Forge

Print Forge is an early Rust workspace for data-driven print and PDF generation.
The template and layout layers are deliberately independent from the PDF library
so additional renderers can be added without changing the project format.

## Workspace

```text
apps/cli          Command-line entry point
crates/template   Serializable template schema
crates/dataset    CSV and JSON dataset loading
crates/engine     Renderer-neutral resolved document and draw commands
crates/pdf        PDF renderer boundary and printpdf adapter home
crates/validation Semantic validation and preflight diagnostics
examples          Example templates and data
```

## Try it

```sh
cargo run -- validate examples/business-card.json
cargo run -- validate examples/business-card.json --dataset examples/people.csv
cargo run -- render examples/business-card.json examples/people.csv output/pdf/business-cards.pdf
cargo run -- render examples/absolute-layout.json examples/absolute-layout-data.json output/pdf/absolute-layout.pdf
cargo test --workspace
```

To see validation failures for missing and empty required CSV fields:

```sh
cargo run -- validate examples/business-card.json --dataset examples/invalid-people.csv
```

## Variable-data jobs

Combined mode is the default. Every selected dataset row contributes the
template's pages to one PDF:

```sh
cargo run -- render examples/business-card.json examples/people.csv \
  output/pdf/business-cards.pdf \
  --summary output/jobs/business-cards.json
```

Separate mode writes one PDF per row. Output names accept dotted dataset fields
and the special one-based `{{row}}` value. Names are made filesystem-safe, and
existing or duplicate names receive a numeric suffix instead of being
overwritten:

```sh
cargo run -- render examples/business-card.json examples/people.csv \
  output/pdf/business-cards \
  --output-mode separate \
  --output-name '{{last_name}}-{{first_name}}' \
  --rows 1-2 \
  --limit 2 \
  --continue-on-error \
  --summary output/jobs/business-cards.json
```

Row ranges are one-based and inclusive. `--limit` is applied after the range.
`--continue-on-error` attempts the remaining rows and produces valid partial
output, but the command still exits unsuccessfully when any row fails. The JSON
summary records status, successes, warnings, failures, output paths, per-row
results, and elapsed milliseconds.

## Print-ready output

`--print-ready` enables the complete prepress profile: PDF/X-4 with a FOGRA39
output intent, 300 DPI image preflight, mandatory embedded fonts, document
metadata, and explicit media, bleed, crop, and trim boxes.

```sh
cargo run -- render examples/business-card.json examples/people.csv \
  output/pdf/business-cards-print-ready.pdf \
  --print-ready \
  --summary output/jobs/business-cards-print-ready.json
```

The checks can also be selected independently with `--pdf-x x4`,
`--min-image-dpi <DPI>`, and `--require-embedded-fonts`. PDF/X output is
validated after serialization and rejected if its version, XMP declaration,
output intent, ICC profile, page boxes, or embedded fonts are incomplete.

Colors use one of three strict forms: `#RRGGBB`, `rgb(R, G, B)` with integer
components from 0 to 255, or `cmyk(C%, M%, Y%, K%)` with percentages from 0 to
100. Template metadata is configured under `document.metadata`; supported
fields are `title`, `author`, `subject`, `keywords`, and `identifier`. When no
identifier is supplied, Print Forge derives a stable one from the resolved
document. Identical inputs produce byte-identical PDFs.

The current renderer supports measured and wrapped absolute-positioned text,
embedded font families, local PNG/JPEG images, rectangles, and lines. Asset
paths are resolved relative to the template file. SVG, QR codes, and flow
elements such as stacks and tables remain explicit implementation errors.

## MVP roadmap

Work is ordered by product importance and implementation dependency. Complete
each priority before moving to the next unless an item is clearly independent.

### Completed foundation

- [x] Create the Rust workspace and renderer-independent crate boundaries.
- [x] Define the versioned JSON template schema and physical measurement units.
- [x] Load flat CSV datasets and nested JSON datasets.
- [x] Resolve template variables, including dotted paths such as
      `{{customer.name}}`.
- [x] Lower absolute text, rectangle, and line elements into draw commands.
- [x] Generate a valid PDF through the `printpdf` backend.
- [x] Provide CLI commands for validation, dataset inspection, and rendering.

### 1. Make incorrect output difficult

- [x] Add semantic template validation for document sizes, page counts, element
      bounds, font sizes, stroke widths, and table column widths.
- [x] Validate the supported `schema_version` and report migration guidance for
      incompatible templates.
- [x] Validate required fields against the selected dataset before rendering.
- [x] Include the dataset row, page, and element path in every rendering error.
- [x] Detect elements outside the page or bleed area and report actionable
      warnings.
- [x] Add integration tests for malformed templates, missing variables, empty
      datasets, and unsupported features.

### 2. Finish the essential absolute-layout elements

- [x] Add text measurement, wrapping, explicit line height, alignment, and
      overflow policies (`clip`, `shrink`, and `error`).
- [x] Load and embed external font files with regular, bold, italic, and
      bold-italic variants.
- [x] Render PNG and JPEG images from local paths.
- [x] Implement image `contain`, `cover`, and `stretch` behavior with clipping.
- [x] Resolve asset paths relative to the template file instead of the current
      working directory.
- [x] Add visual regression fixtures for text, images, rectangles, and lines.

### 3. Support real variable-data jobs

- [x] Render every dataset row rather than only the first row.
- [x] Support one combined multipage PDF and one-PDF-per-row output modes.
- [x] Add collision-safe output naming from a field such as
      `{{invoice_number}}`.
- [x] Add row ranges, record limits, and `--continue-on-error` CLI options.
- [x] Produce a machine-readable job summary containing successes, warnings,
      failures, output paths, and elapsed time.

### 4. Produce print-ready files

- [x] Apply bleed to page geometry and expose trim, bleed, and media boxes.
- [x] Add RGB and CMYK color models with strict color parsing.
- [x] Add an image-resolution preflight with configurable minimum DPI.
- [x] Verify that every required font is embedded before accepting a job.
- [x] Define a PDF/X conformance target and validate generated files against it.
- [x] Add document metadata and deterministic output for repeatable builds.

### 5. Add flow layout and pagination

- [ ] Introduce a measure/layout contract for elements.
- [ ] Implement vertical and horizontal `stack` layout with gaps and padding.
- [ ] Allow absolute and flow-layout regions on the same page.
- [ ] Add automatic page creation and explicit page breaks.
- [ ] Add reusable page headers and footers.
- [ ] Define keep-together, orphan, and overflow behavior.

### 6. Implement MVP tables

- [ ] Support fixed and percentage column widths.
- [ ] Support header rows, cell padding, borders, backgrounds, and text
      alignment.
- [ ] Calculate row heights from wrapped cell contents.
- [ ] Split tables across pages and repeat the header row.
- [ ] Add per-column value formatting for numbers, currency, and dates.
- [ ] Explicitly reject merged cells, nested tables, and arbitrary cell layouts
      for the MVP.

### 7. Add reusable composition and repetition

- [ ] Implement groups with translated child coordinates.
- [ ] Implement repeaters over nested JSON arrays.
- [ ] Support vertical, horizontal, and grid repeater layouts.
- [ ] Add item-level variable scope while retaining access to root job data.
- [ ] Use repeaters to generate a label-sheet and a simple product-catalog
      fixture.

### 8. Add high-value specialty elements

- [ ] Render SVG assets while preserving vector output.
- [ ] Generate QR codes with configurable error correction and quiet zones.
- [ ] Add Code 128 barcode support for labels, tickets, and inventory use cases.
- [ ] Preflight barcode and QR dimensions for reliable scanning.

### 9. Harden the CLI and library API

- [ ] Add structured exit codes and optional JSON output for automation.
- [ ] Add `--dry-run`, warning policies, and verbose diagnostic modes.
- [ ] Stream large datasets instead of loading every row into memory.
- [ ] Define stable public APIs for custom layout engines and renderers.
- [ ] Add benchmarks for large datasets, image-heavy pages, and long tables.
- [ ] Run formatting, Clippy, tests, and representative PDF renders in CI.

### 10. Document and package the MVP

- [ ] Publish a complete template-schema reference with working examples.
- [ ] Document coordinate systems, units, asset resolution, and supported fonts.
- [ ] Add end-to-end examples for business cards, labels, invoices, and product
      sheets.
- [ ] Document installation and release builds for macOS, Linux, and Windows.
- [ ] Define the compatibility and migration policy for future schema versions.

### MVP exit criteria

- [ ] A user can validate a template and dataset before starting a job.
- [ ] A user can generate business cards, labels, and multipage invoices from
      CSV or JSON without editing Rust code.
- [ ] Generated files pass the project's font, image-resolution, page-box, and
      PDF-conformance preflight checks.
- [ ] Failures identify the exact record and template element without silently
      producing incomplete output.
- [ ] The documented examples pass automated tests and visual PDF inspection.
