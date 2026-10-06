# Changelog

## Unreleased

### Fixed

- **Tables in the default Markdown output are tables again.** The standard cleanup preset
  dropped every line without a letter or digit, which took the delimiter row (`| --- |`) out of
  each table — the pipe lines that remained are not a table to any Markdown reader — and also
  dropped rows whose cells are all empty. Soft line breaks no longer leave a trailing space
  before a block or a double space inside a sentence.
- A glyph whose `ToUnicode` entry is a "no mapping" sentinel is decoded through the font's
  `/Encoding` instead of vanishing (`314` read as `3 4`). Glyph names with a variant suffix
  (`one.SP`, `a.smcp`, `uni0041.alt`) resolve to their base character, per the Adobe Glyph List
  specification.
- Two-column pages are read column by column. Reading-order segmentation only split at
  channels five body-font sizes wide, so a typical 12–20 pt gutter never split and the columns
  were read line by line across; a caption or title spanning both columns now splits the page
  into bands instead of blocking the split.
- Stream-mode table detection no longer turns side-by-side text columns or a table of contents
  into a table, finds tables that fill only a small part of a page, and keeps a table whose
  header spans its columns over several lines.
- A numbered section title set in bold at body size (`5. The dynamics`, `3.1. Status`) is a
  heading, its level taken from the number's depth.
- **Filter chains are decoded.** A stream whose `/Filter` is an array — `[/ASCII85Decode
  /FlateDecode]`, which ReportLab writes for every content stream when its ASCII output is on —
  applies each filter to the output of the one before it, each with its own entry of a
  `/DecodeParms` array. Such a page used to come back with no text. `ASCII85Decode`,
  `LZWDecode` (with `EarlyChange`) and `RunLengthDecode` are supported, and the abbreviated
  names inline images use (`A85`, `AHx`, `LZW`, `Fl`, `RL`, `DCT`, `CCF`).
- An image behind a filter chain is extracted: `[/ASCII85Decode /DCTDecode]` yields the JPEG,
  where the ASCII85 text used to be stored as the image. Image samples are re-encoded as PNG
  whichever lossless filters produced them — or none: an unfiltered image was dropped as an
  unsupported format and is now a PNG resource like a `FlateDecode` one.
- A number on a line of its own that ends the line before — the second half of a page range
  (`432: 298-` / `306.`) — is kept as text. It was read as an empty numbered list item and
  dropped. A range broken at its hyphen is rejoined (`298-306`).
- The standard cleanup preset no longer removes line breaks after a number, a Korean
  syllable followed by `.` or `)`, or a hyphen at the end of a line, as though each were a
  list marker standing alone: consecutive references ran together (`866-75.23. Koorstra`)
  and Korean sentences lost the space between them (`…합니다.다음`). Only a marker that is the
  whole line is joined to the line below.
- A superscript or subscript in a table cell is read with the value it marks, after it
  (`0.31*`, not `* 0.31`), in ruled and unruled tables alike.
- **Python:** a document with no extractable text converts to an empty string instead of
  raising `UnpdfError` with kind 0 ("Unknown error") — an image-only page, for instance.
  The string results were declared as `c_char_p`, so an empty result looked like the null
  that signals failure, and no returned string was ever given back to `unpdf_free_string`:
  every conversion leaked its output. Strings are now copied and then freed, as the .NET
  binding already did.
- **.NET:** the binding works in apps that disable reflection-based `System.Text.Json`
  serialization — trimmed and Native AOT apps, and file-based apps (`dotnet run app.cs`).
  `ParseFile`/`ParseBytes` with `ParseOptions`, `GetExtractionQuality`, `GetPageStats` and
  `GetResourceIds` threw `InvalidOperationException` there. The binding now uses
  source-generated serialization and is marked `IsAotCompatible`, so the trim and AOT
  analyzers keep it that way.
- Document metadata (title, author, subject, keywords, …), outline titles and form field
  names and values written in a legacy CJK code page without a byte-order mark — CP949,
  GB 2312/GBK, Big5, Shift_JIS, EUC-JP, as Korean and Japanese tools commonly write the
  title — are decoded in that code page instead of coming back as Latin mojibake
  (`2024³â 3¿ù …` for `2024년 3월 …`). A character the producer damaged decodes as U+FFFD
  rather than as a plausible wrong one. Very short strings (a two-character title) can
  carry too little evidence to tell, and keep the single-byte reading.
- Text strings without a byte-order mark that are not UTF-8 are decoded as PDFDocEncoding,
  as the PDF specification defines them, instead of as Latin-1. The two differ in
  `0x18`–`0x1F` and `0x80`–`0xA0`, where PDFDocEncoding has the bullet, dashes,
  typographic quotes, `™` and `€`; those used to come back as control characters and
  were then dropped. Form field strings took a third path that turned every such byte
  into U+FFFD.
- A text string marked as UTF-8 (`EF BB BF`, PDF 2.0) is decoded as UTF-8.

### Changed

- **Breaking:** `parser::xycut::xycut_segment` takes an `XyCutConfig` instead of two gap
  arguments; `min_gutter` sets the narrower column-gutter cut (equal to `min_x_gap` disables it).

## 0.25.0 — 2026-10-03

### Changed

- **Breaking — one id per resource, the one the rendered output references.** Resource ids
  (`GetResourceIds` / `get_resource_ids` / `unpdf_get_resource_ids`, and the lookups that take
  them) are now the ids Markdown image references use, extension included: `page1_Im0.jpg`
  instead of `page1_Im0`. Before, the inventory and the rendered output named the same image
  differently, so joining an image reference to its metadata meant stripping the extension by
  hand. Ids are listed in reading order (page by page) instead of in no particular order.
- **Breaking — `Document::resources` is a method, not a field.** Each page holds the resources
  collected from it (`Page::images`); `Document::resources()` lists them across the document,
  `resource_count()` counts them and `get_resource(id)` looks one up. The JSON output no longer
  has a top-level `resources` object — the same entries are under each page's `images`.
  `Document::add_resource` is removed.

### Fixed

- Each image was decoded and converted twice when resources were extracted — once for the
  page and once for the document inventory. It is now done once: a 13-page report with
  22 megapixels of CMYK images parses in about 3.3 s instead of 6.5 s.
- Shared images collapse by reading order everywhere. The document inventory used to be
  deduplicated separately, in lexicographic id order, so the copy it kept could be a later
  page's (`page10_…` sorts before `page2_…`) while the rendered output pointed at the first.

## 0.24.0 — 2026-10-03

### Changed

- **Breaking — resource names resolve per resource scope.** `PdfBackend::page_fonts`,
  `decode_text` and `glyph_advances` take a `ResourceScope` (the page, plus the Form XObject
  being interpreted, if any) instead of a `PageId`; a `PageId` converts with `.into()` or
  `ResourceScope::page`. `ContentOp` gains a `form` field (build one with `ContentOp::new`),
  and `LayoutAnalyzer::page_op_counts` returns a `PageOpCounts` struct instead of a tuple.
  Inside a form the same name can mean a different font than on the page, so a page id
  alone cannot say which font a run uses.
- `image_op_count` (on `Page`, and in the page statistics of every binding) now counts image
  paints only, including images painted inside forms. It used to count every `Do`, so a page
  that paints its content through forms looked like an image-only page.

### Added

- `form_op_count` on `Page` and in the page statistics (C ABI `unpdf_page_stats`, C#
  `PageStats.FormOpCount`, Python `get_page_stats`): how many Form XObjects the page paints.
- `Resource::page` and a `page` field in resource info (`unpdf_get_resource_info`,
  `GetResourceInfo`, `get_resource_info`): the page the resource was collected from, 1-based
  like the page markers. Read the page from this field rather than from the resource id,
  whose format is not part of the contract.
- `PdfBackend::xobject`, which says what a `Do` paints in a given scope.

### Fixed

- **Content drawn through Form XObjects is extracted.** A page that paints its text inside a
  form (`q /Fm1 Do Q`) read as empty, and since every `Do` counted as an image the document
  was reported as a scan (`is_scan_pdf`). Forms are now interpreted where they are painted —
  their `/Matrix` composed onto the CTM, their names resolved in their own `/Resources`
  (falling back to the page's), nested forms followed, a form that paints itself drawn once.
  The same applies to everything else that reads the page's operators: table ruling lines,
  the page-covering-image signal and the scan heuristic. A form whose content stream cannot
  be decoded counts towards `undecodable_content_streams`, so strict parsing fails the page.
- Images inside Form XObjects are extracted as resources (a scanner often wraps the page image
  in a form). Their ids name the form they were found in: `page1_Fm1_Im1`.
- `/FlateDecode` images in `DeviceCMYK` and `Indexed` color spaces are extracted as PNG
  instead of being dropped as unsupported: CMYK converts to RGB, palette images are looked up
  in their palette (over a gray, RGB or CMYK base), and samples of 1, 2, 4 and 16 bits are
  read as well as 8. `/Decode` is honoured, so CMYK stored inverted comes out right. `Lab`,
  `Separation` and `DeviceN` images are still dropped and counted in
  `unsupported_image_count`. `RawXObject` gains `color` and `decode`.
- A page without `/Resources` of its own now lists the images of the resources it inherits
  from the page tree, as its fonts already did.
- The NuGet package, the PyPI package and the `unpdf-cli` crate now carry the license text
  (`LICENSE`) next to the `MIT` license expression. Only the npm package had it.

## 0.23.0 — 2026-10-02

### Added

- `StreamingRenderer::render_block_after(previous, block)` — renders a block as it appears
  after `previous` on the same page, including the blank line that separates a list from
  what follows it. A consumer driving its own page loop no longer has to reimplement that
  rule; the batch renderer, the streaming iterator and the CLI now share one definition of
  it instead of three.

### Fixed

- Text in one of the 14 standard fonts named without a `/Widths` array (allowed before
  PDF 1.5) is now measured by the font's own metrics instead of a per-character guess.
  A producer that draws such text one glyph at a time no longer gets words split after
  wide glyphs (`m echanism s`). The widths come from the Adobe Core14 AFM files; the
  common alternate names `Arial`, `TimesNewRoman` and `CourierNew` map to their
  standard equivalents. Other fonts without `/Widths` are still estimated.
- Text in a standard font with no `/Encoding` and no `/ToUnicode` now decodes through
  the font's built-in encoding instead of as Latin-1. Symbol text comes out as the
  Greek letters and math signs it draws (`α ≤ 120°`, where `a £ 120°` came out
  before), ZapfDingbats as dingbats (`✔`, not `4`), and the Latin faces through
  StandardEncoding (code 0x27 is `’`). A run full of control codes is still
  recognised as mis-decoded and suppressed.

## 0.22.0 — 2026-09-28

### Changed

- **Breaking — the CLI's default output directory now carries the input's extension and
  sits next to the input:** `document.pdf` converts into `document_pdf_output/` instead of `document_output/`.
  With the stem alone, two inputs differing only in extension (`report.pdf` beside a `report.docx` converted by a sibling tool) shared one
  directory and the second conversion silently overwrote the first one's `extract.md`
  and images. The extension is lowercased, since `report.PDF` and `report.pdf` are
  the same file where case is ignored. Pass `-o` to choose the directory explicitly, as
  before. The directory also moves from the current working directory to beside the
  input, matching the other two extractors in this family.

### Fixed

Pages whose content stream nests coordinate transforms, and pages with figures beside
the text, lost text outright. Browser-printed PDFs hit both on nearly every page.

- **Nested `cm` transforms were concatenated in the wrong order.** The new matrix must be
  applied before the current one (`cm × CTM`); the reverse agrees only while the CTM is
  the identity or a pure scale. A page that opens with a flipped, scaled CTM and then
  translates into a tall canvas placed every run far off the page, and a run holding only
  a number was then removed as a margin page number — table volumes, citation numbers and
  the digits inside headings (`C3`) went missing. Text and ruling lines share the one
  corrected transform.
- **Unrelated ruling lines were merged into one page-wide table.** Lattice detection
  treated every horizontal and vertical line on a page as a single grid, so a figure frame
  and a separator rule far below it spanned all the text between them, and every run
  whose left edge fell inside that span became a table cell. Headings lost their first or
  last words (`# oncept`) and lines beside figures were cut and reassembled as spurious
  tables. Grids are now inferred per cluster of lines that touch, and a border drawn twice
  a couple of points apart (a background box and its inset frame) snaps to one line at a
  3 pt tolerance instead of producing a sliver column.
- **Words drawn one glyph at a time were split after wide letters.** Glyph advance widths
  were never read, so where one glyph ended was a per-character guess; after a wide glyph
  (`m`, `W`) the guess fell short and the gap read as a word break —
  `Carbon concentrating m echanism s`, `W arburg`, table cells spelled out as `S t a g e`.
  The font's declared widths are now used (`/Widths` and `/MissingWidth` for simple
  fonts, `/W` and `/DW` for Identity-H composite fonts, `/FontMatrix` for Type 3), and
  the text state follows the specification: showing text advances the text matrix,
  line moves are relative to the start of the line, and `Tc`, `Tw`, `Tz`, `TL`, `TD`,
  `'` and `"` take effect and are saved and restored with the graphics state. Runs whose
  widths are measured are joined when they abut instead of by estimate. Fonts without
  declared widths keep the previous estimate. `T*` without a `TL` now moves by the
  specified default leading of 0 rather than an assumed 12.
- **A right-aligned run was glued to the text before it.** A TJ that opens with a large
  offset — how TeX sets `\hfill`, and how many producers place a right-aligned date or a
  tab stop — draws its first glyph far from the text origin, but the run was placed at the
  origin, right where the previous run ended: `**University of Somewhere***October 2022*`.
  Offsets before a TJ's first string now move where the run starts. They depend only on
  the font size, so this holds for fonts without declared widths too.
- **Subscripts and superscripts were split off their lines.** A script set smaller and
  shifted off the baseline by more than the same-line tolerance became a line — then a
  paragraph — of its own, and a superscript was emitted before the line it belongs to:
  `precursor to C` / `4` / `[35]` / `and a useful…`. A smaller line whose runs sit
  beside a neighbouring line's text, within a script's rise or drop of its baseline, is
  now folded into that line. A smaller line that runs across its neighbour's text (a
  byline under a title) stays its own.
- **Prose beside figures was detected as a table, and table cells were spelled out.**
  Table detection looked at runs exactly as the content stream drew them, so a column
  edge could fall at the start of a fragment in the middle of a word
  (`captures ener | gy from`), and a cell assembled from one-glyph runs read `S t a g e`.
  Both detectors now see coalesced runs, the same ones line grouping uses. Text-aligned
  (stream) detection also requires a column edge to be a channel of whitespace: an edge
  that runs of text cross on more than one row in ten is not a column, so justified prose
  whose word boundaries line up by chance is no longer split into cells.

### Added

- `PdfBackend::glyph_advances` — a backend can report each code's advance as the font
  declares it. Defaults to `None`, so existing backends keep compiling and keep the
  estimated extent.

## 0.21.0 — 2026-09-22

### Fixed

Real-world single-column documents were extracted as one glued run with their section
structure lost. Six separate causes, each of which alone produced visible damage:

- **Word spaces were dropped.** TJ kerning adjustments below a font's nominal space width
  were ignored, concatenating words — `Some Road, Somewhere` came out as
  `SomeRoad,Somewhere`. The Latin threshold drops from 200 to 120 thousandths of text
  space: measured word gaps sit at 167-200, while intra-word kerning and display-header
  letterspacing stay at or below 95. The Hangul (500) and CJK (never) rules are unchanged.
- **Reading order broke on right-aligned content.** `TextSpan.width` was always zero, so
  XY-Cut treated every span as a zero-width point and split single-column pages on any
  horizontal gap — a right-aligned date was pushed to the end of the page, away from the
  row it belongs to. Span width is now estimated per script from the font size.
- **Mixed line heights produced phantom headings.** Line grouping scaled its Y tolerance
  by the incoming span's font size, so a large-font line could absorb a small-font
  neighbour a few points away and the two averaged into a mid-size pseudo-heading.
  Tolerance now scales with the smaller of the two sizes.
- **Bold ALL-CAPS section headers were never promoted.** Classification looked only at
  font size, so a header set at body size stayed body text. A style-based rule promotes a
  short bold all-capitals line; mixed-case bold lines are deliberately excluded so job
  titles and names-in-bold stay body text.
- **Hard hyphens were deleted.** De-hyphenation joined any hyphen followed by a lowercase
  letter, so `Self-teaching` became `Selfteaching`. It now requires whitespace after the
  hyphen, which only line-break hyphenation has.
- **Inline emphasis was discarded.** Per-span bold/italic was flattened to plain text.
  The parser now emits styled runs and preserves them through same-row merges.

Markdown assembly, found while fixing the above:

- Emphasis markers wrapped surrounding whitespace (`**word **`), which abuts an adjacent
  emphasis run as `***` and is read as one bold-italic opener. Whitespace is kept outside
  the markers.
- A list was not separated from the paragraph or heading that follows it, and a
  sentence-ending newline before a list marker was doubled, turning every tight list
  loose.

### Changed

- `apply_text_style` moves to the shared `render::syntax` module. The batch and streaming
  renderers had a copy each; they can no longer drift on the rule.

### Known limitations

- The span width is a per-script average advance, not a glyph metric, so a wide estimate
  can still swallow the gap between two columns of one row. Where that happens to a bold
  cell beside an italic one, the markers abut as `***`.
- Label/value row layouts are not reconstructed as tables or definition lists.
- Styled runs fall back to plain text for blocks containing right-to-left text: BiDi
  reordering operates on the joined string and cannot be represented as independent runs.

### Internal

- Five tests were reported green without running: three CLI tests returned early on a
  fixture that has never been committed, and both streaming-equivalence tests looped over
  an empty corpus. All five now build their input in the test, and assert the parsed page
  count and non-empty text so an empty result cannot satisfy them either.

## 0.20.0 — 2026-09-22

### Changed

- **`ExtractMode` is replaced by `ParseOptions::extract_text: bool`** (builder `with_text`),
  and the enum is gone from every surface: the crate root re-export, the streaming options,
  the C ABI (`"extract_mode": "full" | "structure_only"` becomes `"extract_text": <bool>`),
  the .NET binding (`ExtractMode` enum and `ParseOptions.ExtractMode` become
  `ParseOptions.ExtractText`), and the Python options table. With `TextOnly` removed the
  enum had two variants, one of which meant "not the other" — that is a boolean, and naming
  it one makes it symmetric with the `extract_resources` flag beside it.
- **Structure-only no longer suppresses resource extraction.** `extract_text` and
  `extract_resources` are independent axes now; previously `StructureOnly` turned resources
  off as a side effect, so "structure, but keep the images" could not be expressed. Callers
  who relied on that coupling set `extract_resources` to `false` explicitly — which is what
  the default already is.
- `tests/structure_only.rs` pins the contract the parser already followed and nothing
  asserted: **the page survives with its number and dimensions, and its content blocks are
  not built.** No test anywhere exercised the mode before, which is how the sibling variant
  stayed dead through several releases.

- The built-in backend's `page_content` now fails when none of a page's content streams can
  be decoded. A page whose content array failed entirely used to return empty content, while
  a page with a single failed stream already failed — both now behave the same.

### Removed

- **`ExtractMode::TextOnly`**, and with it `ParseOptions::text_only()`, the `Unpdf::text_only()`
  builder, the WebAssembly `ParseOptions.textOnly()`, the C ABI's `"text_only"` value for
  `extract_mode`, and `Unpdf.ExtractMode.TextOnly` in the .NET binding. **No code ever
  compared against the variant** — a document parsed with it was byte-for-byte a document
  parsed with `Full`, on every surface that advertised it. Selecting text without resources
  is `extract_resources` (`ParseOptions::with_resources(false)`, `"extract_resources": false`
  over the C ABI), which is a separate axis and always was. `Full` and `StructureOnly` keep
  their meanings, and `StructureOnly` remains the only variant that gates anything.

  Passing `"text_only"` in the C ABI's options JSON is now rejected rather than silently
  treated as `Full`. The .NET `ExtractMode` enum loses a member, so `StructureOnly`'s
  ordinal shifts; the value crosses the boundary as a string, so only code depending on the
  numeric value is affected.

### Fixed

- `--image-dir` is reflected in the rendered image links. The images were written to the
  directory given, while the markdown kept pointing at `images/`, so choosing a directory
  produced a document whose every image link resolved to nothing. The prefix is now
  derived from the directory actually used -- relative to the output directory when it
  sits inside it, and as given when it does not.

- `ErrorMode::Strict` now fails a page whose content is split across several streams when
  one of those streams cannot be decoded. The undecodable stream was left out and the rest
  of the page reported as complete — while a page whose only content stream could not be
  decoded already failed under strict. Lenient parsing (the default) still keeps what the
  other streams hold.
- `ErrorMode::Strict` no longer fails a page that has no `/Contents` entry. The entry is
  optional — such a page (form fields or annotations only, for instance) is empty, not
  damaged — but it was reported as a parse error, and lenient parsing logged it as a
  failed page.

- A password given to `ParseOptions::with_password` (or `ConvertOptions::with_password`, or the
  C ABI's `password` field) is now offered to the document. It was carried through every
  options struct and stopped one call short of decryption, which only ever tried the empty
  password, so an encrypted document reported `Encrypted` whether or not the caller supplied a
  password and no caller could tell that theirs was ignored. A wrong password is now reported as
  `ErrorKind::InvalidPassword` (7), a discriminant published on every binding surface that
  nothing had ever produced; `Encrypted` (6) keeps its meaning of "no password was offered".
  The empty password is still tried first, so every document that opened before opens unchanged.

### Security

- The lockfile moves `rustls` to 0.23.45 (RUSTSEC-2026-0285: TLS 1.3 handshake messages accepted
  across encryption level boundaries, medium). It reaches this crate through `ureq` behind the
  `ai` feature, so the released CLI binaries and any build with that feature carried it; a
  library consumer resolving their own dependencies was never bound by this lockfile.

### Documentation

- The README states the default error mode and what a lenient default costs. Parsing has
  always been lenient by default, and a successful call can therefore return less than the
  document held; that was only discoverable from the source. The section also points at the
  `extraction_quality` fields that count each kind of loss, and notes that the sibling
  parsers default differently.

### Added

- `ErrorMode` and `ExtractMode` are re-exported at the crate root, alongside the
  `ParseOptions` they configure. Naming the enum required reaching into `unpdf::parser`
  while the struct it fills was already at the root.

- `PdfBackend::page_content_with_losses`, returning the page's content together with the
  number of content streams that could not be decoded (`PageContent`). It has a default
  implementation, so existing backends are unaffected.
- `ExtractionQuality::undecodable_content_streams` and `Page::undecodable_content_streams`:
  page content streams that could not be decoded. Lenient parsing (the default) leaves such
  a stream out and keeps the rest of the page — a page whose only content stream failed came
  back empty — and nothing in the output said so. The count reaches the C ABI
  (`unpdf_get_extraction_quality`, `unpdf_page_stats`), C# (`ExtractionQuality` /
  `PageStats.UndecodableContentStreams`), Python, `warning_message()` and `unpdf info`.

## 0.19.0 — 2026-09-11

### Changed

- **Breaking:** the shared plumbing — error-kind numbering, the C-ABI helpers, (feature `refine`)
  the markdown shape-refinement pass and (feature `ai`) the VLM capabilities — now comes from one
  crate, `unparser-shared`, instead of three (`uncore`, `unrefine` and `unparser-shared`
  together). `refine` and `RefineOptions` are still re-exported from this crate at the same
  paths, so code that names them through `unpdf` is unaffected. Code that named
  `unrefine::RefineOptions` directly has to switch, because `RenderOptions::refine` now holds
  `unparser_shared::refine::RefineOptions`. The refine pass itself now runs on `pulldown-cmark`
  0.13 / `pulldown-cmark-to-cmark` 22 rather than 0.12 / 18. Error kinds, the C ABI and the
  bindings are unchanged.

- The declared minimum supported Rust version is now 1.89. It had said 1.87, which the crate
  has not actually built on for some time — a dependency in the tree raised its own floor —
  so anyone taking the manifest at its word got a compile error rather than a clear refusal.
  1.89 is what `aes` 0.9 requires, and the manifest now says so. CI builds the workspace on
  exactly the declared version, so the two cannot drift apart again.
  The CLI crate, published alongside the library, now declares it as well — it named no
  minimum at all, which reads as "any version" to anyone checking.

- Updated `aes` (0.8 -> 0.9) and `cbc` (0.1 -> 0.2), moving the AES-128-CBC path onto
  `cipher` 0.5. The `rc4` path was already there, so the two halves of the decryption
  code now share one generation of the trait surface rather than pulling in both.

- Refreshed the declared minimum for `colored` (2 -> 3), `indicatif` (0.17 -> 0.18),
  `criterion` (0.5 -> 0.8), `png` (0.17 -> 0.18) and `md-5` (0.10 -> 0.11). The `indicatif`
  bump is the one that matters: 0.17 pulled in `number_prefix`, which is unmaintained
  (RUSTSEC-2025-0119), and 0.18 does not depend on it. `criterion` 0.8 deprecates its
  `black_box` re-export in favour of `std::hint::black_box`, and `png` 0.18 requires a
  seekable reader and returns the output buffer size as an `Option` -- both are test- and
  benchmark-only call sites here.

- Updated `self_update` to 1.3, whose default TLS backend is rustls rather than native-tls.
  Two things fall out of that, and both shrink the CLI's dependency surface:
  - The CLI no longer depends on `reqwest`. Its only use was setting one `Accept` header on
    the release-asset download, which 1.x no longer supports; the download now goes to the
    public release URL directly, which needs no header.
  - The vendored OpenSSL build for musl targets is gone. It existed solely for the
    `self_update` -> `reqwest` -> `native-tls` -> `openssl-sys` chain, and that chain no
    longer exists.
  `TempDir` and `self_replace` are no longer re-exported by `self_update`, so they are now
  direct dependencies, as its own documentation directs. The 0.41 tree also carried an
  optional `quick-xml` 0.23 with two open advisories against it; it was never compiled, but
  it was in the lockfile and had to be exempted from the audit. It is gone, and so is the
  exemption -- `cargo audit` is now clean with nothing ignored.


### Fixed

- An encryption revision this build does not implement (AES-256, that is AESV3 revisions 5
  and 6) is now reported as `UnsupportedVersion` rather than the catch-all kind. It was
  already refused rather than mis-decrypted, but arriving as `Other` made "this build does
  not do that" indistinguishable from any other failure. No discriminant was added: the one
  used has been public on all three binding surfaces from the start.

- Identical images are now one resource instead of one per page that draws them. A document
  that shares a single image XObject across its pages — a running-header logo is the ordinary
  case — was extracted as one resource entry, one copy in the page inventory and one output
  file *per page*, because resources are keyed by page. A 40-page document with one logo
  produced 40 of everything. Image references now point at the first occurrence in reading
  order, which is stable across runs; images whose bytes differ are untouched.

- With AI extraction on, an image that appears more than once is now sent to the vision model
  once, not once per occurrence. The running-header logo above still cost one model call per
  page even after it became one resource. Each of those calls also carried that page's
  surrounding paragraphs as context, so the logo's caption differed from page to page for no
  reason tied to the logo. A repeated image is now captioned once, without any single page's
  context, and every occurrence receives that caption. An image that appears only once is
  still captioned with its surrounding text, as before. If that one call fails, it counts as
  one fallback in `ai_fallback_count`, which counts calls, not occurrences.

- A document whose encryption key is shorter than the cipher accepts is now reported as
  undecryptable instead of aborting the process. Object keys are truncated to
  `min(file key length + 5, 16)` bytes, so a file key under 11 bytes yields a key AES-128
  cannot take, and the previous conversion panicked on that rather than returning.

### Added

- Known-answer tests for the AES-128-CBC path, covering the NIST SP 800-38A vector, a
  PKCS7-padded payload, a payload that is not a whole number of blocks, an IV with no
  ciphertext after it, and the short-key case above. The path had no direct coverage
  before, so a green suite said nothing about whether it still decrypted correctly.

## 0.18.0 — 2026-09-06

### Added

- **AI-assisted extraction (`ai` feature, on by default, inert until configured).**
  Pointing the library or CLI at an OpenAI-compatible endpoint lets a vision model
  describe images the text layer cannot represent — a full-page scan with no
  extractable text, a chart, a photo — and recover tables with their merged-cell
  structure intact, which Markdown table syntax cannot express. Without an endpoint
  configured, output is byte-identical to a build without the feature.
  - `ParseOptions::with_ai` runs the pass over the assembled document; the streaming
    API (`for_each_page`) deliberately does not, since it never materialises one.
  - `RenderOptions::with_ai_refine` adds an optional AI pass over rendered markdown,
    after the existing rule-based `refine`.
  - A failed call never fails extraction: the original content is kept and the
    fallback is counted in `ExtractionQuality::ai_fallback_count`, carried through the
    JSON and FFI surfaces and exposed to both bindings — `AiFallbackCount` on the .NET
    `ExtractionQuality` record, and the `ai_fallback_count` key Python already returns.
  - CLI: `--ai-base-url`, `--ai-api-key` (also `UNPDF_AI_API_KEY`), `--ai-model` and
    `--ai-image-scope` on `convert`, `markdown`, `text` and `json`, plus `--ai-refine`
    on the two that render markdown (`convert`, `markdown`). Supplying only some of
    the three required flags is an error rather than a silent fallback.
  - `convert` normally streams pages; configuring AI switches it to a buffered parse,
    because the passes need an assembled document. The outputs are unchanged — same
    files, same bytes — and a run without AI flags still streams exactly as before.
  - `AiConfig`, `ImageScope` and `AiRefineOptions` are re-exported from the crate
    root, so callers do not need a direct dependency on the shared crate to name the
    types the public API takes.
  - C ABI: `unpdf_parse_*_with_options` accept `ai_base_url`, `ai_api_key`, `ai_model`
    and `ai_image_scope`. A partial set is rejected with `UNPDF_ERROR_INVALID_ARGUMENT`
    rather than silently ignored, matching the CLI.
- **`unpdf_to_markdown_with_options`** — a JSON render-options entry point for the C ABI,
  mirroring the existing `unpdf_parse_*_with_options` convention. The flag bitmask reaches
  four settings and cannot carry the AI refine pass's credentials; this reaches
  `table_fallback`, `max_heading_level`, `cleanup_preset`, `page_selection`,
  `image_path_prefix`, `preserve_line_breaks`, `line_width` and `ai_refine` as well, none
  of which any C-ABI-based binding could previously set. `unpdf_to_markdown` is unchanged.

### Changed

- The `ai` and `refine` features are now listed in the README's feature table, and
  `--refine` is documented in the CLI's Markdown options — both were implemented but
  undocumented.

### Fixed

- The WebAssembly usage examples no longer open with `import init` and `await init()`. The npm
  package is built for bundlers, where the module initialises itself and no `init` export exists,
  so anyone following the first example stopped on its first line. The examples now import the
  functions directly, and the README states the bundler target once up front.

## 0.17.0 — 2026-08-30

### Added

- **Lattice-mode table detection** — tables with explicit ruling lines (drawn
  cell borders, not just aligned text) are now detected directly from the
  page's own vector graphics, complementing the existing text-alignment-based
  detection. This catches bordered tables the alignment heuristic previously
  missed or misclassified as plain paragraphs — sparse cells, short cell
  content, or otherwise low text-occupancy regions that nonetheless have a
  visible grid on the page. When a page has both a ruling-line grid and
  aligned-text evidence, the ruling-line grid takes priority for that region;
  the rest of the page is still processed by the existing detector unchanged.
  No new option is introduced — this runs automatically, the same as the
  existing table detection it complements.
- **List detection** — bulleted and numbered lines are now recognized as list
  items and rendered as real Markdown list syntax (`- item`, `1. item`)
  instead of plain paragraph text. Previously documented but unimplemented:
  the block type existed but was never produced by the parser, so every
  bullet or numbered line fell through to an ordinary paragraph. An ordered
  marker's number is read as printed on the page, not inferred or
  renumbered. Nesting is not detected — every item lands at the top level.

### Fixed

- **Table-of-contents dot leaders (`Chapter 1 .......... 6`) are now normalized
  structurally during layout analysis**, not only by the opt-in cleanup pass. The
  dot run is detected from the original text-fragment positions on the page before
  line-joining collapses their boundaries — a stronger, position-based signal than
  matching the dot count in already-joined text — and a trailing page number is
  folded into `(p.N)` in the same step. This closes a gap where the plain-text
  output (and any consumer that doesn't opt into the markdown cleanup pipeline) saw
  raw dot leaders untouched, and removes a source of duplicated page-number
  fragments the text-level regex pass could produce when leader runs were split
  across multiple fragments. The existing opt-in cleanup pass is unchanged and
  still applies when a table-of-contents line is extracted as a single merged
  fragment rather than separate title/leader/number fragments. The same
  normalization also now applies to a table-of-contents line that a page's
  table detector mistakes for a low-confidence table row (its title/leader/
  number fragments look table-row-shaped) and converts to plain text as a
  fallback — that fallback previously bypassed the normalization entirely.
- **A font whose embedded TrueType `cmap` table advertises only a (3,10) or (0,4)
  subtable — the standard pairing for full Unicode repertoire, including
  supplementary-plane characters — is now decoded correctly.** Subtable selection
  previously only recognized (3,1)/(0,3)/(0,x) pairings, so a lone (3,10)/(0,4)
  subtable was skipped entirely and its text fell through to suppression. Worse,
  when a font shipped both a BMP-only (3,1) subtable and a full-repertoire (3,10)
  one — common for fonts that need to cover characters above U+FFFF — the BMP-only
  subtable was always chosen, silently dropping any supplementary-plane character.
  (3,10)/(0,4) now take priority, per the OpenType spec's own recommendation.
- **A malformed embedded TrueType `cmap` format-12 subtable with an inverted
  character range (`endCharCode < startCharCode`) could crash the process** via an
  integer underflow instead of being skipped as invalid. Such a subtable is now
  rejected before the arithmetic that previously panicked.

## 0.15.0 — 2026-08-28

### Added

- **`ParseOptions` reachable from every C-ABI-based binding (C#, Python)** — the C
  ABI's parse entry points previously took no arguments beyond the document source,
  so no `ParseOptions` field (resource extraction, page selection, password,
  extraction mode, and more) was reachable from C# or Python, even though the Rust
  API and CLI already supported all of them. New C ABI functions
  `unpdf_parse_file_with_options` / `unpdf_parse_bytes_with_options` accept a JSON
  options payload; existing `unpdf_parse_file` / `unpdf_parse_bytes` are unchanged.
  C#: `ParseFile`/`ParseBytes` gained an optional `ParseOptions?` parameter. Python:
  every parsing function gained an optional `options: dict` parameter, and
  `get_resource_ids` / `get_resource_info` / `get_resource_data` — previously
  unreachable from Python entirely — are now exposed. `unpdf-wasm`'s `ParseOptions`
  also gained `withResources`/`withMinImageDimension`, closing a parity gap against
  its own `parseWithOptions` entry point.
- **`/FlateDecode` embedded images are re-encoded as PNG instead of being dropped.**
  Only `DCTDecode` (JPEG) and `JPXDecode` (JPEG2000) image XObjects were ever
  materialized into the resource inventory; `/FlateDecode` — the filter most
  Office-export-to-PDF screenshots and diagrams use — was always classified as an
  undecoded raw format and discarded, even with `extract_resources: true`. 8-bit
  `DeviceGray`/`DeviceRGB` images (including `ICCBased` color spaces, resolved by
  their embedded profile's component count) are now reconstructed into a valid PNG
  container. Other color spaces (Indexed, CMYK, ...) are still dropped, but now
  counted in the new `ExtractionQuality.unsupported_image_count` field — surfaced
  through `unpdf_get_extraction_quality`'s JSON and the C#/Python bindings — so a
  caller can tell "no image" from "image present but not extractable".

### Fixed

- **Resource extraction (`ParseOptions.extract_resources`) ignored
  `min_image_dimension` and included undecodable image formats** when parsing
  through the primary `parse()` API (i.e. every consumer of `parse_file`/
  `parse_bytes`, including the CLI's own resource-extraction flag) — a second,
  independently-drifted copy of the resource-collection logic never applied the
  filtering the streaming API already did. Both paths now share one filter.

## 0.14.0 — 2026-08-24

### Added

- **`PageStats.SuppressedTextRuns` (C#) / `suppressed_text_runs` (Python `get_page_stats`)** —
  the per-page count of text runs the font decoder could not read and discarded. The
  C ABI (`unpdf_page_stats`) has reported this alongside `ocr_text_suppressed` since
  0.12.0; the document-level total (`ExtractionQuality.SuppressedTextRuns` /
  `extraction_quality()["suppressed_text_runs"]`) was already exposed, but neither
  binding surfaced the same signal per page — so a consumer discriminating causes
  across pages in a mixed-quality document (e.g. some pages losing text to an
  unresolvable font while others are clean) had no way to attribute the loss to a
  specific page, unlike the identically-shaped `OcrTextSuppressed` signal. The
  document-level total is unchanged and remains the sum of the per-page counts.

## 0.13.0 — 2026-08-20

### Added

- **Markdown shape-refinement pass** (`RenderOptions.refine`, CLI `--refine`, C-ABI
  `UNPDF_FLAG_REFINE`, C# `MarkdownOptions.Refine`, Python `to_markdown(..., flags=
  UNPDF_FLAG_REFINE)`) — a lossless, idempotent post-processing pass
  ([`unrefine`](https://crates.io/crates/unrefine)) that normalizes table shape,
  ordered-list numbering, link/image path separators, frontmatter formatting, and
  assigns a GitHub-compatible slug to every heading. Never deletes visible text. Off
  by default — existing output is unaffected.
- Gated behind a new `refine` cargo feature (on by default). `unpdf-wasm` excludes
  it: `unrefine` needs a fresh `pulldown-cmark`/`pulldown-cmark-to-cmark` that unpdf
  otherwise has no use for — with the feature off, the wasm bundle delta is
  negligible (+54 bytes).

## 0.12.2 — 2026-08-20

### Fixed

- **A link or image destination containing a space is now wrapped in `<...>`** —
  previously a raw space in the destination (e.g. a resource path built from a
  filename with a space in it) produced Markdown that is not valid CommonMark
  outside `<...>` at all, so a downstream consumer read the literal brackets and
  parentheses as text instead of a link. A destination containing a literal `<`
  or `>` is now backslash-escaped rather than percent-encoded, so the escaped
  target still resolves to the original path.

## 0.12.1 — 2026-08-20

### Fixed

- **Batch rendering (`to_markdown()`) now emits a real image link for an image block** —
  previously it emitted only an HTML comment placeholder and never used the image's resource
  id, so a document with images produced no usable image reference through the batch API at
  all. The streaming renderer (and everything built on it) already did this correctly; batch
  now matches.
- **Streaming rendering now honors `table_fallback` for a table with merged cells** —
  previously it always rendered a plain Markdown table regardless of the option, silently
  losing the merge structure that the batch renderer preserved via HTML fallback. Both
  renderers now share one table-rendering implementation, so they cannot diverge again.

## 0.12.0 — 2026-08-06

### Added

- **Extraction quality reports text runs the decoder discarded** —
  `ExtractionQuality::suppressed_text_runs` (C# `SuppressedTextRuns`, Python
  `suppressed_text_runs`, and per page in `unpdf_page_stats`).

  A run is one text string handed to the decoder — a `Tj` operand, or a single element
  of a `TJ` array.

  When a font's character codes cannot be resolved, the run is discarded rather than
  emitted as mojibake. That policy is unchanged — what changes is that the extraction
  no longer reports plain success afterwards. Content the document held and the output
  does not is now counted, so a consumer can tell "the document did not say this" from
  "we could not read it". Any non-zero value means the extraction is incomplete.

  Counted in runs rather than characters: the discarded text was never decoded, so its
  length is unknowable. Treat non-zero as a boolean "incomplete" signal; the magnitude
  only compares documents to each other.

  `unpdf info` reports the count on its own line when non-zero, alongside the page-loss
  line and for the same reason: `--quiet` silences warnings, and a diagnostic command
  must not hide missing content because the caller asked for less noise.

  `warning_message()` reports it ahead of the empty-text warning, whose text lists
  "unsupported font encoding" among several guesses — when runs were suppressed, that
  cause is observed rather than guessed.

- **Page markers reach the C ABI and the language packages** —
  `UNPDF_FLAG_PAGE_MARKERS` (`8`), C# `MarkdownOptions.PageMarkers`, Python
  `unpdf.UNPDF_FLAG_PAGE_MARKERS`. The option has been in the core renderer and the CLI
  since 0.5.0 but was reachable from neither binding, so a caller outside Rust had no way
  to keep track of which page a passage came from — the common need behind chunking a
  long document.

- **Python exports the markdown flags.** `to_markdown` has always taken a `flags`
  argument, but the constants naming its bits lived in a private module, so the only way
  to use it was to hard-code integers or import from `unpdf._native`.
  `UNPDF_FLAG_FRONTMATTER`, `UNPDF_FLAG_ESCAPE_SPECIAL` and `UNPDF_FLAG_PAGE_MARKERS` are
  now importable from `unpdf`.

### Changed

- **Flag bit `4` is retired.** It was published on all four surfaces as a
  paragraph-spacing option — C `UNPDF_FLAG_PARAGRAPH_SPACING`, C#
  `MarkdownOptions.ParagraphSpacing`, Python `UNPDF_FLAG_PARAGRAPH_SPACING` — and
  documented as "add extra spacing between paragraphs", but it never reached the
  renderer: setting it produced exactly the default output. The C# property and the two
  constants are removed; C# code that sets `ParagraphSpacing` stops compiling, and its
  behaviour was already whatever the library does without it.

  The bit value itself is **not reused**. A caller still passing `4` to the C ABI gets the
  default rendering, which is what it always got.

- **`PdfBackend::decode_text` returns `DecodedText`, not `String`** (breaking for code
  implementing the trait; no effect on library users). A decoder that gives up on a run
  reports it via `DecodedText::suppressed` — an empty string could not distinguish
  "discarded this run" from "this run held no text", and that difference is the whole
  of the new diagnostic.

## 0.11.0 — 2026-07-31

### Upgrade notes

Two things to check before upgrading. Neither is a bug fix, and both are easy to miss
in the lists below.

- **Minimum supported Rust version is now 1.87** (was 1.75). This follows the shared
  `uncore` crate the FFI plumbing moved onto.
- **Three public options that never did anything have been removed** — `HeadingConfig` (and
  `RenderOptions::heading_config`), `CleanupOptions::remove_headers_footers` and
  `CleanupOptions::detect_mojibake`. Code that sets them stops compiling; its behaviour was
  already whatever the library does without them. See **Removed** below.
- **Python: the first parameter of every function is now named `source`, not `path`.**
  A positional call is unaffected — `to_markdown("a.pdf")` keeps working. A call that
  passes it by keyword (`to_markdown(path="a.pdf")`) must be renamed. The parameter now
  accepts the PDF's own bytes as well as a path, which is why it is no longer called
  `path`.

### Added
- Structural-integrity reporting on `ExtractionQuality`, so a caller can tell a
  document that says what it says from what survived a damaged file. A PDF whose
  cross-reference table outlives the objects it points at parses *successfully* over
  a short page set — previously indistinguishable from a genuinely shorter document:
  - `pages_incomplete` — the one field to branch on: pages are known to be missing.
  - `declared_page_count` — the page count the document declares (root `Pages`
    `/Count`), or absent when that declaration was unreadable too.
  - `unresolved_page_nodes` — unreadable page-tree *nodes*. Non-zero means
    incomplete; it is not a count of lost pages, because one unusable intermediate
    node drops its whole subtree.
  - `skipped_object_count` — objects the xref table named that could not be loaded.
    A damage indicator only: most (fonts, annotations, metadata) cost no page.
  - Surfaced everywhere the quality struct already reaches: `warning_message()` (and
    so the CLI), the JSON output, FFI `unpdf_get_extraction_quality`, and the C# and
    Python bindings. All fields are additive and default to the intact case.
  - `unpdf info` now marks a short page set on the `Pages` line, so `--quiet` cannot
    hide it.
- WASM: `PdfDocument.extractionQuality()`, matching what the other bindings expose.
- Python: every function now accepts a path *or* the PDF's own bytes. Paths go through
  `os.fspath`, so `pathlib.Path` works; bytes are parsed in memory through the native
  bytes entry point, with no temporary file. The types are unambiguous (`str` is always
  a path, `bytes` always content), and the accepted union is exported as `PdfSource`.
  The first parameter is named `source` rather than `path` — a positional call is
  unaffected; a caller passing it by keyword (`to_markdown(path=...)`) must rename.
  `is_pdf` and `get_page_count` keep reporting an unparsable PDF by return value
  (`False` / `-1`), while a wrong-*typed* argument now raises `TypeError` — a caller
  bug is not an unparsable PDF, and it previously surfaced as an `AttributeError`
  from the internals.

### Fixed
- **Enumerated and bulleted list items were promoted to headings.** Heading detection refused
  to promote a line opening with one of ten bullet glyphs, and enclosed enumerations
  (①, ⒈, ㈀, ❶ …) were not among them — so a document setting its clause lists in a display
  face turned every item into a heading. Documents that number clauses this way tend to do so
  throughout, which put the noise everywhere at once. The exclusion now covers those three
  Unicode ranges, and the glyph list grew from ten to thirty-three: among others `○ ● ■ □ ◆ ★
  → ► ▹ ◁ ◀ ◃ ◂ ㆍ ㅇ ∙ ◼ ◾` and the en/em dashes. A line starting with any of them is now
  read as a list item, so one that was previously emitted as a heading no longer is.
- **The Markdown escaping and Roman-numeral helpers existed twice**, once per rendering path,
  so batch and streaming output could drift apart wherever one copy was changed and the other
  was not. Both paths now use the same implementation. No output change on its own.

- Extracted text carried C0/C1 control characters straight into the output. PDF string
  literals may legally contain control bytes (`\000` is a valid octal escape) and some
  producers leave NUL padding in the text layer, so such a file is not damaged — but
  reporting the byte back as text wrote a raw NUL into Markdown and text output, and at
  the C ABI the string could not be transported at all: the caller lost the whole page,
  or, for document metadata and outline titles, received "absent" with no error at all.
  Text now passes through a single sanitizing step (page content, metadata, outline
  titles, form field names and values) that removes control characters other than
  `\n`, `\r` and `\t`.
  - Removal rather than `U+FFFD` substitution, because `replacement_char_count` means
    *font decoding failed* and feeds `is_good()`; substituting for transport reasons
    would report clean documents as badly decoded.
  - Sanitizing happens on the way out, after any decode-quality judgement: control
    character *density* is how a mis-decoded run (a CID font read as Latin-1) is
    recognised, and cleaning first would erase that evidence and turn "emit nothing"
    into "emit garbage-derived letters". Such runs stay suppressed.
  - The `InvalidOutput` guard at the ABI boundary is kept as a backstop; it should now
    be unreachable, and reaching it again means the invariant was broken upstream.
- Form field names and values were decoded one byte at a time, with no UTF-16 handling
  at all. A PDF text string is either PDFDocEncoded or UTF-16BE (PDF 1.7 §7.9.2.2), and
  AcroForm producers write these as UTF-16BE *with* the byte-order mark — which is not
  valid UTF-8, so lossy decoding turned the mark itself into two `U+FFFD`. On a real
  form the great majority of field strings are written that way, and their names came
  back as `<?><?>topmostSubform[0].<?><?>Page1[0].<?><?>Step1a[0]…`, one pair per path
  segment; the interleaved NULs were removed by the output-hygiene pass, which recovered
  the ASCII by accident and hid the rest. Field strings now take the UTF-16BE reading,
  so the name is the name.
  - The unmarked form is also handled, but only where it cannot be mistaken: every
    even-offset byte zero, which is an all-ASCII string. A looser rule accepts
    `CHAP\0TER` — plain text with one stray NUL, which is what damaged content looks
    like — and rewrites it as `䍈䅐T䕒`.
  - Consequently an unmarked UTF-16BE string mixing ASCII with anything above `U+00FF`
    keeps the single-byte reading, as does one with no character below `U+0100` (those
    carry no NUL at all, and `Café` reads as valid UTF-16BE too). The mark the
    specification already requires resolves both.
  - Document metadata and outline titles already decoded the marked form; they gain the
    unmarked all-ASCII one.
- Page-tree traversal recursed without cycle detection or a depth bound, so a
  damaged or hostile PDF whose `Pages` node reached back to an ancestor could exhaust
  the stack and abort the process. Replaced with an iterative walk over a visited
  set. (`PdfBackend::outline` already required implementations to guard cycles; only
  the page tree was exempt.)
- An unreadable object stream (`ObjStm`) silently discarded every object it carried;
  those objects are now counted, so "one object skipped" and "a chapter missing" are
  no longer reported identically.
- The repository README's C# section documented a `Pdf` static class and `PdfOptions`
  that do not exist — the same defect fixed in the NuGet README in 0.10.0, still
  present here. Rewritten against the real handle-based `UnpdfDocument` API.
- Python: passing a `pathlib.Path` raised a bare `AttributeError` instead of working.
- README: the Python section showed `to_markdown(pdf_bytes)`, which raised (the
  binding was path-only at the time; bytes are now supported), and read the page count
  from a `page_count` key that `get_info` never returns (it is `section_count`).
- README: `convert --keep-ocr-text` was missing from the options table — the flag the
  low-confidence-OCR warning tells the user to reach for.
- `unpdf_get_title`/`unpdf_get_author`: a value containing a NUL byte silently
  returned `null` with no error at all. Now reports `InvalidOutput` at the ABI
  boundary, matching every other string-returning entry point.
- `unpdf_section_count`/`unpdf_resource_count`: did not clear the last-error slot on
  entry, so a stale error `kind` from an earlier failed call could still be read
  after a later, successful call to these two functions.
- **The CLI update notification went to stdout, corrupting piped output.** `md`, `json`
  and `text` emit document data on stdout, so the notification line landed in the middle
  of it — `unpdf json … | jq .` failed to parse. It now goes to stderr, where it still
  appears in an interactive terminal.

### Changed
- Internal: the FFI last-error/panic-guard plumbing now runs on the shared `uncore`
  crate (thread-local slot, panic guard, boundary-reason helpers) instead of a
  hand-rolled implementation duplicated across the `un*` extraction family. Every
  `ErrorKind` discriminant and every exported C symbol's name and signature stay
  exactly as they were; see Fixed above for the two behavior changes this swap
  surfaced. `rust-version` raised to 1.87 to match `uncore`'s MSRV.
- Internal: each C ABI entry point is now assembled from `uncore`'s scaffold macros
  rather than a hand-written preamble repeated at every one of them. No observable
  change — the exported symbol list is byte-identical and every existing test passes
  unmodified.

### Removed
- **`HeadingConfig` and `RenderOptions::heading_config`**, with the two builder methods that
  set them (`with_heading_config`, `with_heading_analysis`). Nothing read any of it: heading
  level is decided during layout analysis, from the absolute point difference against body
  text and the rank of that size among the sizes actually observed on the page. The
  `h1_min_ratio` / `h2_min_ratio` fields describe a ratio-based scheme that was never how
  this library works, and `korean_patterns` promised pattern matching that does not exist
  here at all. Tuning knobs for the real algorithm need a design of their own rather than a
  struct that reads as if it already provides them.
- **`CleanupOptions::remove_headers_footers` and `CleanupOptions::detect_mojibake`.** Neither
  gated anything — the cleanup pass never read either field — so `CleanupPreset::Standard`
  and `Aggressive`, which set them, advertised header/footer removal and mojibake handling
  that never happened. An option is a promise that the pipeline acts on it.
- `docs/superpowers/` — plan and design notes for a feature shipped in 0.5.0, kept in
  the published docs tree with nothing referencing them.

## 0.10.0 — 2026-07-28

### Added
- Structured error classification, so a caller can tell *why* extraction failed
  without matching on message text:
  - `Error::kind()` returning a stable `ErrorKind` — one variant per `Error`
    variant, with explicit discriminants that are part of the public contract.
  - FFI: `unpdf_last_error_kind()`, written in lockstep with `unpdf_last_error()`
    (every call that records a message records a kind; a successful call clears
    both). Values 1..17 mirror `ErrorKind`; 100+ are FFI-boundary reasons — a null
    or non-UTF-8 argument, a caught panic, output holding an interior NUL byte.
    `UnpdfErrorKind` is declared in `bindings/unpdf.h`.
  - C#: `UnpdfException.Kind` (`UnpdfErrorKind` enum).
  - Python: `UnpdfError` with a `kind` attribute and an `ErrorKind` enum. It
    subclasses `RuntimeError`, which is what this package raised before, so existing
    `except RuntimeError` handlers keep working.
  - This completes the diagnostic surface 0.9.0 started: `extraction_quality` and
    `page_stats` explain a *successful* parse that produced no text, while error
    kinds explain a parse that failed at all — previously reachable only by string
    matching.

### Fixed
- C#: error messages were decoded as ANSI while the native side emits UTF-8, so
  messages embedding a non-ASCII file path came back mangled.
- C#: the NuGet package README documented a `Pdf` static class that does not exist —
  every sample in it failed to compile. Rewritten against the real `UnpdfDocument`
  API.
- Python: the README's install command named the import name (`unpdf`) rather than
  the distribution (`unpdf-markdown`).

### Documentation
- README (both the repository and the per-binding ones) now covers failure handling
  and, for Python, the `get_extraction_quality` / `get_page_stats` surface added in
  0.9.0.
- The searchable-scan caveat on `page_stats` (a page image with an invisible OCR
  layer reports `text_op_count > 0`; combine with `ocr_text_suppressed`) now appears
  in the C# and Python API docs as well, not only in `unpdf.h` and the README —
  consumers reading IDE tooltips were being shown the naive two-term rule.

## 0.9.0 — 2026-07-23

### Added
- Introspection surface for telling a scanned (image-only) page apart from a genuinely
  blank page — the "why is my extraction empty?" question consumers could not answer:
  - `Page::text_op_count` / `Page::image_op_count`: per-page counts of text-showing
    operators (`Tj`/`TJ`/`'`/`"`) and XObject `Do` invocations, gathered during the
    existing content-stream traversal at no extra cost. `text_op_count == 0` with
    `image_op_count > 0` identifies an image-only page; both 0 means a blank page.
    Serialized in JSON output (omitted when 0). Works per page, so mixed documents
    (text + scanned pages) are classified page by page — the document-level
    `is_scan_pdf` flag cannot do that.
  - FFI: `unpdf_get_extraction_quality` (JSON of `ExtractionQuality`, including
    `is_scan_pdf` and `suppressed_ocr_pages`) and `unpdf_page_stats` (per-page counts
    plus `ocr_text_suppressed`).
  - C#: `UnpdfDocument.GetExtractionQuality()` / `GetPageStats(int)` with typed
    `ExtractionQuality` / `PageStats` DTOs.
  - Python: `get_extraction_quality(path)` / `get_page_stats(path, page_number)`.

### Documentation
- `unpdf_resource_count` semantics: it counts the extracted-resource inventory, which
  the FFI parse path leaves empty (resource extraction defaults to off since 0.4.0),
  so it returns 0 there — it was never a count of images referenced by content
  streams. C#/Python docs updated accordingly; use the new page stats for scan
  detection instead.

## 0.8.0 — 2026-07-21

### Security
- Bump `crossbeam-epoch` 0.9.18 → 0.9.20 (RUSTSEC-2026-0204: invalid pointer dereference in the
  `fmt::Pointer` impl for `Atomic`/`Shared`). A transitive dependency of `rayon`; lifted within the
  existing semver range, so it is a lockfile-only change.

### Added
- Low-confidence OCR text layers are dropped. A searchable scan carries its OCR result
  as invisible text over the page image; when the OCR recognised nothing readable — a
  drawing, a stamp, a poor scan — the page now yields the image alone instead of a wall
  of meaningless characters. A page qualifies only when a raster image covers it, the
  text is drawn in rendering mode 3, *and* the text has no word structure, so visible
  text is never affected. Reported via `ExtractionQuality::suppressed_ocr_pages`; opt
  out with `ParseOptions::with_ocr_suppression(false)` or `unpdf convert --keep-ocr-text`.
- Predefined CJK CMap support for Type0 fonts without a ToUnicode map: `KSC-EUC`,
  `KSCms-UHC` (Adobe-Korea1), `90ms-RKSJ` (Adobe-Japan1), `GBK-EUC` (Adobe-GB1) and
  `ETen-B5` (Adobe-CNS1), in both writing modes, plus the `UniXX-UCS2`/`UniXX-UTF16`
  CMaps (decoded as UTF-16BE). Code→CID tables are generated at build time from the
  Adobe `cid2code.txt` files already shipped for CID→Unicode lookup. Decoding agrees
  with the vendor codecs (EUC-KR, CP949, CP932, GBK, Big5) on 98.7% of mapped codes;
  the remainder are punctuation where Adobe's character collection and the vendor
  codec legitimately disagree (e.g. `⋯` vs `…`).

### Fixed
- `unpdf convert` computed extraction-quality warnings but never printed them.
- CID→Unicode lookup picked the first code point listed for a CID, which is sometimes
  a compatibility duplicate — Adobe-GB1 CID 3795 resolved to the Kangxi radical `⽂`
  instead of `文`. Radicals, compatibility ideographs, Hangul fillers and U+2329/232A
  are now skipped when the CID has an ordinary code point. Affects every CID-keyed
  font, including the Identity-H path.
- Composite (Type0/CID) fonts with no usable CMap no longer fall back to byte-wise Latin-1 decoding,
  which produced mojibake (`°Ë ,¥õ ²ô`). The guard previously covered only `Identity-H/V`, so Type0
  fonts using a predefined CMap (e.g. `/Encoding /KSC-EUC-H` in scanner OCR layers) leaked garbage
  text. Such fonts now yield no text. Predefined CJK CMap support itself is tracked separately.
- Extracting the same PDF twice produced different output. PDF dictionaries were `HashMap`s, whose
  iteration order is seeded per instance, so a page's XObjects came out shuffled on every run:
  images were appended to the Markdown in a different order, and — because image dedup keeps the
  first occurrence as canonical — the same picture was written under a different filename each
  time. Dictionaries are now `BTreeMap`s, so every dictionary traversal is ordered by key. Verified
  byte-identical across repeated extractions of 38 documents.

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
