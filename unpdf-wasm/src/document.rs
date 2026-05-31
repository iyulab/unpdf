use unpdf::render::{JsonFormat, RenderOptions};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct PdfDocument {
    #[allow(dead_code)]
    pub(crate) inner: unpdf::Document,
}

#[wasm_bindgen]
impl PdfDocument {
    #[wasm_bindgen(js_name = fromBytes)]
    pub fn from_bytes(data: &[u8]) -> Result<PdfDocument, JsValue> {
        unpdf::parse_bytes(data)
            .map(|inner| PdfDocument { inner })
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    #[wasm_bindgen(js_name = toMarkdown)]
    pub fn to_markdown(&self) -> Result<String, JsValue> {
        let opts = RenderOptions::default();
        unpdf::render::to_markdown(&self.inner, &opts)
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    #[wasm_bindgen(js_name = toText)]
    pub fn to_text(&self) -> Result<String, JsValue> {
        let opts = RenderOptions::default();
        unpdf::render::to_text(&self.inner, &opts).map_err(|e| JsValue::from_str(&e.to_string()))
    }

    #[wasm_bindgen(js_name = toJson)]
    pub fn to_json(&self) -> Result<String, JsValue> {
        unpdf::render::to_json(&self.inner, JsonFormat::Compact)
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    #[wasm_bindgen(js_name = pageCount)]
    pub fn page_count(&self) -> u32 {
        self.inner.page_count()
    }

    pub fn metadata(&self) -> Result<String, JsValue> {
        serde_json::to_string(&self.inner.metadata).map_err(|e| JsValue::from_str(&e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_node_experimental);

    const MINIMAL_PDF: &[u8] = b"%PDF-1.4\n\
        1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n\
        2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n\
        3 0 obj<</Type/Page/MediaBox[0 0 612 792]>>endobj\n\
        xref\n\
        0 4\n\
        0000000000 65535 f \n\
        0000000009 00000 n \n\
        0000000058 00000 n \n\
        0000000115 00000 n \n\
        trailer<</Size 4/Root 1 0 R>>\n\
        startxref\n\
        190\n\
        %%EOF";

    #[wasm_bindgen_test]
    fn test_page_count() {
        let doc = PdfDocument::from_bytes(MINIMAL_PDF).unwrap();
        assert_eq!(doc.page_count(), 1);
    }

    #[wasm_bindgen_test]
    fn test_to_text_returns_string() {
        let doc = PdfDocument::from_bytes(MINIMAL_PDF).unwrap();
        let text = doc.to_text().unwrap();
        // Any string is valid (empty PDF has no text content)
        let _ = text;
    }

    #[wasm_bindgen_test]
    fn test_to_markdown_returns_string() {
        let doc = PdfDocument::from_bytes(MINIMAL_PDF).unwrap();
        let md = doc.to_markdown().unwrap();
        let _ = md;
    }

    #[wasm_bindgen_test]
    fn test_to_json_is_valid_json() {
        let doc = PdfDocument::from_bytes(MINIMAL_PDF).unwrap();
        let json = doc.to_json().unwrap();
        let trimmed = json.trim();
        assert!(trimmed.starts_with('{') || trimmed.starts_with('['));
    }

    #[wasm_bindgen_test]
    fn test_invalid_bytes_returns_error() {
        let result = PdfDocument::from_bytes(b"not a pdf");
        assert!(result.is_err());
    }
}
