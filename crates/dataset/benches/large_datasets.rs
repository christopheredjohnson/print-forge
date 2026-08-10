use std::{hint::black_box, ops::ControlFlow, time::Instant};

use print_forge_dataset::{visit_csv_reader, visit_json_reader};

const ROWS: usize = 50_000;

fn main() {
    let mut csv = String::from("id,name,amount\n");
    let mut json = String::from("[");
    for index in 0..ROWS {
        csv.push_str(&format!("{index},Customer {index},123.45\n"));
        if index > 0 {
            json.push(',');
        }
        json.push_str(&format!(
            r#"{{"id":{index},"name":"Customer {index}","amount":123.45}}"#
        ));
    }
    json.push(']');

    measure("stream 50k CSV rows", || {
        visit_csv_reader(csv.as_bytes(), |_, row| {
            black_box(row);
            ControlFlow::Continue(())
        })
        .unwrap();
    });
    measure("stream 50k JSON rows", || {
        visit_json_reader(json.as_bytes(), |_, row| {
            black_box(row);
            ControlFlow::Continue(())
        })
        .unwrap();
    });
}

fn measure(name: &str, mut operation: impl FnMut()) {
    let started = Instant::now();
    for _ in 0..5 {
        operation();
    }
    println!("{name}: {:?} total (5 iterations)", started.elapsed());
}
