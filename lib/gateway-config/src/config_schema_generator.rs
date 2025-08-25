use std::io::Write;

use gateway_config::HiveRouterConfig;
use schemars::generate::SchemaSettings;

pub fn main() {
    let generator = SchemaSettings::draft2020_12()
        .with(|s| {
            s.inline_subschemas = true;
        })
        .into_generator();
    let schema = generator.into_root_schema_for::<HiveRouterConfig>();
    let schema_str = serde_json::to_string_pretty(&schema).unwrap();
    let args = std::env::args().collect::<Vec<String>>();

    match args.get(1) {
        Some(output_file) => {
            let mut file = std::fs::File::create(output_file).unwrap();
            file.write_all(schema_str.as_bytes()).unwrap();

            println!("JSON Schema written to {}", output_file);
        }
        None => {
            println!("{}", schema_str);
        }
    }
}
