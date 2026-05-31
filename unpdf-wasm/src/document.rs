use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct PdfDocument {
    pub(crate) inner: unpdf::Document,
}
