//! Rendering options and configuration.

use super::CleanupOptions;
use std::ops::RangeInclusive;
use std::path::PathBuf;

/// Options for rendering document content.
#[derive(Debug, Clone)]
pub struct RenderOptions {
    /// Directory to save extracted images
    pub image_dir: Option<PathBuf>,

    /// Prefix for image paths in output (e.g., "./images/")
    pub image_path_prefix: String,

    /// How to render complex tables
    pub table_fallback: TableFallback,

    /// Maximum heading level (1-6)
    pub max_heading_level: u8,

    /// Include YAML frontmatter with metadata
    pub include_frontmatter: bool,

    /// Preserve line breaks from source
    pub preserve_line_breaks: bool,

    /// Character to use for unordered list markers
    pub list_marker: char,

    /// Escape special Markdown characters
    pub escape_special_chars: bool,

    /// Text cleanup options
    pub cleanup: Option<CleanupOptions>,

    /// Page selection
    pub page_selection: PageSelection,

    /// Heading detection configuration
    pub heading_config: Option<HeadingConfig>,

    /// Width for wrapping long lines (0 = no wrap)
    pub line_width: u32,

    /// Collect extraction statistics during rendering
    pub collect_stats: bool,
}

impl RenderOptions {
    /// Create new render options with defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the image directory.
    pub fn with_image_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.image_dir = Some(dir.into());
        self
    }

    /// Set the image path prefix.
    pub fn with_image_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.image_path_prefix = prefix.into();
        self
    }

    /// Set the table fallback mode.
    pub fn with_table_fallback(mut self, fallback: TableFallback) -> Self {
        self.table_fallback = fallback;
        self
    }

    /// Set the maximum heading level.
    pub fn with_max_heading(mut self, level: u8) -> Self {
        self.max_heading_level = level.clamp(1, 6);
        self
    }

    /// Enable or disable frontmatter.
    pub fn with_frontmatter(mut self, include: bool) -> Self {
        self.include_frontmatter = include;
        self
    }

    /// Enable or disable line break preservation.
    pub fn with_line_breaks(mut self, preserve: bool) -> Self {
        self.preserve_line_breaks = preserve;
        self
    }

    /// Set the list marker character.
    pub fn with_list_marker(mut self, marker: char) -> Self {
        self.list_marker = marker;
        self
    }

    /// Set cleanup options.
    pub fn with_cleanup(mut self, cleanup: CleanupOptions) -> Self {
        self.cleanup = Some(cleanup);
        self
    }

    /// Set cleanup preset.
    pub fn with_cleanup_preset(mut self, preset: super::CleanupPreset) -> Self {
        self.cleanup = Some(CleanupOptions::from_preset(preset));
        self
    }

    /// Set page selection.
    pub fn with_pages(mut self, selection: PageSelection) -> Self {
        self.page_selection = selection;
        self
    }

    /// Set specific page range.
    pub fn with_page_range(mut self, range: RangeInclusive<u32>) -> Self {
        self.page_selection = PageSelection::Range(range);
        self
    }

    /// Set specific pages.
    pub fn with_page_list(mut self, pages: Vec<u32>) -> Self {
        self.page_selection = PageSelection::Pages(pages);
        self
    }

    /// Set heading configuration.
    pub fn with_heading_config(mut self, config: HeadingConfig) -> Self {
        self.heading_config = Some(config);
        self
    }

    /// Enable heading analysis with default config.
    pub fn with_heading_analysis(mut self) -> Self {
        self.heading_config = Some(HeadingConfig::default());
        self
    }

    /// Set line width for wrapping.
    pub fn with_line_width(mut self, width: u32) -> Self {
        self.line_width = width;
        self
    }
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            image_dir: None,
            image_path_prefix: String::new(),
            table_fallback: TableFallback::Markdown,
            max_heading_level: 6,
            include_frontmatter: false,
            preserve_line_breaks: false,
            list_marker: '-',
            escape_special_chars: true,
            cleanup: None,
            page_selection: PageSelection::All,
            heading_config: None,
            line_width: 0,
            collect_stats: false,
        }
    }
}

impl RenderOptions {
    /// Enable statistics collection during rendering.
    pub fn with_stats(mut self, collect: bool) -> Self {
        self.collect_stats = collect;
        self
    }
}

/// How to render complex tables that can't be expressed in simple Markdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TableFallback {
    /// Use standard Markdown table syntax
    #[default]
    Markdown,
    /// Use HTML table tags for complex tables
    Html,
    /// Use ASCII art tables
    Ascii,
}

/// Page selection for rendering.
#[derive(Debug, Clone, Default)]
pub enum PageSelection {
    /// Render all pages
    #[default]
    All,
    /// Render a range of pages (inclusive, 1-indexed)
    Range(RangeInclusive<u32>),
    /// Render specific pages (1-indexed)
    Pages(Vec<u32>),
}

impl PageSelection {
    /// Check if a page number should be included.
    pub fn includes(&self, page: u32) -> bool {
        match self {
            PageSelection::All => true,
            PageSelection::Range(range) => range.contains(&page),
            PageSelection::Pages(pages) => pages.contains(&page),
        }
    }

    /// Parse a page selection string (e.g., "1-10", "1,3,5,7-10").
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim();

        if s.is_empty() || s == "all" {
            return Ok(PageSelection::All);
        }

        // Check for simple range (e.g., "1-10")
        if let Some((start, end)) = s.split_once('-') {
            if !start.contains(',') && !end.contains(',') {
                let start: u32 = start.trim().parse().map_err(|_| "Invalid start page")?;
                let end: u32 = end.trim().parse().map_err(|_| "Invalid end page")?;
                return Ok(PageSelection::Range(start..=end));
            }
        }

        // Parse comma-separated list with possible ranges
        let mut pages = Vec::new();
        for part in s.split(',') {
            let part = part.trim();
            if let Some((start, end)) = part.split_once('-') {
                let start: u32 = start.trim().parse().map_err(|_| "Invalid page number")?;
                let end: u32 = end.trim().parse().map_err(|_| "Invalid page number")?;
                for p in start..=end {
                    if !pages.contains(&p) {
                        pages.push(p);
                    }
                }
            } else {
                let p: u32 = part.parse().map_err(|_| "Invalid page number")?;
                if !pages.contains(&p) {
                    pages.push(p);
                }
            }
        }

        pages.sort();
        Ok(PageSelection::Pages(pages))
    }
}

/// Configuration for heading detection.
#[derive(Debug, Clone)]
pub struct HeadingConfig {
    /// Minimum font size ratio to body text for H1
    pub h1_min_ratio: f32,

    /// Minimum font size ratio to body text for H2
    pub h2_min_ratio: f32,

    /// Whether to detect headings from bold/large text
    pub detect_from_style: bool,

    /// Whether to detect headings from outline structure
    pub use_outline: bool,

    /// Korean-specific heading patterns (e.g., "제1장", "1.", "가.")
    pub korean_patterns: bool,
}

impl Default for HeadingConfig {
    fn default() -> Self {
        Self {
            h1_min_ratio: 1.5,
            h2_min_ratio: 1.3,
            detect_from_style: true,
            use_outline: true,
            korean_patterns: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_options_builder() {
        let options = RenderOptions::new()
            .with_frontmatter(true)
            .with_max_heading(3)
            .with_table_fallback(TableFallback::Html);

        assert!(options.include_frontmatter);
        assert_eq!(options.max_heading_level, 3);
        assert_eq!(options.table_fallback, TableFallback::Html);
    }

    #[test]
    fn test_page_selection_includes() {
        let all = PageSelection::All;
        assert!(all.includes(1));
        assert!(all.includes(100));

        let range = PageSelection::Range(5..=10);
        assert!(!range.includes(4));
        assert!(range.includes(5));
        assert!(range.includes(10));
        assert!(!range.includes(11));

        let pages = PageSelection::Pages(vec![1, 3, 5, 7]);
        assert!(pages.includes(1));
        assert!(!pages.includes(2));
        assert!(pages.includes(3));
    }

    #[test]
    fn test_page_selection_parse() {
        let all = PageSelection::parse("all").unwrap();
        assert!(matches!(all, PageSelection::All));

        let range = PageSelection::parse("1-10").unwrap();
        assert!(matches!(range, PageSelection::Range(_)));

        let mixed = PageSelection::parse("1,3,5-7,10").unwrap();
        if let PageSelection::Pages(pages) = mixed {
            assert_eq!(pages, vec![1, 3, 5, 6, 7, 10]);
        } else {
            panic!("Expected Pages variant");
        }
    }
}
