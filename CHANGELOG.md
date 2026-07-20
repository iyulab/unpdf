# Changelog

## Unreleased

### Added
- Predefined CJK CMap support for Type0 fonts without a ToUnicode map: `KSC-EUC`,
  `KSCms-UHC` (Adobe-Korea1), `90ms-RKSJ` (Adobe-Japan1), `GBK-EUC` (Adobe-GB1) and
  `ETen-B5` (Adobe-CNS1), in both writing modes, plus the `UniXX-UCS2`/`UniXX-UTF16`
  CMaps (decoded as UTF-16BE). Code→CID tables are generated at build time from the
  Adobe `cid2code.txt` files already shipped for CID→Unicode lookup. Decoding agrees
  with the vendor codecs (EUC-KR, CP949, CP932, GBK, Big5) on 98.7% of mapped codes;
  the remainder are punctuation where Adobe's character collection and the vendor
  codec legitimately disagree (e.g. `⋯` vs `…`).

### Fixed
- CID→Unicode lookup picked the first code point listed for a CID, which is sometimes
  a compatibility duplicate — Adobe-GB1 CID 3795 resolved to the Kangxi radical `⽂`
  instead of `文`. Radicals, compatibility ideographs, Hangul fillers and U+2329/232A
  are now skipped when the CID has an ordinary code point. Affects every CID-keyed
  font, including the Identity-H path.
- Composite (Type0/CID) fonts with no usable CMap no longer fall back to byte-wise Latin-1 decoding,
  which produced mojibake (`°Ë ,¥õ ²ô`). The guard previously covered only `Identity-H/V`, so Type0
  fonts using a predefined CMap (e.g. `/Encoding /KSC-EUC-H` in scanner OCR layers) leaked garbage
  text. Such fonts now yield no text. Predefined CJK CMap support itself is tracked separately.

## 0.7.1 — 2026-07-05

Also ships the parsing-quality and CI work that accumulated on `main` after the v0.7.0 tag.

### Security
- Bump `tar` 0.4.44 → 0.4.46 (RUSTSEC-2026-0067 symlink-traversal chmod, RUSTSEC-2026-0068 PAX size
  header ignored). Both fixed in ≥ 0.4.45; lifted within the existing semver range.
- Add a `cargo audit` CI gate (Security Audit job) with a documented `.cargo/audit.toml`. The only
  accepted advisories are quick-xml 0.23 RUSTSEC-2026-0194/-0195 — an optional, uncompiled transitive
  of self_update's S3 backend (unused; unpdf has no direct quick-xml dependency).

### Added
- Scanned-PDF detection — recognizes image-only PDFs with no embedded fonts and flags them via
  extraction-quality diagnostics instead of emitting empty output.

### Fixed
- Header/footer filtering was skipped on pages containing tables, leaking page numbers and running
  headers into extracted headings; the filter now runs regardless of table presence.
- Deterministic layout output — `ToUnicodeMap` and TrueType `cmap` tables now use `BTreeMap`, so
  repeated runs over the same document produce byte-identical Markdown (verified via two-run diff).
- Inline table-of-contents dot-leader cleanup threshold raised from 4+ to 8+ dots, avoiding false
  removal of legitimate dotted text.
- CI: npm publish uses `--access public` so `@iyulab/unpdf` publishes correctly.

## 0.7.0 — 2026-05-31

### Added
- **WebAssembly support** — `unpdf-wasm` crate with wasm-bindgen bindings (`PdfDocument`, `ParseOptions`,
  `parse()`, `parseWithOptions()`). Published to npm as `@iyulab/unpdf`.
- CI: `build-wasm` job (bundler + nodejs targets + wasm-pack test)
- CI/CD: `publish-npm` job in release workflow for automatic npm publishing

### Changed
- Node.js runtime upgraded from 20 → 24 in GitHub Actions (EOL: 2026-06-02)

### Fixed
- `ExtractionQuality::warning_message()` returned strings with a "Warning: " prefix, causing the CLI
  to output "Warning: Warning: …" when displaying quality diagnostics. Prefix removed; callers own the label.

## 0.6.4 — 2026-05-31

### Fixed
- CLI: `manual_contains` and `io_other_error` Clippy suggestions applied
- WASM: suppress `dead_code` warnings on wasm-bindgen struct fields

## 0.6.3 — 2026-05-12

### Added
- `RenderOptions::with_minimal_cleanup()`, `with_standard_cleanup()`, `with_aggressive_cleanup()`,
  `without_cleanup()` — convenience builder shortcuts (previously required `with_cleanup_preset(CleanupPreset::…)`)
- CLI `convert` completion now reports written file paths, image count → directory, and total word count

### Changed
- `MultiFormatWriter::finish()` now returns `WriteSummary { md_path, txt_path, json_path, image_count, word_count }`
  instead of `()`. Callers no longer need to call `image_count()` before `finish()`.
- CLI `convert`: word count displayed in completion summary (non-quiet mode)

### Fixed
- Pre-existing Clippy warnings cleaned up: `approx_constant` (tokenizer, backend tests),
  `single_match` (raw_parser_test), `map_or` and `print_literal` (realworld_test example)

## 0.6.2 — 2026-05-12

### Performance
- RwLock font caches — parallel reads on cache hit instead of exclusive Mutex lock;
  ~25% faster on multi-threaded parallel parsing workloads

## 0.6.1 — 2026-05-12

### Performance
- Sample-based image hash — O(1) per image (head+tail 64-byte sample) instead of O(size) full hash

## 0.6.0 — 2026-05-12

### Added
- **Image deduplication** — identical images (same bytes) are written to disk only once;
  duplicate references in Markdown reuse the canonical file path. Reduces output size for
  PDFs that repeat logos, watermarks, or decorative images across pages.

## 0.5.0 — 2026-05-09

### Added

- **Page boundary markers** — opt-in `<!-- page N -->` HTML comment markers at each page
  boundary in Markdown output. Markers are invisible in rendered Markdown but make it
  trivial to correlate extracted text with source PDF page numbers (regex: `<!-- page (\d+) -->`).
  - `PageMarkerStyle` enum (`None` | `Comment`) added to `RenderOptions`
  - `RenderOptions::with_page_markers(PageMarkerStyle::Comment)` builder method
  - CLI: `--page-markers` flag on `markdown` and `convert` subcommands
  - Works in both streaming (`convert`) and non-streaming (`markdown`) render paths
  - Default is `None` — existing output unchanged unless opted in

## 0.4.3 — 2026-04-14

Validation release for the 0.4.2 self-update fix + housekeeping.

### Changed
- CI/CD: `release.yml` gains a `cleanup-old-releases` job that deletes
  GitHub releases (and their git tags) beyond the 10 most recent after
  each successful release. Keeps the releases page and tag list
  bounded; aligns with CLAUDE.md's GitHub Actions storage policy.

## 0.4.2 — 2026-04-14

### Fixed
- `unpdf update` failed with `ZipError: unsupported Zip archive:
  Compression method not supported` when updating from 0.4.0/0.4.1 on
  Windows. Root cause: `self_update` 0.41's `archive-zip` feature alone
  enables **stored-only** (uncompressed) zip support. PowerShell's
  `Compress-Archive` (used by our release workflow) emits Deflate
  (method 8) archives, which requires the separate
  `compression-zip-deflate` feature. Added that feature to
  `cli/Cargo.toml::self_update`. `zip` crate now pulls `flate2` as
  verified in `Cargo.lock`.
- **Affects users on 0.4.0 / 0.4.1**: because the buggy self-update
  lives in the binary being replaced, those versions cannot update
  themselves past this fix. Install 0.4.2 manually (see README) and
  all subsequent `unpdf update` runs will work.

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
