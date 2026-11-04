use unpdf::render::PageSelection;
use wasm_bindgen::prelude::*;

#[derive(Default)]
#[wasm_bindgen]
pub struct ParseOptions {
    #[allow(dead_code)]
    pub(crate) inner: unpdf::ParseOptions,
}

#[wasm_bindgen]
impl ParseOptions {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn lenient(mut self) -> Self {
        self.inner = self.inner.lenient();
        self
    }

    #[wasm_bindgen(js_name = withPassword)]
    pub fn with_password(mut self, password: &str) -> Self {
        self.inner = self.inner.with_password(password);
        self
    }

    #[wasm_bindgen(js_name = withPages)]
    pub fn with_pages(mut self, from: u32, to: u32) -> Self {
        self.inner = self.inner.with_pages(PageSelection::Range(from..=to));
        self
    }

    /// Enable or disable resource (embedded image) extraction. Off by default —
    /// large PDFs loading every embedded image into memory is the largest peak-
    /// memory vector in this library. See `withMinImageDimension` to also tune
    /// which images count.
    #[wasm_bindgen(js_name = withResources)]
    pub fn with_resources(mut self, extract: bool) -> Self {
        self.inner = self.inner.with_resources(extract);
        self
    }

    /// Leave a page's running heads and feet (a title, volume or date in the top and bottom
    /// margin) out of the text, judged from that page's own layout; a footnote is kept. Off by
    /// default. It is for a pipeline that handles one page at a time, and costs accuracy on
    /// documents whose reference keeps the running heads.
    #[wasm_bindgen(js_name = withOmitRunningHeads)]
    pub fn with_omit_running_heads(mut self, enabled: bool) -> Self {
        self.inner = self.inner.with_omit_running_heads(enabled);
        self
    }

    /// Minimum pixel width/height for an extracted image to be kept — images
    /// below this on either axis are dropped as decorative (logos, rule lines,
    /// tracking pixels). Default 64; `0` keeps every image regardless of size.
    #[wasm_bindgen(js_name = withMinImageDimension)]
    pub fn with_min_image_dimension(mut self, min_px: u32) -> Self {
        self.inner = self.inner.with_min_image_dimension(min_px);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_node_experimental);

    #[wasm_bindgen_test]
    fn test_parse_options_new() {
        let opts = ParseOptions::new();
        assert!(matches!(
            opts.inner.error_mode,
            unpdf::parser::ErrorMode::Lenient
        ));
    }

    #[wasm_bindgen_test]
    fn test_parse_options_with_password() {
        let opts = ParseOptions::new().with_password("test");
        assert_eq!(opts.inner.password, Some("test".to_string()));
    }

    #[wasm_bindgen_test]
    fn test_parse_options_pages() {
        let opts = ParseOptions::new().with_pages(1, 3);
        assert!(matches!(
            opts.inner.pages,
            unpdf::render::PageSelection::Range(_)
        ));
    }

    #[wasm_bindgen_test]
    fn test_parse_options_with_omit_running_heads() {
        assert!(!ParseOptions::new().inner.omit_running_heads);
        let opts = ParseOptions::new().with_omit_running_heads(true);
        assert!(opts.inner.omit_running_heads);
    }

    #[wasm_bindgen_test]
    fn test_parse_options_with_resources() {
        let opts = ParseOptions::new().with_resources(true);
        assert!(opts.inner.extract_resources);
    }

    #[wasm_bindgen_test]
    fn test_parse_options_with_min_image_dimension() {
        let opts = ParseOptions::new().with_min_image_dimension(0);
        assert_eq!(opts.inner.min_image_dimension, 0);
    }
}
