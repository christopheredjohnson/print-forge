use std::{hint::black_box, path::PathBuf, time::Instant};

use print_forge_engine::{BasicLayoutEngine, LayoutOptions};
use print_forge_template::Template;
use serde_json::{Map, Value};

fn main() {
    let template: Template =
        serde_json::from_str(include_str!("../../../examples/table-invoice.json")).unwrap();
    let seed: Vec<Map<String, Value>> =
        serde_json::from_str(include_str!("../../../examples/table-invoice-data.json")).unwrap();
    let mut data = seed[0].clone();
    let row = data["items"].as_array().unwrap()[0].clone();
    data.insert("items".to_owned(), Value::Array(vec![row; 1_000]));
    let options = LayoutOptions {
        asset_base: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples"),
    };

    let started = Instant::now();
    for _ in 0..10 {
        black_box(
            BasicLayoutEngine
                .layout_with_options(&template, &data, &options)
                .unwrap(),
        );
    }
    println!(
        "layout 1,000-row table: {:?} total (10 iterations)",
        started.elapsed()
    );
}
