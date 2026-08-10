use std::{hint::black_box, path::PathBuf, time::Instant};

use print_forge_engine::{BasicLayoutEngine, LayoutOptions};
use print_forge_pdf::{DocumentRenderer, PdfRenderer};
use print_forge_template::Template;
use serde_json::{Map, Value};

fn main() {
    let template: Template =
        serde_json::from_str(include_str!("../../../examples/absolute-layout.json")).unwrap();
    let rows: Vec<Map<String, Value>> =
        serde_json::from_str(include_str!("../../../examples/absolute-layout-data.json")).unwrap();
    let options = LayoutOptions {
        asset_base: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples"),
    };
    let mut document = BasicLayoutEngine
        .layout_with_options(&template, &rows[0], &options)
        .unwrap();
    let page = document.pages[0].clone();
    document.pages = vec![page; 24];

    let started = Instant::now();
    for _ in 0..5 {
        black_box(PdfRenderer.render(&document).unwrap());
    }
    println!(
        "render 24 image-heavy pages: {:?} total (5 iterations)",
        started.elapsed()
    );
}
