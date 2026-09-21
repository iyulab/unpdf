//! Parsing options and configuration.

use crate::render::PageSelection;

/// Options for parsing PDF documents.
#[derive(Debug, Clone)]
pub struct ParseOptions {
    /// Error handling mode
    pub error_mode: ErrorMode,

    /// Whether to extract the text content of each page.
    ///
    /// `true` by default. Setting it to `false` is "structure only": every page is still
    /// produced, carrying its number and dimensions, and none of its content blocks are
    /// built. It is an axis of its own -- orthogonal to [`Self::extract_resources`], so
    /// asking for structure does not decide anything about images.
    pub extract_text: bool,

    /// Whether to extract embedded resources (images, fonts).
    ///
    /// Default is `false` since 0.4.0 — large PDFs silently loading all
    /// images into memory was the largest peak-memory vector. Opt in via
    /// `.with_resources(true)` when images are needed.
    pub extract_resources: bool,

    /// Minimum pixel dimension for extracted images. Images whose width
    /// OR height falls below this threshold are dropped as decorative
    /// (logos, bullets, rule lines, tracking pixels). Set to 0 to keep
    /// every image. Default 64 — conservative cutoff for technical docs.
    pub min_image_dimension: u32,

    /// Whether to use parallel processing
    pub parallel: bool,

    /// Page selection (which pages to parse)
    pub pages: PageSelection,

    /// Password for encrypted documents
    pub password: Option<String>,

    /// Whether to drop an invisible OCR text layer whose text is not readable.
    ///
    /// Searchable scans carry the OCR result as invisible text over the page
    /// image. When the OCR recognised nothing real — a drawing, a stamp, a poor
    /// scan — that layer decodes to meaningless characters, which are worse than
    /// no text at all. Default `true`; set `false` to keep the raw layer.
    pub suppress_low_confidence_ocr: bool,

    /// VLM-based image understanding — text/table scans get structured extraction,
    /// other images get a context-aware description filled into `alt_text`.
    /// `None` (the default) leaves parsing unchanged; the AI endpoint is only
    /// ever contacted when this is `Some`.
    ///
    /// Setting this forces internal image-byte decoding for the pages/images the
    /// AI call needs, regardless of `extract_resources` — that flag still governs
    /// whether the resulting resource inventory is kept in the output.
    #[cfg(feature = "ai")]
    pub ai: Option<unparser_shared::ai::AiConfig>,
}

impl ParseOptions {
    /// Create new parse options with defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Keep or drop unreadable OCR text layers (default: drop).
    pub fn with_ocr_suppression(mut self, enabled: bool) -> Self {
        self.suppress_low_confidence_ocr = enabled;
        self
    }

    /// Set error mode.
    pub fn with_error_mode(mut self, mode: ErrorMode) -> Self {
        self.error_mode = mode;
        self
    }

    /// Enable lenient mode (skip invalid content).
    pub fn lenient(mut self) -> Self {
        self.error_mode = ErrorMode::Lenient;
        self
    }

    /// Extract the text content of each page, or leave it out ("structure only").
    pub fn with_text(mut self, extract: bool) -> Self {
        self.extract_text = extract;
        self
    }

    /// Enable or disable resource extraction.
    pub fn with_resources(mut self, extract: bool) -> Self {
        self.extract_resources = extract;
        self
    }

    /// Enable or disable parallel processing.
    pub fn with_parallel(mut self, parallel: bool) -> Self {
        self.parallel = parallel;
        self
    }

    /// Disable parallel processing.
    pub fn sequential(mut self) -> Self {
        self.parallel = false;
        self
    }

    /// Set page selection.
    pub fn with_pages(mut self, pages: PageSelection) -> Self {
        self.pages = pages;
        self
    }

    /// Set password for encrypted documents.
    pub fn with_password(mut self, password: impl Into<String>) -> Self {
        self.password = Some(password.into());
        self
    }

    /// Set the minimum image dimension (pixels). Images with width OR
    /// height below this value are dropped as decorative. `0` keeps all.
    pub fn with_min_image_dimension(mut self, min_px: u32) -> Self {
        self.min_image_dimension = min_px;
        self
    }

    /// Enable VLM-based image understanding, contacting the endpoint `config`
    /// describes. `None` (the default) leaves parsing unchanged.
    #[cfg(feature = "ai")]
    pub fn with_ai(mut self, config: unparser_shared::ai::AiConfig) -> Self {
        self.ai = Some(config);
        self
    }

    /// Whether image bytes must be decoded during parsing: the user-requested
    /// `extract_resources`, or forced on because `ai` needs image bytes to send.
    pub(crate) fn effective_extract_resources(&self) -> bool {
        #[cfg(feature = "ai")]
        {
            self.extract_resources || self.ai.is_some()
        }
        #[cfg(not(feature = "ai"))]
        {
            self.extract_resources
        }
    }
}

impl Default for ParseOptions {
    fn default() -> Self {
        Self {
            error_mode: ErrorMode::Lenient,
            extract_text: true,
            extract_resources: false,
            min_image_dimension: 64,
            parallel: true,
            pages: PageSelection::All,
            password: None,
            suppress_low_confidence_ocr: true,
            #[cfg(feature = "ai")]
            ai: None,
        }
    }
}

/// Error handling mode during parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ErrorMode {
    /// Fail on any error
    Strict,
    /// Skip invalid content and continue
    #[default]
    Lenient,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_options_builder() {
        let options = ParseOptions::new().lenient().with_text(false).sequential();

        assert_eq!(options.error_mode, ErrorMode::Lenient);
        assert!(!options.extract_text);
        assert!(!options.parallel);
    }

    #[test]
    fn test_default_min_image_dimension() {
        let options = ParseOptions::default();
        assert_eq!(options.min_image_dimension, 64);
    }

    #[test]
    fn test_with_min_image_dimension_override() {
        let o = ParseOptions::new().with_min_image_dimension(0);
        assert_eq!(o.min_image_dimension, 0);
        let o = ParseOptions::new().with_min_image_dimension(200);
        assert_eq!(o.min_image_dimension, 200);
    }

    #[cfg(feature = "ai")]
    #[test]
    fn test_ai_is_none_by_default() {
        let options = ParseOptions::default();
        assert!(options.ai.is_none());
    }

    #[cfg(feature = "ai")]
    #[test]
    fn test_with_ai_sets_config() {
        let config = unparser_shared::ai::AiConfig::new("https://example.test", "key", "model");
        let options = ParseOptions::new().with_ai(config);
        assert!(options.ai.is_some());
        assert_eq!(options.ai.unwrap().base_url, "https://example.test");
    }

    #[test]
    fn test_default_options() {
        let options = ParseOptions::default();
        assert_eq!(options.error_mode, ErrorMode::Lenient);
        assert!(options.parallel);
        // 0.4.0 breaking: default is now false
        assert!(!options.extract_resources);
    }
}
