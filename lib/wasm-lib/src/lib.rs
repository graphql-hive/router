mod utils;

// use query_planner::{
//     operation_advisor::OperationAdvisor, parse_schema, supergraph::SupergraphMetadata,
// };
use wasm_bindgen::prelude::*;

extern crate web_sys;

// A macro to provide `println!(..)`-style syntax for `console.log` logging.
macro_rules! log {
    ( $( $t:tt )* ) => {
        web_sys::console::log_1(&format!( $( $t )* ).into());
    }
}

#[wasm_bindgen]
pub fn init_panic_hook() {
    console_error_panic_hook::set_once();
}

#[wasm_bindgen]
pub fn init(supergraph_sdl: &str) {
    log!("called");

    // let schema_sdl = parse_schema(supergraph_sdl);
    // let supergraph = SupergraphMetadata::new(&schema_sdl);
    // let advisor = OperationAdvisor::new(supergraph);
    log!("Hello, wasm-lib: {}", supergraph_sdl);
}
