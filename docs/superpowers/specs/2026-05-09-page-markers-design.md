# Page Boundary Markers — Design Spec

**Date**: 2026-05-09  
**Status**: Approved  
**Target version**: 0.5.0 (minor — new opt-in feature)

---

## Problem

`unpdf` extracts PDF content to Markdown without any page boundary information. Images are
named `page_042.png` but the corresponding text in `extract.md` has no page anchors, making
content-to-page mapping impossible. AI pipelines (RAG, document Q&A, error tracing) need
page numbers to correlate text chunks back to the source PDF.

---

## Goals

- Insert `<!-- page N -->` HTML comment markers at page boundaries in Markdown output.
- Opt-in only — default behavior unchanged (no markers).
- Both render paths (streaming `convert` and non-streaming `markdown`) must produce
  identical marker placement.
- Markers must survive the `CleanupPipeline` post-processing pass unchanged.

## Non-Goals

- Frontmatter `page_index` (method B from the issue) — deferred; design impact is larger
  and demand from multiple consumers not yet established.
- Plain text or JSON output — only Markdown output receives markers.
- Anchor-style markers (`<a id="page-N">`) — not needed yet; `PageMarkerStyle` enum makes
  future addition trivial.

---

## Architecture

### New type: `PageMarkerStyle` (`src/render/options.rs`)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PageMarkerStyle {
    #[default]
    None,
    Comment,   // emits <!-- page N -->
}
```

Added to `RenderOptions`:

```rust
pub struct RenderOptions {
    // ... existing fields ...
    pub page_markers: PageMarkerStyle,
}
```

Builder method:

```rust
pub fn with_page_markers(mut self, style: PageMarkerStyle) -> Self {
    self.page_markers = style;
    self
}
```

Default value: `PageMarkerStyle::None` — no change in existing output.

---

### Render path A — `MarkdownRenderer` (`src/render/markdown.rs`)

`render_page()` emits the marker before the first block of each page:

```rust
fn render_page(&mut self, output: &mut String, page: &Page) {
    if self.options.page_markers == PageMarkerStyle::Comment {
        if !output.is_empty() && !output.ends_with('\n') {
            output.push('\n');
        }
        output.push_str(&format!("<!-- page {} -->\n\n", page.number));
    }
    for block in &page.elements { ... }
}
```

Also remove the stale "Optionally add page break marker" comment from the
`Block::PageBreak | Block::SectionBreak` arm (line 129) — that intent is now fulfilled here.

---

### Render path B — `MultiFormatWriter` (`cli/src/writer.rs`)

`write_page()` emits the marker into the MD file before writing blocks.
The `page.number` field is already available on the `Page` argument:

```rust
pub fn write_page(&mut self, page: &Page) -> std::io::Result<()> {
    self.flush_page_images(page)?;

    if let Some(w) = self.md.as_mut() {
        if self.render_opts.page_markers == PageMarkerStyle::Comment {
            w.write_all(format!("<!-- page {} -->\n\n", page.number).as_bytes())?;
        }
        // ... existing block rendering ...
    }
    // ... txt / json unchanged ...
}
```

Note: the `CleanupPipeline` read-modify-write pass in `finish()` operates on the completed
file. HTML comments (`<!-- ... -->`) are not touched by any cleanup rule (verified: cleanup
targets whitespace, duplicate lines, and encoding artefacts only).

---

### CLI flags (`cli/src/main.rs`)

Added to `Markdown` subcommand:

```
/// Insert HTML page boundary markers (<!-- page N -->)
#[arg(long)]
page_markers: bool,
```

Added to `ConvertArgs` struct (used by `convert` subcommand and the default convert path):

```
/// Insert HTML page boundary markers (<!-- page N -->)
#[arg(long)]
pub page_markers: bool,
```

Both flags map to `RenderOptions::with_page_markers(PageMarkerStyle::Comment)` when `true`.

---

## Marker Format & Placement

```markdown
---
pages: 2298
---

<!-- page 1 -->

# SCOPE OF WORK AND TECHNICAL SPECIFICATIONS ...

<!-- page 2 -->

## 1.1 Introduction
...
```

- Marker appears **before** the first content of each page.
- For page 1, the marker follows the frontmatter block (if enabled).
- The marker is always emitted, even for pages with no extractable text, so consumers
  can rely on strict `page N` → `page N+1` ordering.
- Regex for consumers: `<!-- page (\d+) -->`

---

## Testing

| Test | Location | Assertion |
|------|----------|-----------|
| Marker inserted when `Comment` | `render/markdown.rs` tests | output contains `<!-- page 1 -->` |
| No marker by default (`None`) | `render/markdown.rs` tests | output does NOT contain `<!--` |
| Streaming path parity | `cli/writer.rs` integration | same marker position as non-streaming |
| CleanupPipeline passthrough | `render/cleanup.rs` tests | `<!-- page N -->` survives all presets |
| CLI `--page-markers` flag | manual / doc | rendered output contains markers |

---

## Files Changed

| File | Change |
|------|--------|
| `src/render/options.rs` | Add `PageMarkerStyle` enum, field, builder |
| `src/render/markdown.rs` | Emit marker in `render_page()`; remove stale comment |
| `cli/src/writer.rs` | Emit marker in `write_page()` |
| `cli/src/main.rs` | Add `--page-markers` to `Markdown` and `ConvertArgs` |

---

## Version & Release

- Semver: **0.5.0** (minor — new opt-in public API field)
- All four version files must be bumped together per CLAUDE.md checklist:
  `Cargo.toml`, `cli/Cargo.toml`, `bindings/python/pyproject.toml`,
  `bindings/csharp/Unpdf/Unpdf.csproj`
