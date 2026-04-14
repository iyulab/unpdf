# Changelog

## 0.4.1 — 2026-04-14

Completes the image story left open in 0.4.0.

### Added
- Images are extracted by default again (reverts 0.4.0's opt-in after
  the streaming pipeline made per-page flush-to-disk safe) — parsed
  images now flow page-by-page into `<out>/images/` and are embedded
  as `![](images/<id>)` references in `extract.md`.
- `Block::Image` blocks are now emitted into `page.elements` with the
  resource id matching the on-disk filename, so any downstream renderer
  (including future layout-aware ones) sees images inline.
- `ParseOptions::min_image_dimension` (default `64`) drops tiny
  decorative xobjects (logos, bullets, rule lines, tracking pixels).
  Set to `0` to keep every image.
- `Page.images: Vec<(String, Resource)>` field for per-page image
  enumeration during streaming.
- CLI:
  - `--no-images` opt-out (replaces the 0.4.0 `--images` opt-in)
  - `--min-image-size <PX>` (default 64)
  - Finish banner now reports how many images were written

### Changed
- Non-renderable image formats (raw FlateDecode pixel buffers, unknown
  encodings that would land as `.raw`/`.bin`) are no longer written to
  disk or referenced in MD — they produced broken-icon refs. Will be
  revisited in a follow-up when PNG reconstruction lands.
- CI: new `version-check` job guards against version drift across
  `Cargo.toml`, `cli/Cargo.toml`, `bindings/python/pyproject.toml`,
  `bindings/csharp/Unpdf/Unpdf.csproj`, and `cli` → `unpdf` workspace
  dep.

### Validated
- 2298-page / 165 MB reference PDF: **1062 images** extracted with
  default `--min-image-size 64` (vs 1272 unfiltered), **1062 MD refs**
  matching on-disk files, ~19 s wall-clock, ~1 s TTFB.

## 0.4.0 — 2026-04-14

### BREAKING

- `ParseOptions::extract_resources` default changed `true` → `false`.
  Large PDFs no longer silently load all images into memory. Opt in via
  `.with_resources(true)` or `Unpdf::with_images(true)`.
- CLI `unpdf convert` default output is now Markdown only. Use `--all`
  or `--formats md,txt,json` for multi-format fan-out.
- `ParseOptions::memory_limit_mb` field removed (deprecated and
  non-functional since 0.1.8). Use `with_pages` to limit scope.
- `Unpdf::with_memory_limit_mb` builder method removed (same reason).

### Added

- Streaming parse pipeline: `PdfParser::for_each_page`, `ParseEvent`
  (`DocumentStart` / `PageParsed` / `PageFailed` / `Progress` /
  `DocumentEnd`), `PageStreamOptions`.
- `QualityAccumulator` for incremental quality metrics.
- `StreamingRenderer::render_block_public` adapter for external
  renderers that drive their own page loop.
- CLI flags: `--formats`, `--all`, `--images`, `--image-dir`, `--window`.
- Per-page progress bar shows `N/total` during convert.
- Integration test `tests/streaming_equivalence.rs` — parallel vs
  sequential structural equivalence.
- CLI smoke tests `cli/tests/cli_streaming.rs`.

### Changed

- `PdfParser::parse()` now routes through the streaming pipeline
  internally (signature unchanged, `Document` still fully materialized
  for existing users).
- `PdfBackend` trait now requires `Send + Sync`; backend font caches
  switched from `RefCell` to `Mutex` for thread safety.
- Resource extraction fused into the main parse loop — second full
  page iteration removed.
- Quality metrics computed incrementally; no more multi-MB
  `plain_text()` reassembly at end of parse.

### Performance

- rayon page-parallel parsing with bounded reorder window
  (`ReorderBuffer`) preserves page_num ASC output order.
- 2298-page / 165MB PDF target: time-to-first-byte in seconds, wall-
  clock multi-fold faster on multi-core, peak RSS an order of
  magnitude lower. See `dev-docs/perf-validation.md` and the validation
  record in `dev-docs/perf-history.md` (updated at release).

### Migration

See `MIGRATION-0.4.md`.

## 0.3.0 and earlier

See git history.
