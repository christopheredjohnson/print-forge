# Large dataset example

This project renders 10,000 variable-data fulfillment labels from a streamed
CSV dataset. Every row changes the recipient, address, service level, tracking
barcode, and order-detail QR code. The template uses only the PDF engine's
bundled fonts and has no external assets.

Inspect or validate the complete dataset without generating output:

```sh
cargo run -- inspect-data examples/large-dataset/data.csv
cargo run -- validate examples/large-dataset \
  --dataset examples/large-dataset/data.csv
```

Render a small range while editing the template:

```sh
cargo run -- render examples/large-dataset \
  examples/large-dataset/data.csv \
  output/pdf/large-dataset-sample \
  --output-mode separate \
  --output-name '{{record_id}}' \
  --rows 1-25
```

Render the complete job by removing `--rows 1-25`. Separate mode streams the
dataset and retains only the current row's resolved document, which bounds
memory use; expect 10,000 PDF files and substantial disk activity. Combined
mode produces one 10,000-page PDF and holds resolved pages until final assembly.

Regenerate the committed fixture, or create a differently sized dataset:

```sh
examples/large-dataset/generate-data.sh
examples/large-dataset/generate-data.sh /tmp/print-forge-50k.csv 50000
```
