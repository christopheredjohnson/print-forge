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

## CLI

During development, run the CLI through Cargo as `cargo run -- <COMMAND>`.
After installing the binary, use the equivalent `print-forge <COMMAND>` form.
Every command supports `--help`; the root command also supports `--version`.

### Commands

| Command | Usage | Purpose |
| --- | --- | --- |
| `validate` | `validate <TEMPLATE> [--dataset <DATASET>]` | Validate a JSON template and, optionally, every CSV or JSON dataset row. |
| `inspect-data` | `inspect-data <DATASET>` | Parse a CSV or JSON dataset and report its row count. |
| `render` | `render <TEMPLATE> <DATASET> <OUTPUT> [OPTIONS]` | Render one combined PDF or one PDF per selected dataset row. |

For `render`, `<OUTPUT>` is a PDF path in combined mode and a directory in
separate mode.

### Render options

| Option | Default | Description |
| --- | --- | --- |
| `--output-mode <MODE>` | `combined` | Choose `combined` for one multipage PDF or `separate` for one PDF per row. |
| `--rows <START-END>` | All rows | Select a one-based, inclusive row range. |
| `--limit <COUNT>` | No limit | Limit the number of rows after applying `--rows`. |
| `--continue-on-error` | Off | Continue after row failures; the command still exits unsuccessfully if any row fails. |
| `--output-name <PATTERN>` | `row-{{row}}` | Set filenames in separate mode using fields, dotted paths, and the one-based `{{row}}` value. |
| `--summary <PATH>` | None | Write a JSON job summary with results, warnings, output paths, and elapsed time. |
| `--print-ready` | Off | Enable PDF/X-4, 300 DPI image preflight, and mandatory embedded fonts. |
| `--min-image-dpi <DPI>` | None | Reject images below the specified effective output resolution. |
| `--require-embedded-fonts` | Off | Reject built-in PDF fonts and require embedded external fonts. |
| `--pdf-x <x4>` | None | Generate and validate against the selected PDF/X target. |

Separate-mode names are made filesystem-safe. Existing or duplicate names get
a numeric suffix instead of being overwritten. `--print-ready` is the complete
prepress preset; `--pdf-x`, `--min-image-dpi`, and
`--require-embedded-fonts` can also be selected independently.

### Examples

```sh
# Discover the complete API.
cargo run -- --help
cargo run -- render --help

# Validate inputs or inspect a dataset.
cargo run -- validate examples/business-card.json --dataset examples/people.csv
cargo run -- inspect-data examples/people.csv

# Render every row into one combined PDF.
cargo run -- render examples/business-card.json examples/people.csv \
  output/pdf/business-cards.pdf \
  --summary output/jobs/business-cards.json

# Render selected rows into separate, safely named PDFs.
cargo run -- render examples/business-card.json examples/people.csv \
  output/pdf/business-cards \
  --output-mode separate \
  --output-name '{{last_name}}-{{first_name}}' \
  --rows 1-2 \
  --continue-on-error

# Apply the complete print-ready preset.
cargo run -- render examples/business-card.json examples/people.csv \
  output/pdf/business-cards-print-ready.pdf \
  --print-ready

# Render the flow-layout and pagination fixture.
cargo run -- render examples/flow-layout.json examples/flow-layout-data.json \
  output/pdf/flow-layout.pdf

# Render the paginated invoice-table fixture.
cargo run -- render examples/table-invoice.json examples/table-invoice-data.json \
  output/pdf/table-invoice.pdf
```

## Output and template behavior

### Print-ready PDFs

The print-ready preset produces PDF/X-4 with a FOGRA39 output intent, 300 DPI
image preflight, mandatory embedded fonts, document metadata, and explicit
media, bleed, crop, and trim boxes. PDF/X output is validated after
serialization and rejected if its version, XMP declaration, output intent, ICC
profile, page boxes, or embedded fonts are incomplete.

Colors use one of three strict forms: `#RRGGBB`, `rgb(R, G, B)` with integer
components from 0 to 255, or `cmyk(C%, M%, Y%, K%)` with percentages from 0 to
100. Template metadata is configured under `document.metadata`; supported
fields are `title`, `author`, `subject`, `keywords`, and `identifier`. When no
identifier is supplied, Print Forge derives a stable one from the resolved
document. Identical inputs produce byte-identical PDFs.

### Flow layout and pagination

A positioned `stack` defines a flow region that can coexist with absolute
elements on the same template page. Vertical stacks place children from top to
bottom and create continuation pages by default; horizontal stacks place one
row from left to right. Both support uniform `padding` and inter-item `gap`.

Within a flow stack, child `position.width` and `position.height` are size
hints; `position.x` and `position.y` are ignored. Vertical text may omit its
position and is measured from its resolved, wrapped content. Images,
rectangles, SVGs, and horizontal text require size hints. Coordinate-based
lines remain absolute elements.

`overflow: "paginate"` is the default. `overflow: "error"` rejects content
that needs an automatic continuation page. A `page_break` starts a continuation
explicitly in a vertical stack or between top-level page elements.
`keep_together: true` requires the entire stack to fit one page and cannot be
combined with an explicit break. `orphans` defaults to 1 and sets the minimum
number of flow items that must precede an automatic break.

Each page may declare absolute-positioned `header` and `footer` arrays. Their
commands are reused on every continuation page generated from that template
page. Flow items are currently atomic across page boundaries; text and images
are moved as units rather than split internally.

### MVP tables

A positioned top-level `table` reads an array of objects from its `source`.
Columns use either a physical fixed width or a percentage of the table region;
all resolved column widths must exactly fill that region. Cell text wraps and
determines row height before pagination. Rows remain atomic, continuation pages
repeat the table header, and the template page's reusable header and footer are
also preserved.

Tables support uniform cell padding, grid borders, header/body/alternating row
backgrounds, and per-column text alignment. Column formats include grouped
numbers, symbol-prefixed currency, and validated ISO `YYYY-MM-DD` dates rendered
as ISO, US, European, or long dates.

The MVP deliberately accepts scalar cell values only. Table rows must be
objects; merged cells, nested tables, arbitrary cell children, and custom cell
layouts are rejected rather than silently simplified.

The current renderer supports measured and wrapped text, embedded font
families, local PNG/JPEG images, rectangles, lines, stacks, tables, and
pagination. Asset paths are resolved relative to the template file. SVG, QR
codes, groups, and repeaters remain explicit implementation errors.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

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

- [x] Introduce a measure/layout contract for elements.
- [x] Implement vertical and horizontal `stack` layout with gaps and padding.
- [x] Allow absolute and flow-layout regions on the same page.
- [x] Add automatic page creation and explicit page breaks.
- [x] Add reusable page headers and footers.
- [x] Define keep-together, orphan, and overflow behavior.

### 6. Implement MVP tables

- [x] Support fixed and percentage column widths.
- [x] Support header rows, cell padding, borders, backgrounds, and text
      alignment.
- [x] Calculate row heights from wrapped cell contents.
- [x] Split tables across pages and repeat the header row.
- [x] Add per-column value formatting for numbers, currency, and dates.
- [x] Explicitly reject merged cells, nested tables, and arbitrary cell layouts
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
