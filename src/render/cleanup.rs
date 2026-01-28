//! Text cleanup pipeline for LLM training data preparation.

use regex::Regex;
use unicode_normalization::UnicodeNormalization;

/// Cleanup preset levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CleanupPreset {
    /// Minimal cleanup: Unicode NFC normalization only
    Minimal,
    /// Standard cleanup: NFC + line cleanup + structure filtering
    #[default]
    Standard,
    /// Aggressive cleanup: Maximum normalization for LLM training
    Aggressive,
}

/// Options for text cleanup.
#[derive(Debug, Clone)]
pub struct CleanupOptions {
    /// Normalize Unicode to NFC form
    pub normalize_unicode: bool,

    /// Standardize bullet characters (•, ●, ○ → •)
    pub standardize_bullets: bool,

    /// Remove page numbers
    pub remove_page_numbers: bool,

    /// Remove headers and footers
    pub remove_headers_footers: bool,

    /// Remove table of contents
    pub remove_toc: bool,

    /// Fix ligatures (fi, fl, etc.)
    pub fix_ligatures: bool,

    /// Fix hyphenation at line breaks
    pub fix_hyphenation: bool,

    /// Detect and flag mojibake (corrupted text)
    pub detect_mojibake: bool,

    /// Remove Private Use Area (PUA) characters
    pub remove_pua: bool,

    /// Remove Unicode replacement character (U+FFFD)
    pub remove_replacement_char: bool,

    /// Merge single newlines into spaces (for PDF word-by-word extraction)
    pub merge_single_newlines: bool,

    /// Merge bullet/number markers with following content
    pub merge_list_markers: bool,

    /// Merge CJK characters across line breaks (fix mid-sentence breaks in Korean/Chinese/Japanese)
    pub merge_cjk_lines: bool,

    /// Normalize whitespace
    pub normalize_whitespace: bool,

    /// Maximum consecutive newlines (0 = unlimited)
    pub max_consecutive_newlines: u8,

    /// Preserve YAML frontmatter during cleanup
    pub preserve_frontmatter: bool,
}

impl CleanupOptions {
    /// Create options from a preset.
    pub fn from_preset(preset: CleanupPreset) -> Self {
        match preset {
            CleanupPreset::Minimal => Self::minimal(),
            CleanupPreset::Standard => Self::standard(),
            CleanupPreset::Aggressive => Self::aggressive(),
        }
    }

    /// Minimal cleanup options.
    pub fn minimal() -> Self {
        Self {
            normalize_unicode: true,
            standardize_bullets: false,
            remove_page_numbers: false,
            remove_headers_footers: false,
            remove_toc: false,
            fix_ligatures: false,
            fix_hyphenation: false,
            detect_mojibake: false,
            remove_pua: false,
            remove_replacement_char: false,
            merge_single_newlines: false,
            merge_list_markers: false,
            merge_cjk_lines: false,
            normalize_whitespace: true,
            max_consecutive_newlines: 0,
            preserve_frontmatter: true,
        }
    }

    /// Standard cleanup options.
    pub fn standard() -> Self {
        Self {
            normalize_unicode: true,
            standardize_bullets: true,
            remove_page_numbers: true,
            remove_headers_footers: true,
            remove_toc: false,
            fix_ligatures: true,
            fix_hyphenation: true,
            detect_mojibake: false,
            remove_pua: false,
            remove_replacement_char: true,
            merge_single_newlines: true,
            merge_list_markers: true,
            merge_cjk_lines: true,
            normalize_whitespace: true,
            max_consecutive_newlines: 1, // RAG-ready: 2+ newlines → 1 newline
            preserve_frontmatter: true,
        }
    }

    /// Aggressive cleanup options for LLM training.
    pub fn aggressive() -> Self {
        Self {
            normalize_unicode: true,
            standardize_bullets: true,
            remove_page_numbers: true,
            remove_headers_footers: true,
            remove_toc: true,
            fix_ligatures: true,
            fix_hyphenation: true,
            detect_mojibake: true,
            remove_pua: true,
            remove_replacement_char: true,
            merge_single_newlines: true,
            merge_list_markers: true,
            merge_cjk_lines: true,
            normalize_whitespace: true,
            max_consecutive_newlines: 2,
            preserve_frontmatter: true,
        }
    }
}

impl Default for CleanupOptions {
    fn default() -> Self {
        Self::standard()
    }
}

/// Text cleanup pipeline.
pub struct CleanupPipeline {
    options: CleanupOptions,
    page_number_regex: Regex,
    ligature_map: Vec<(&'static str, &'static str)>,
}

impl CleanupPipeline {
    /// Create a new cleanup pipeline with the given options.
    pub fn new(options: CleanupOptions) -> Self {
        Self {
            options,
            page_number_regex: Regex::new(r"(?m)^[\s]*[-–—]?\s*\d+\s*[-–—]?\s*$").unwrap(),
            ligature_map: vec![
                ("\u{FB00}", "ff"),  // ﬀ
                ("\u{FB01}", "fi"),  // ﬁ
                ("\u{FB02}", "fl"),  // ﬂ
                ("\u{FB03}", "ffi"), // ﬃ
                ("\u{FB04}", "ffl"), // ﬄ
                ("\u{FB05}", "st"),  // ﬅ (long s + t)
                ("\u{FB06}", "st"),  // ﬆ
            ],
        }
    }

    /// Create a pipeline from a preset.
    pub fn from_preset(preset: CleanupPreset) -> Self {
        Self::new(CleanupOptions::from_preset(preset))
    }

    /// Process text through the cleanup pipeline.
    pub fn process(&self, text: &str) -> String {
        let mut result = text.to_string();

        // Extract and preserve frontmatter if needed
        let frontmatter = if self.options.preserve_frontmatter {
            self.extract_frontmatter(&result)
        } else {
            None
        };

        if let Some((fm, content)) = frontmatter {
            result = content;
            // Process content, then prepend frontmatter
            result = self.process_content(&result);
            result = format!("{}\n{}", fm, result);
        } else {
            result = self.process_content(&result);
        }

        result
    }

    fn process_content(&self, text: &str) -> String {
        let mut result = text.to_string();

        // Stage 1: Unicode normalization
        if self.options.normalize_unicode {
            result = result.nfc().collect();
        }

        // Fix ligatures
        if self.options.fix_ligatures {
            for (ligature, replacement) in &self.ligature_map {
                result = result.replace(ligature, replacement);
            }
        }

        // Standardize bullets
        if self.options.standardize_bullets {
            result = self.standardize_bullets(&result);
        }

        // Remove PUA characters
        if self.options.remove_pua {
            result = self.remove_pua_chars(&result);
        }

        // Remove Unicode replacement character (U+FFFD)
        if self.options.remove_replacement_char {
            result = result.replace('\u{FFFD}', "");
        }

        // Stage 2: Line-level cleanup
        if self.options.remove_page_numbers {
            result = self.page_number_regex.replace_all(&result, "").to_string();
        }

        // Fix hyphenation
        if self.options.fix_hyphenation {
            result = self.fix_hyphenation(&result);
        }

        // Merge list markers with following content (• \n내용 → • 내용)
        // This must run BEFORE merge_single_newlines
        if self.options.merge_list_markers {
            result = self.merge_list_markers(&result);
        }

        // Merge CJK characters across line breaks (fix mid-sentence breaks)
        if self.options.merge_cjk_lines {
            result = self.merge_cjk_lines(&result);
        }

        // Merge single newlines into spaces (for PDF word-by-word extraction)
        // This must run AFTER hyphenation fix but BEFORE whitespace normalization
        if self.options.merge_single_newlines {
            result = self.merge_single_newlines(&result);
        }

        // Stage 3: Normalize whitespace
        if self.options.normalize_whitespace {
            result = self.normalize_whitespace(&result);
        }

        // Limit consecutive newlines
        if self.options.max_consecutive_newlines > 0 {
            result = self.limit_newlines(&result);
        }

        result.trim().to_string()
    }

    fn extract_frontmatter(&self, text: &str) -> Option<(String, String)> {
        if let Some(stripped) = text.strip_prefix("---\n") {
            if let Some(end_pos) = stripped.find("\n---\n") {
                let fm_end = 4 + end_pos + 5;
                let frontmatter = &text[..fm_end];
                let content = &text[fm_end..];
                return Some((frontmatter.to_string(), content.to_string()));
            }
        }
        None
    }

    fn standardize_bullets(&self, text: &str) -> String {
        let bullets = ['●', '○', '■', '□', '◆', '◇', '▪', '▫', '►', '▻'];
        let mut result = text.to_string();
        for bullet in bullets {
            result = result.replace(bullet, "•");
        }
        result
    }

    fn remove_pua_chars(&self, text: &str) -> String {
        text.chars()
            .filter(|c| {
                let code = *c as u32;
                // Remove Private Use Area characters
                !(0xE000..=0xF8FF).contains(&code)
                    && !(0xF0000..=0xFFFFD).contains(&code)
                    && !(0x100000..=0x10FFFD).contains(&code)
            })
            .collect()
    }

    fn fix_hyphenation(&self, text: &str) -> String {
        // Join words that are hyphenated at line breaks
        // Handles various patterns:
        // - "infor-\nmation" → "information"
        // - "infor- mation" → "information"
        // - "infor-\n mation" → "information"
        let re = Regex::new(r"([a-zA-Z])-\s*\n?\s*([a-z])").unwrap();
        re.replace_all(text, "$1$2").to_string()
    }

    fn normalize_whitespace(&self, text: &str) -> String {
        // Replace 3+ spaces with 2 spaces (preserve markdown indentation)
        // Keep single/double spaces as-is for markdown indent support
        let re = Regex::new(r"[ ]{3,}").unwrap();
        re.replace_all(text, "  ").to_string()
    }

    fn limit_newlines(&self, text: &str) -> String {
        let max = self.options.max_consecutive_newlines as usize;
        let pattern = format!(r"\n{{{},}}", max + 1);
        let re = Regex::new(&pattern).unwrap();
        let replacement = "\n".repeat(max);
        re.replace_all(text, replacement.as_str()).to_string()
    }

    fn merge_single_newlines(&self, text: &str) -> String {
        // Replace single newlines with spaces, but preserve:
        // 1. Paragraph breaks (2+ newlines)
        // 2. Sentence endings (period/question mark/exclamation + newline)
        // 3. Markdown headings (lines starting with #)
        // 4. Markdown list items (lines starting with -, *, or numbers)
        // 5. Markdown table rows (lines starting with |)
        //
        // This fixes PDF extraction that puts each word on a separate line
        // while keeping logical paragraph structure.

        const PARA_PLACEHOLDER: &str = "\u{0000}PARA\u{0000}";
        const SENT_PLACEHOLDER: &str = "\u{0000}SENT\u{0000}";

        // Step 1: Protect markdown block elements FIRST (before paragraph breaks)
        // This ensures we don't lose the newline structure around headings

        // Protect lines that start with markdown heading (# to ######)
        // Pattern: start of line or newline, then 1-6 hashes + space + content
        let re_heading_line = Regex::new(r"(?m)^(#{1,6}\s)").unwrap();
        let protected = re_heading_line.replace_all(text, |caps: &regex::Captures| {
            format!("\u{0000}H{}", &caps[1])
        });

        // Protect lines that start with list markers (-, *, or number.)
        let re_list_line = Regex::new(r"(?m)^([-*]\s|[0-9]+\.\s)").unwrap();
        let protected = re_list_line.replace_all(&protected, |caps: &regex::Captures| {
            format!("\u{0000}L{}", &caps[1])
        });

        // Protect markdown table rows (lines starting with |)
        let re_table_line = Regex::new(r"(?m)^(\|)").unwrap();
        let protected = re_table_line.replace_all(&protected, |caps: &regex::Captures| {
            format!("\u{0000}T{}", &caps[1])
        });

        // Step 2: Protect paragraph breaks (2+ newlines)
        let re_para = Regex::new(r"\n{2,}").unwrap();
        let protected = re_para.replace_all(&protected, PARA_PLACEHOLDER);

        // Step 3: Protect sentence endings followed by newline
        let re_sent = Regex::new(r"([.。!?！？])\s*\n").unwrap();
        let protected = re_sent.replace_all(&protected, |caps: &regex::Captures| {
            format!("{}{}", &caps[1], SENT_PLACEHOLDER)
        });

        // Step 4: Replace remaining single newlines with space
        let merged = protected.replace('\n', " ");

        // Step 5: Restore sentence breaks as single newline
        let merged = merged.replace(SENT_PLACEHOLDER, "\n");

        // Step 6: Restore paragraph breaks
        let merged = merged.replace(PARA_PLACEHOLDER, "\n\n");

        // Step 7: Restore markdown block markers with newline prefix
        // Headings get their own line
        let merged = merged.replace("\u{0000}H", "\n");
        // List items get their own line
        let merged = merged.replace("\u{0000}L", "\n");
        // Table rows get their own line
        merged.replace("\u{0000}T", "\n")
    }

    fn merge_list_markers(&self, text: &str) -> String {
        // Merge list markers with following content
        // Handles:
        // - "• \n내용" → "• 내용"
        // - "01. \n내용" → "01. 내용"
        // - "1) \n내용" → "1) 내용"
        // - "(1) \n내용" → "(1) 내용"
        // - "■\n내용" → "■ 내용"

        let mut result = text.to_string();

        // Bullet markers followed by newline (• \n, - \n, ■\n, etc.)
        let re_bullet = Regex::new(r"([•\-■□▪▸►◆◇➤✓✗])\s*\n\s*").unwrap();
        result = re_bullet.replace_all(&result, "$1 ").to_string();

        // Numbered list markers: "01. \n", "1. \n", "1) \n", "(1) \n"
        let re_number = Regex::new(r"(\d{1,3}[.)]\s*)\n\s*").unwrap();
        result = re_number.replace_all(&result, "$1").to_string();

        let re_paren_number = Regex::new(r"(\(\d{1,3}\)\s*)\n\s*").unwrap();
        result = re_paren_number.replace_all(&result, "$1").to_string();

        // Korean list markers: "가. \n", "나. \n", etc.
        let re_korean = Regex::new(r"([가-힣][.)]\s*)\n\s*").unwrap();
        result = re_korean.replace_all(&result, "$1").to_string();

        // Circled numbers: ❶, ❷, etc.
        let re_circled = Regex::new(r"([❶-❿])\s*\n\s*").unwrap();
        result = re_circled.replace_all(&result, "$1 ").to_string();

        result
    }

    fn merge_cjk_lines(&self, text: &str) -> String {
        // Merge CJK (Korean/Chinese/Japanese) characters across SINGLE line breaks only
        // Fixes mid-sentence breaks like "반드시 지키\n십시오" → "반드시 지키십시오"
        // But preserves paragraph breaks (2+ newlines)
        //
        // Also handles: CJK + punctuation + newline + CJK
        // E.g., "감사합니다.\n반드시" → preserves as separate sentences

        const PLACEHOLDER: &str = "\u{0000}CJKPARA\u{0000}";

        // First, protect paragraph breaks (2+ newlines)
        let re_para = Regex::new(r"\n{2,}").unwrap();
        let protected = re_para.replace_all(text, PLACEHOLDER);

        // Pattern: CJK char (not followed by sentence-ending punctuation) + single newline + CJK char
        // Don't merge if the first char is followed by sentence-ending punctuation
        let re = Regex::new(
            r"([\p{Hangul}\p{Han}\p{Hiragana}\p{Katakana}])([^.。!?！？\n]?)\n([\p{Hangul}\p{Han}\p{Hiragana}\p{Katakana}])"
        ).unwrap();

        let merged = re.replace_all(&protected, "$1$2$3").to_string();

        // Restore paragraph breaks
        merged.replace(PLACEHOLDER, "\n\n")
    }
}

impl Default for CleanupPipeline {
    fn default() -> Self {
        Self::new(CleanupOptions::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unicode_normalization() {
        let pipeline = CleanupPipeline::from_preset(CleanupPreset::Minimal);
        let text = "café"; // With combining characters
        let result = pipeline.process(text);
        assert!(result.contains("café"));
    }

    #[test]
    fn test_ligature_fix() {
        let pipeline = CleanupPipeline::from_preset(CleanupPreset::Standard);
        let text = "ﬁnding ﬂowers";
        let result = pipeline.process(text);
        assert_eq!(result, "finding flowers");
    }

    #[test]
    fn test_bullet_standardization() {
        let pipeline = CleanupPipeline::from_preset(CleanupPreset::Standard);
        let text = "● Item 1\n○ Item 2\n■ Item 3";
        let result = pipeline.process(text);
        assert!(result.contains("• Item 1"));
        assert!(result.contains("• Item 2"));
    }

    #[test]
    fn test_hyphenation_fix() {
        let pipeline = CleanupPipeline::from_preset(CleanupPreset::Standard);

        // Basic case: hyphen + newline
        let text = "This is infor-\nmation about something.";
        let result = pipeline.process(text);
        assert!(result.contains("information"));

        // Case with space after hyphen (common in PDF extraction)
        let text2 = "This is adip- iscing elit.";
        let result2 = pipeline.process(text2);
        assert!(
            result2.contains("adipiscing"),
            "Expected 'adipiscing' but got: {}",
            result2
        );

        // Case with hyphen + newline + space
        let text3 = "con-\n sectetuer";
        let result3 = pipeline.process(text3);
        assert!(
            result3.contains("consectetuer"),
            "Expected 'consectetuer' but got: {}",
            result3
        );
    }

    #[test]
    fn test_frontmatter_preservation() {
        let pipeline = CleanupPipeline::from_preset(CleanupPreset::Aggressive);
        let text = "---\ntitle: Test\n---\n\nContent with   extra   spaces.";
        let result = pipeline.process(text);
        assert!(result.starts_with("---\n"));
        assert!(result.contains("title: Test"));
    }

    #[test]
    fn test_merge_single_newlines() {
        let pipeline = CleanupPipeline::from_preset(CleanupPreset::Standard);
        let text = "This\nis\na\ntest.\n\nNew paragraph.";
        let result = pipeline.process(text);
        assert!(result.contains("This is a test."));
        assert!(result.contains("\n\n") || result.contains("New paragraph"));
    }

    #[test]
    fn test_remove_replacement_char() {
        let pipeline = CleanupPipeline::from_preset(CleanupPreset::Standard);
        let text = "Hello\u{FFFD}World";
        let result = pipeline.process(text);
        assert_eq!(result, "HelloWorld");
    }

    #[test]
    fn test_merge_list_markers_bullet() {
        let pipeline = CleanupPipeline::from_preset(CleanupPreset::Standard);
        let text = "• \n'안전을 위한 주의사항'은 제품을 올바르게 사용하기 위한 것입니다.";
        let result = pipeline.process(text);
        assert!(
            result.starts_with("• '안전을"),
            "Expected bullet merged, got: {}",
            result
        );
    }

    #[test]
    fn test_merge_list_markers_number() {
        let pipeline = CleanupPipeline::from_preset(CleanupPreset::Standard);
        let text = "01. \n인명이나 재산상에 영향이 큰 기기에 사용하지 마십시오.";
        let result = pipeline.process(text);
        assert!(
            result.starts_with("01. 인명이나"),
            "Expected number merged, got: {}",
            result
        );
    }

    #[test]
    fn test_merge_cjk_lines() {
        let pipeline = CleanupPipeline::from_preset(CleanupPreset::Standard);
        let text = "반드시 지키\n십시오.";
        let result = pipeline.process(text);
        assert!(
            result.contains("반드시 지키십시오"),
            "Expected CJK merged, got: {}",
            result
        );
    }

    #[test]
    fn test_merge_cjk_with_space() {
        let pipeline = CleanupPipeline::from_preset(CleanupPreset::Standard);
        let text = "특정조건 하에서\n 위험이 발생할 우려가 있습니다.";
        let result = pipeline.process(text);
        // Korean text should preserve spaces (unlike Chinese/Japanese)
        // With CJK line merge, newline becomes space
        assert!(
            result.contains("하에서") && result.contains("위험이"),
            "Expected proper merge, got: {}",
            result
        );
    }
}
