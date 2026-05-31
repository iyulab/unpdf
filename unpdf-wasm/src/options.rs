use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct ParseOptions {
    pub(crate) inner: unpdf::ParseOptions,
}
