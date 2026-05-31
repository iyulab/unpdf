use unpdf::render::PageSelection;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct ParseOptions {
    pub(crate) inner: unpdf::ParseOptions,
}

#[wasm_bindgen]
impl ParseOptions {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            inner: unpdf::ParseOptions::default(),
        }
    }

    pub fn lenient(mut self) -> Self {
        self.inner = self.inner.lenient();
        self
    }

    #[wasm_bindgen(js_name = textOnly)]
    pub fn text_only(mut self) -> Self {
        self.inner = self.inner.text_only();
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
}
