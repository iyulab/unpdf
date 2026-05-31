mod document;
mod options;

pub use document::PdfDocument;
pub use options::ParseOptions;

use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn parse(data: &[u8]) -> Result<PdfDocument, JsValue> {
    unpdf::parse_bytes(data)
        .map(|inner| PdfDocument { inner })
        .map_err(|e| JsValue::from_str(&e.to_string()))
}

#[wasm_bindgen(js_name = parseWithOptions)]
pub fn parse_with_options(data: &[u8], opts: &ParseOptions) -> Result<PdfDocument, JsValue> {
    unpdf::parse_bytes_with_options(data, opts.inner.clone())
        .map(|inner| PdfDocument { inner })
        .map_err(|e| JsValue::from_str(&e.to_string()))
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
0000000052 00000 n \n\
0000000101 00000 n \n\
trailer<</Size 4/Root 1 0 R>>\n\
startxref\n\
151\n\
%%EOF";

    #[wasm_bindgen_test]
    fn test_parse_returns_document() {
        let doc = parse(MINIMAL_PDF).unwrap();
        assert_eq!(doc.page_count(), 1);
    }

    #[wasm_bindgen_test]
    fn test_parse_with_options_lenient() {
        let opts = ParseOptions::new().lenient();
        let doc = parse_with_options(MINIMAL_PDF, &opts).unwrap();
        assert_eq!(doc.page_count(), 1);
    }

    #[wasm_bindgen_test]
    fn test_parse_invalid_returns_error() {
        let result = parse(b"garbage data");
        assert!(result.is_err());
    }
}
