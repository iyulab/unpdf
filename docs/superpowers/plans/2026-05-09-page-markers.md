# Page Boundary Markers Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Insert `<!-- page N -->` HTML comment markers at page boundaries in Markdown output, opt-in via `RenderOptions::page_markers` and `--page-markers` CLI flag.

**Architecture:** Add `PageMarkerStyle` enum to `src/render/options.rs` with `None` (default) and `Comment` variants; emit the marker in both render paths — `MarkdownRenderer::render_page()` (non-streaming) and `MultiFormatWriter::write_page()` (streaming); wire CLI flags in both `Markdown` and `ConvertArgs` subcommands.

**Tech Stack:** Rust, clap (CLI), cargo test (unit tests)

---

## File Map

| File | Change |
|------|--------|
| `src/render/options.rs` | Add `PageMarkerStyle` enum, field on `RenderOptions`, builder method |
| `src/render/mod.rs` | Re-export `PageMarkerStyle` |
| `src/lib.rs` | Re-export `PageMarkerStyle` at crate root |
| `src/render/markdown.rs` | Emit marker in `render_page()`; remove stale comment on line 129 |
| `cli/src/writer.rs` | Emit marker in `write_page()` |
| `cli/src/main.rs` | Add `--page-markers` flag to `Markdown` subcommand and `ConvertArgs` |

---

## Task 1: Add `PageMarkerStyle` enum to `RenderOptions`

**Files:**
- Modify: `src/render/options.rs`

- [ ] **Step 1: Write failing tests** — append to the `mod tests` block (line 282) in `src/render/options.rs`:

```rust
#[test]
fn test_page_marker_style_default_is_none() {
    let options = RenderOptions::new();
    assert_eq!(options.page_markers, PageMarkerStyle::None);
}

#[test]
fn test_page_marker_style_builder() {
    let options = RenderOptions::new().with_page_markers(PageMarkerStyle::Comment);
    assert_eq!(options.page_markers, PageMarkerStyle::Comment);
}
```

- [ ] **Step 2: Run tests to verify they fail**

```
cargo test -p unpdf page_marker_style
```

Expected: compilation error — `PageMarkerStyle` not yet defined.

- [ ] **Step 3: Add the `PageMarkerStyle` enum** — insert before the existing `TableFallback` enum (around line 176) in `src/render/options.rs`:

```rust
/// Style for page boundary markers in Markdown output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PageMarkerStyle {
    /// No markers (default, preserves existing output unchanged).
    #[default]
    None,
    /// HTML comment: `<!-- page N -->` inserted before each page's content.
    Comment,
}
```

- [ ] **Step 4: Add the field to `RenderOptions`** — in the `RenderOptions` struct, add after `collect_stats`:

```rust
    /// Style for page boundary markers in Markdown output.
    pub page_markers: PageMarkerStyle,
```

- [ ] **Step 5: Set the default** — in `impl Default for RenderOptions`, add:

```rust
            page_markers: PageMarkerStyle::None,
```

- [ ] **Step 6: Add builder method** — in the second `impl RenderOptions` block (the one with `with_stats`), add:

```rust
    /// Set the page marker style.
    pub fn with_page_markers(mut self, style: PageMarkerStyle) -> Self {
        self.page_markers = style;
        self
    }
```

- [ ] **Step 7: Run tests to verify they pass**

```
cargo test -p unpdf page_marker_style
```

Expected: both tests PASS.

---

## Task 2: Re-export `PageMarkerStyle`

**Files:**
- Modify: `src/render/mod.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Add to `src/render/mod.rs`** — change line 15:

```rust
// Before:
pub use options::{HeadingConfig, PageSelection, RenderOptions, TableFallback};

// After:
pub use options::{HeadingConfig, PageMarkerStyle, PageSelection, RenderOptions, TableFallback};
```

- [ ] **Step 2: Add to `src/lib.rs`** — change line 58:

```rust
// Before:
pub use render::{
    CleanupOptions, CleanupPreset, HeadingConfig, JsonFormat, PageSelection, RenderOptions,
    TableFallback,
};

// After:
pub use render::{
    CleanupOptions, CleanupPreset, HeadingConfig, JsonFormat, PageMarkerStyle, PageSelection,
    RenderOptions, TableFallback,
};
```

- [ ] **Step 3: Verify compilation**

```
cargo check -p unpdf
```

Expected: no errors.

- [ ] **Step 4: Commit**

```
git add src/render/options.rs src/render/mod.rs src/lib.rs
git commit -m "feat: add PageMarkerStyle enum to RenderOptions"
```

---

## Task 3: Emit markers in `MarkdownRenderer`

**Files:**
- Modify: `src/render/markdown.rs`

- [ ] **Step 1: Add `PageMarkerStyle` to the imports** — change line 9 in `src/render/markdown.rs`:

```rust
// Before:
use super::{CleanupPipeline, ExtractionStats, RenderOptions, RenderResult, TableFallback};

// After:
use super::{CleanupPipeline, ExtractionStats, PageMarkerStyle, RenderOptions, RenderResult, TableFallback};
```

- [ ] **Step 2: Write failing tests** — append to the `mod tests` block in `src/render/markdown.rs`:

```rust
#[test]
fn test_page_markers_comment_inserted() {
    let mut doc = Document::new();
    let mut page1 = Page::letter(1);
    page1.add_paragraph(Paragraph::with_text("First page content"));
    doc.add_page(page1);
    let mut page2 = Page::letter(2);
    page2.add_paragraph(Paragraph::with_text("Second page content"));
    doc.add_page(page2);

    let options = RenderOptions::new().with_page_markers(PageMarkerStyle::Comment);
    let result = to_markdown(&doc, &options).unwrap();
    assert!(result.contains("<!-- page 1 -->"), "marker for page 1 missing in:\n{}", result);
    assert!(result.contains("<!-- page 2 -->"), "marker for page 2 missing in:\n{}", result);
}

#[test]
fn test_page_markers_none_by_default() {
    let mut doc = Document::new();
    let mut page = Page::letter(1);
    page.add_paragraph(Paragraph::with_text("Content"));
    doc.add_page(page);

    let options = RenderOptions::new();
    let result = to_markdown(&doc, &options).unwrap();
    assert!(!result.contains("<!--"), "unexpected comment marker in output:\n{}", result);
}

#[test]
fn test_page_markers_precede_content() {
    let mut doc = Document::new();
    let mut page = Page::letter(1);
    page.add_paragraph(Paragraph::heading("Chapter 1", 1));
    doc.add_page(page);

    let options = RenderOptions::new().with_page_markers(PageMarkerStyle::Comment);
    let result = to_markdown(&doc, &options).unwrap();
    let marker_pos = result.find("<!-- page 1 -->").expect("marker missing");
    let heading_pos = result.find("# Chapter 1").expect("heading missing");
    assert!(marker_pos < heading_pos, "marker must precede page content");
}
```

- [ ] **Step 3: Run tests to verify they fail**

```
cargo test -p unpdf page_markers
```

Expected: `test_page_markers_comment_inserted` FAILS — output has no `<!-- page 1 -->`.

- [ ] **Step 4: Implement marker emission in `render_page()`** — replace the existing `render_page` method (line 95-102) in `src/render/markdown.rs`:

```rust
fn render_page(&mut self, output: &mut String, page: &Page) {
    if self.options.page_markers == PageMarkerStyle::Comment {
        if !output.is_empty() && !output.ends_with("\n\n") {
            output.push('\n');
        }
        output.push_str(&format!("<!-- page {} -->\n\n", page.number));
    }
    if self.options.collect_stats {
        self.stats.add_page();
    }
    for block in &page.elements {
        self.render_block(output, block);
    }
}
```

> **Note on newline guard:** `!output.is_empty() && !output.ends_with("\n\n")` ensures a blank
> line is inserted before the marker when frontmatter (ending with `\n`) precedes page 1. For
> page 2+, the previous block already ends with `\n\n`, so no extra newline is added. For
> page 1 without frontmatter, `output` is empty, so the guard is skipped and the marker
> becomes the first content of the file.

- [ ] **Step 5: Remove stale comment** — in the `render_block` method, find the `Block::PageBreak | Block::SectionBreak` arm (around line 129) and remove the comment "Optionally add page break marker":

```rust
// Before:
Block::PageBreak | Block::SectionBreak => {
    // Optionally add page break marker
    if !output.ends_with("\n\n") {
        output.push_str("\n\n");
    }
}

// After:
Block::PageBreak | Block::SectionBreak => {
    if !output.ends_with("\n\n") {
        output.push_str("\n\n");
    }
}
```

- [ ] **Step 6: Run tests to verify they pass**

```
cargo test -p unpdf page_markers
```

Expected: all three new tests PASS.

- [ ] **Step 7: Run full library tests to check for regressions**

```
cargo test -p unpdf
```

Expected: all tests PASS.

- [ ] **Step 8: Verify cleanup pipeline does not strip markers** — append this test to the same `mod tests` block:

```rust
#[test]
fn test_page_markers_survive_cleanup() {
    use crate::render::{CleanupOptions, CleanupPreset};
    let mut doc = Document::new();
    let mut page = Page::letter(1);
    page.add_paragraph(Paragraph::with_text("Content"));
    doc.add_page(page);

    for preset in [CleanupPreset::Minimal, CleanupPreset::Standard, CleanupPreset::Aggressive] {
        let options = RenderOptions::new()
            .with_page_markers(PageMarkerStyle::Comment)
            .with_cleanup(CleanupOptions::from_preset(preset));
        let result = to_markdown(&doc, &options).unwrap();
        assert!(
            result.contains("<!-- page 1 -->"),
            "marker stripped by {:?} cleanup preset", preset
        );
    }
}
```

Run:

```
cargo test -p unpdf page_markers_survive_cleanup
```

Expected: PASS.

- [ ] **Step 9: Commit**

```
git add src/render/markdown.rs
git commit -m "feat: emit <!-- page N --> markers in MarkdownRenderer"
```

---

## Task 4: Emit markers in streaming `MultiFormatWriter`

**Files:**
- Modify: `cli/src/writer.rs`

- [ ] **Step 1: Add `PageMarkerStyle` import** — in `cli/src/writer.rs`, change the existing import line:

```rust
// Before:
use unpdf::render::{CleanupPipeline, RenderOptions, StreamingRenderer};

// After:
use unpdf::render::{CleanupPipeline, PageMarkerStyle, RenderOptions, StreamingRenderer};
```

- [ ] **Step 2: Write a failing integration test** — append to `cli/src/writer.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use unpdf::model::{Page, Paragraph};
    use unpdf::render::PageMarkerStyle;

    #[test]
    fn test_streaming_writer_inserts_page_marker() {
        let tmp = std::env::temp_dir().join("unpdf_writer_marker_test");
        std::fs::create_dir_all(&tmp).unwrap();

        let doc = unpdf::model::Document::new();
        let render_opts = RenderOptions::new()
            .with_page_markers(PageMarkerStyle::Comment)
            .with_cleanup(unpdf::render::CleanupOptions::from_preset(
                unpdf::CleanupPreset::Minimal,
            ));
        let formats = vec![OutputFormat::Markdown];
        let mut mfw = MultiFormatWriter::new(&tmp, &formats, render_opts, None).unwrap();

        mfw.write_document_start(&doc.metadata, 2).unwrap();

        let mut page1 = Page::letter(1);
        page1.add_paragraph(Paragraph::with_text("Page one text"));
        mfw.write_page(&page1).unwrap();

        let mut page2 = Page::letter(2);
        page2.add_paragraph(Paragraph::with_text("Page two text"));
        mfw.write_page(&page2).unwrap();

        mfw.finish().unwrap();

        let content = std::fs::read_to_string(tmp.join("extract.md")).unwrap();
        assert!(content.contains("<!-- page 1 -->"), "page 1 marker missing:\n{}", content);
        assert!(content.contains("<!-- page 2 -->"), "page 2 marker missing:\n{}", content);

        let p1_pos = content.find("<!-- page 1 -->").unwrap();
        let p2_pos = content.find("<!-- page 2 -->").unwrap();
        assert!(p1_pos < p2_pos, "page 1 marker must precede page 2 marker");

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn test_streaming_writer_no_marker_by_default() {
        let tmp = std::env::temp_dir().join("unpdf_writer_no_marker_test");
        std::fs::create_dir_all(&tmp).unwrap();

        let doc = unpdf::model::Document::new();
        let render_opts = RenderOptions::new();
        let formats = vec![OutputFormat::Markdown];
        let mut mfw = MultiFormatWriter::new(&tmp, &formats, render_opts, None).unwrap();

        mfw.write_document_start(&doc.metadata, 1).unwrap();
        let mut page = Page::letter(1);
        page.add_paragraph(Paragraph::with_text("Content"));
        mfw.write_page(&page).unwrap();
        mfw.finish().unwrap();

        let content = std::fs::read_to_string(tmp.join("extract.md")).unwrap();
        assert!(!content.contains("<!--"), "unexpected marker:\n{}", content);

        std::fs::remove_dir_all(&tmp).ok();
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

```
cargo test -p unpdf-cli
```

Expected: `test_streaming_writer_inserts_page_marker` FAILS — output has no `<!-- page 1 -->`.

- [ ] **Step 4: Implement marker emission in `write_page()`** — in `cli/src/writer.rs`, replace the inner MD block in `write_page()`:

```rust
pub fn write_page(&mut self, page: &Page) -> std::io::Result<()> {
    self.flush_page_images(page)?;

    if let Some(w) = self.md.as_mut() {
        if self.render_opts.page_markers == PageMarkerStyle::Comment {
            // Leading `\n` ensures a blank line after frontmatter (which ends with `\n`).
            // For page 1 without frontmatter, the leading `\n` is trimmed by the cleanup
            // pipeline (enabled by default). Page 2+ content always ends with `\n\n`,
            // so the extra `\n` produces clean triple-newline spacing before cleanup.
            w.write_all(
                format!("\n<!-- page {} -->\n\n", page.number).as_bytes(),
            )?;
        }
        let placeholder = unpdf::model::Document::new();
        let renderer = StreamingRenderer::new(&placeholder, self.render_opts.clone());
        for block in &page.elements {
            let chunk = renderer.render_block_public(block);
            if !chunk.is_empty() {
                w.write_all(chunk.as_bytes())?;
            }
        }
    }
    if let Some(w) = self.txt.as_mut() {
        for block in &page.elements {
            let mut buf = String::new();
            block.append_plain_text(&mut buf);
            if !buf.is_empty() {
                w.write_all(buf.as_bytes())?;
                w.write_all(b"\n")?;
            }
        }
    }
    if let Some(w) = self.json.as_mut() {
        if !self.json_first_page {
            w.write_all(b",")?;
        }
        serde_json::to_writer(&mut *w, page).map_err(io_err)?;
        self.json_first_page = false;
    }
    Ok(())
}
```

- [ ] **Step 5: Run tests to verify they pass**

```
cargo test -p unpdf-cli
```

Expected: both new tests PASS.

- [ ] **Step 6: Commit**

```
git add cli/src/writer.rs
git commit -m "feat: emit page markers in streaming MultiFormatWriter"
```

---

## Task 5: Add `--page-markers` CLI flag

**Files:**
- Modify: `cli/src/main.rs`

- [ ] **Step 1: Add flag to `Markdown` subcommand** — in the `Markdown` variant of the `Commands` enum (around line 95), add the new field:

```rust
Commands::Markdown {
    input: PathBuf,
    output: Option<PathBuf>,
    frontmatter: bool,
    table_mode: TableMode,
    cleanup: Option<CleanupLevel>,
    max_heading: u8,
    pages: Option<String>,
    // ADD THIS:
    /// Insert HTML page boundary markers (<!-- page N -->)
    #[arg(long)]
    page_markers: bool,
},
```

- [ ] **Step 2: Add flag to `ConvertArgs`** — in the `ConvertArgs` struct (around line 20), add after `window`:

```rust
    /// Insert HTML page boundary markers (<!-- page N -->)
    #[arg(long)]
    pub page_markers: bool,
```

- [ ] **Step 3: Wire `Markdown` subcommand** — update the `cmd_markdown` call in `main()` to pass the new flag:

```rust
// In main(), the Markdown match arm:
Some(Commands::Markdown {
    input,
    output,
    frontmatter,
    table_mode,
    cleanup,
    max_heading,
    pages,
    page_markers,    // ADD
}) => cmd_markdown(
    &input,
    output.as_deref(),
    frontmatter,
    table_mode,
    cleanup,
    max_heading,
    pages.as_deref(),
    page_markers,    // ADD
    quiet,
),
```

- [ ] **Step 4: Update `cmd_markdown` signature and body** — change the function signature and add the wiring:

```rust
#[allow(clippy::too_many_arguments)]
fn cmd_markdown(
    input: &Path,
    output: Option<&Path>,
    frontmatter: bool,
    table_mode: TableMode,
    cleanup: Option<CleanupLevel>,
    max_heading: u8,
    pages: Option<&str>,
    page_markers: bool,    // ADD
    quiet: bool,
) -> Result<bool, Box<dyn std::error::Error>> {
    let page_selection = if let Some(p) = pages {
        PageSelection::parse(p).map_err(|e| format!("Invalid page range: {}", e))?
    } else {
        PageSelection::All
    };

    let options = ParseOptions::new()
        .lenient()
        .with_pages(page_selection.clone());
    let doc = parse_file_with_options(input, options)?;
    let had_warnings = check_quality(&doc, quiet);

    let mut render_options = RenderOptions::new()
        .with_frontmatter(frontmatter)
        .with_table_fallback(table_mode.into())
        .with_max_heading(max_heading)
        .with_pages(page_selection);

    if page_markers {                                                     // ADD
        render_options = render_options                                   // ADD
            .with_page_markers(unpdf::PageMarkerStyle::Comment);         // ADD
    }                                                                     // ADD

    if let Some(level) = cleanup {
        render_options = render_options.with_cleanup_preset(level.into());
    }

    let markdown = unpdf::render::to_markdown(&doc, &render_options)?;

    if let Some(path) = output {
        fs::write(path, &markdown)?;
        println!("{} {}", "Saved to".green(), path.display());
    } else {
        println!("{}", markdown);
    }

    Ok(had_warnings)
}
```

- [ ] **Step 5: Fix `ConvertArgs` struct literal in the `None` branch** — in `main()`, the `None =>` arm (around line 329) constructs `ConvertArgs` directly. Add the new field to avoid a compile error:

```rust
let args = ConvertArgs {
    input,
    output: cli.output,
    cleanup: cli.cleanup,
    formats: vec!["md".to_string()],
    all: false,
    no_images: false,
    image_dir: None,
    min_image_size: 64,
    window: None,
    quiet,
    page_markers: false,    // ADD
};
```

- [ ] **Step 6: Wire `ConvertArgs` in `cmd_convert`** — in `cmd_convert`, after building `render_opts`, add:

```rust
    if args.page_markers {
        render_opts = render_opts.with_page_markers(unpdf::PageMarkerStyle::Comment);
    }
```

Place this block immediately after:
```rust
    if let Some(level) = args.cleanup {
        render_opts = render_opts.with_cleanup_preset(level.into());
    }
```

- [ ] **Step 7: Verify compilation and linting**

```
cargo check && cargo clippy
```

Expected: no errors or warnings.

- [ ] **Step 8: Run full test suite**

```
cargo test
```

Expected: all tests PASS.

- [ ] **Step 9: Commit**

```
git add cli/src/main.rs
git commit -m "feat(cli): add --page-markers flag to markdown and convert subcommands"
```

---

## Task 6: Smoke test with real PDF

This task is manual verification — no automated test.

- [ ] **Step 1: Run convert on a small PDF**

```
cargo run -p unpdf-cli -- convert tests/fixtures/sample.pdf --page-markers
```

If no fixture PDF is available, use any PDF on disk.

Expected output in `<name>_output/extract.md`:
```markdown
<!-- page 1 -->

# Title ...

<!-- page 2 -->

## Section 1.1 ...
```

- [ ] **Step 2: Verify marker count matches page count** — in PowerShell:

```powershell
(Select-String -Path "<name>_output\extract.md" -Pattern "<!-- page \d+ -->").Count
```

Expected: count equals the PDF page count.

- [ ] **Step 3: Verify default (no flag) has no markers**

```
cargo run -p unpdf-cli -- convert tests/fixtures/sample.pdf
```

```powershell
(Select-String -Path "<name>_output\extract.md" -Pattern "<!--").Count
```

Expected: 0.

---

## Task 7: Final cleanup and format

- [ ] **Step 1: Run formatter**

```
cargo fmt
```

- [ ] **Step 2: Run clippy**

```
cargo clippy -- -D warnings
```

Expected: no warnings.

- [ ] **Step 3: Run full test suite one final time**

```
cargo test
```

Expected: all tests PASS.

- [ ] **Step 4: Final commit if any fmt changes**

```
git add -u
git commit -m "style: cargo fmt after page-markers feature"
```
