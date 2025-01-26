mod utils;

use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    fn alert(s: &str);
}

#[wasm_bindgen]
pub fn init(supergraph_sdl: &str) {
    alert(format!("Hello, wasm-lib: {}", supergraph_sdl).as_str());
}
