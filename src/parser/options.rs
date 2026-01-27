//! Parsing options and configuration.

use crate::render::PageSelection;

/// Options for parsing PDF documents.
#[derive(Debug, Clone)]
pub struct ParseOptions {
    /// Error handling mode
    pub error_mode: ErrorMode,

    /// What to extract from the document
    pub extract_mode: ExtractMode,

    /// Memory limit in MB (0 = unlimited)
    pub memory_limit_mb: u32,

    /// Whether to extract embedded resources (images, fonts)
    pub extract_resources: bool,

    /// Whether to use parallel processing
    pub parallel: bool,

    /// Page selection (which pages to parse)
    pub pages: PageSelection,

    /// Password for encrypted documents
    pub password: Option<String>,
}

impl ParseOptions {
    /// Create new parse options with defaults.
    pub fn new() -> Self {
        Self::default()
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

    /// Set extract mode.
    pub fn with_extract_mode(mut self, mode: ExtractMode) -> Self {
        self.extract_mode = mode;
        self
    }

    /// Extract text only.
    pub fn text_only(mut self) -> Self {
        self.extract_mode = ExtractMode::TextOnly;
        self
    }

    /// Set memory limit in MB.
    pub fn with_memory_limit(mut self, mb: u32) -> Self {
        self.memory_limit_mb = mb;
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
}

impl Default for ParseOptions {
    fn default() -> Self {
        Self {
            error_mode: ErrorMode::Strict,
            extract_mode: ExtractMode::Full,
            memory_limit_mb: 0,
            extract_resources: true,
            parallel: true,
            pages: PageSelection::All,
            password: None,
        }
    }
}

/// Error handling mode during parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ErrorMode {
    /// Fail on any error
    #[default]
    Strict,
    /// Skip invalid content and continue
    Lenient,
}

/// What content to extract from the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExtractMode {
    /// Extract everything (text, structure, resources)
    #[default]
    Full,
    /// Extract text content only
    TextOnly,
    /// Extract structure only (no text content)
    StructureOnly,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_options_builder() {
        let options = ParseOptions::new()
            .lenient()
            .text_only()
            .with_memory_limit(512)
            .sequential();

        assert_eq!(options.error_mode, ErrorMode::Lenient);
        assert_eq!(options.extract_mode, ExtractMode::TextOnly);
        assert_eq!(options.memory_limit_mb, 512);
        assert!(!options.parallel);
    }

    #[test]
    fn test_default_options() {
        let options = ParseOptions::default();
        assert_eq!(options.error_mode, ErrorMode::Strict);
        assert!(options.parallel);
        assert!(options.extract_resources);
    }
}
