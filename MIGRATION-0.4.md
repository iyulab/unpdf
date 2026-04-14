# Migration Guide: 0.3.x → 0.4.0

## Breaking Changes

### 1. `ParseOptions::extract_resources` default is now `false`

Previously `true`. Large PDFs silently loading all images into memory was
the largest peak-memory vector.

Before (0.3.x):
```rust
let doc = parse_file("big.pdf")?; // images extracted by default
```

After (0.4.0):
```rust
let opts = ParseOptions::new().with_resources(true);
let doc = parse_file_with_options("big.pdf", opts)?;
// or with the builder:
let result = Unpdf::new().with_images(true).parse("big.pdf")?;
```

### 2. CLI `unpdf convert` default formats

Previously produced `extract.md`, `extract.txt`, `content.json`.
Now produces only `extract.md`.

Before:
```
unpdf convert file.pdf -o out/
# -> out/extract.md, out/extract.txt, out/content.json
```

After:
```
unpdf convert file.pdf -o out/
# -> out/extract.md only
unpdf convert file.pdf -o out/ --all
# -> out/extract.md, out/extract.txt, out/content.json
unpdf convert file.pdf -o out/ --formats md,json
# -> out/extract.md, out/content.json
```

Image extraction is also opt-in:
```
unpdf convert file.pdf -o out/ --images
unpdf convert file.pdf -o out/ --image-dir out/img/
```

### 3. `ParseOptions::memory_limit_mb` removed

Deprecated and non-functional since 0.1.8. Use `with_pages` to limit
processing scope instead.

`Unpdf::with_memory_limit_mb` is also removed for the same reason.

## New APIs (non-breaking)

### Streaming parse

```rust
use std::ops::ControlFlow;
use unpdf::{PdfParser, PageStreamOptions, ParseEvent};

let parser = PdfParser::open("large.pdf")?;
parser.for_each_page(PageStreamOptions::default(), |ev| {
    match ev {
        ParseEvent::DocumentStart { page_count, .. } => {
            println!("opening document, {} pages", page_count);
        }
        ParseEvent::PageParsed(page) => {
            // process or render page immediately; no need to hold the
            // whole document in memory
            println!("page {}: {} blocks", page.number, page.elements.len());
        }
        ParseEvent::PageFailed { page, error } => {
            eprintln!("page {} failed: {}", page, error);
        }
        ParseEvent::Progress { done, total } => {
            eprintln!("{}/{}", done, total);
        }
        ParseEvent::DocumentEnd { .. } => {}
    }
    ControlFlow::Continue(())
})?;
```

### `PageStreamOptions`

```rust
PageStreamOptions {
    parallel: true,            // rayon page-parallel parsing
    window_size: N,            // in-flight cap (default cores*2)
    emit_progress_every: 16,
    extract_resources: false,
    flush_resources_to: None,
    // ... ParseOptions fields inherited
}
```

### CLI flags

- `--formats md,txt,json` (default `md`)
- `--all` (all three formats)
- `--images`, `--image-dir <DIR>` (opt-in image extraction)
- `--window <N>` (override in-flight page window)

## Why

The 0.4.0 pipeline supports very large PDFs (validated at 2298 pages /
165MB):

- Time-to-first-byte is measured in seconds, not minutes.
- Wall-clock is faster on multi-core systems via rayon.
- Peak memory scales with `window_size` instead of document size.
- Output is deterministic page-number ASC order regardless of window size.

See `dev-docs/perf-validation.md` for measurement procedures.
