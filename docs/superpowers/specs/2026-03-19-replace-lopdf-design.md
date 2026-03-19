# Design: Replace lopdf with Custom PDF Parser

**Date**: 2026-03-19
**Issue**: `claudedocs/issues/ISSUE-unpdf-20260303-replace-lopdf.md`
**Status**: Approved

## Goal

Remove the `lopdf` dependency entirely and replace it with a lightweight, purpose-built PDF parser optimized for unpdf's text extraction needs. The only new external dependency is `flate2` for FlateDecode decompression.

## Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Execution strategy | Single branch, Phase 1-4 continuous | Avoid intermediate states with dual maintenance |
| Parser implementation | From-scratch, PDF spec-based | Full control, no unnecessary abstractions |
| Encryption | Detect only, no decryption | Same as current behavior; separate future issue |
| Font code placement | Separate `font.rs` module | Separation of concerns between parser and font logic |
| Approach | Hybrid: abstract first, then bottom-up parser | Keeps tests passing throughout development |

## Module Structure

```
src/parser/
  raw/                      # Custom PDF parser
    mod.rs                   # pub API: RawDocument
    tokenizer.rs             # PDF object parser (bytes -> PdfObject)
    xref.rs                  # xref table/stream/ObjStm/incremental
    document.rs              # RawDocument: object resolution, page tree, metadata
    stream.rs                # FlateDecode decompression (flate2)
    content.rs               # Content stream parser (operators/operands)
  font.rs                    # ToUnicode CMap, TrueType cmap, encoding (extracted from backend.rs)
  backend.rs                 # PdfBackend trait (extended) + RawBackend impl
  pdf_parser.rs              # Uses PdfBackend trait only (no raw_doc())
  layout.rs                  # (unchanged)
  ...
```

## PdfBackend Trait (Extended)

```rust
pub trait PdfBackend {
    // --- Existing 5 methods (retained) ---
    fn pages(&self) -> BTreeMap<u32, PageId>;
    fn page_fonts(&self, page: PageId) -> Result<Vec<BackendFontInfo>>;
    fn page_content(&self, page: PageId) -> Result<Vec<u8>>;
    fn decode_content(&self, data: &[u8]) -> Result<Vec<ContentOp>>;
    fn decode_text(&self, page: PageId, font_name: &[u8], bytes: &[u8]) -> String;

    // --- New: metadata ---
    fn metadata(&self) -> PdfMetadataRaw;

    // --- New: page info ---
    /// Returns (width, height) in points. Falls back to Letter size (612, 792)
    /// if MediaBox is absent or unparseable — this is intentional to match
    /// current behavior and avoid breaking callers on malformed PDFs.
    fn page_dimensions(&self, page: PageId) -> (f32, f32);

    // --- New: outline ---
    /// Returns the outline tree with cycle detection (visited set + depth limit).
    /// The backend is responsible for safe recursive traversal.
    fn outline(&self) -> Result<Vec<RawOutlineItem>>;

    // --- New: resources (images) ---
    fn page_xobjects(&self, page: PageId) -> Result<Vec<RawXObject>>;
}
```

### Supporting Types

```rust
pub struct PdfMetadataRaw {
    pub version: String,
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub keywords: Option<String>,
    pub creator: Option<String>,
    pub producer: Option<String>,
    pub creation_date: Option<String>,
    pub mod_date: Option<String>,
    pub encrypted: bool,
}

pub struct RawOutlineItem {
    pub title: String,
    pub page: Option<u32>,
    pub level: u8,
    pub children: Vec<RawOutlineItem>,
}

pub struct RawXObject {
    pub name: String,
    pub subtype: String,
    pub data: Vec<u8>,
    pub filter: Option<String>,  // None if Filter key absent (per ISO 32000)
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub bits_per_component: Option<u8>,
    pub color_space: Option<String>,
}
```

## Custom Parser Internal Design

### Core Types

```rust
// raw/tokenizer.rs
pub enum PdfObject {
    Null,
    Bool(bool),
    Integer(i64),
    Real(f64),
    Name(Vec<u8>),
    Str(Vec<u8>),
    Array(Vec<PdfObject>),
    Dict(PdfDict),
    Stream(PdfStream),
    Reference(u32, u16),
}

pub type PdfDict = HashMap<Vec<u8>, PdfObject>;

pub struct PdfStream {
    pub dict: PdfDict,
    pub raw_data: Vec<u8>,
}

// raw/document.rs
pub struct RawDocument {
    objects: HashMap<(u32, u16), PdfObject>,
    trailer: PdfDict,
    version: String,
}
```

### Module Responsibilities

| Module | ~LOC | Responsibility |
|---|---|---|
| `tokenizer.rs` | ~400 | Parse PDF byte stream into `PdfObject` values. Handles all 10 object types including indirect objects. |
| `xref.rs` | ~450 | Parse xref tables (traditional + stream), ObjStm, incremental updates via `/Prev` chain. Highest complexity. |
| `document.rs` | ~200 | Build `RawDocument` from xref. Object resolution, reference tracking, Catalog -> Pages tree traversal. |
| `stream.rs` | ~60 | FlateDecode via `flate2::read::ZlibDecoder` with `DeflateDecoder` fallback for raw deflate streams. Passthrough for uncompressed, error for unsupported filters. |
| `content.rs` | ~200 | Parse content streams into `ContentOp` (operand stack + operator recognition). Replaces `lopdf::content::Content::decode`. |

### Parsing Flow

```
File bytes -> find startxref -> parse xref -> build object table -> RawDocument
                                                                        |
                                                    trailer -> Catalog -> Pages tree
                                                                        |
                                              Page -> content stream -> decompress -> parse ops
```

## font.rs Extraction

Code moved from `backend.rs` to `src/parser/font.rs`:

| Source (backend.rs lines) | Content |
|---|---|
| L88-270 | `ToUnicodeMap`, `parse_to_unicode_cmap()` |
| L336-563 | `find_font_dict()`, CMap cache lookup, embedded CMap parsing |
| L565-796 | TrueType cmap parsing (format 4, format 12) |
| L798-824 | `is_likely_binary()` heuristic |
| L62-86 | `decode_text_simple()` fallback |

```rust
pub struct FontResolver {
    cmap_cache: RefCell<HashMap<(u32, u16), Option<ToUnicodeMap>>>,
}

impl FontResolver {
    pub fn new() -> Self;
    pub fn decode_text(&self, doc: &RawDocument, page_id: PageId, font_name: &[u8], bytes: &[u8]) -> String;
    pub fn page_fonts(&self, doc: &RawDocument, page_id: PageId) -> Result<Vec<BackendFontInfo>>;
}
```

## Execution Phases

### Phase 1: PdfBackend Abstraction

- Add `metadata()`, `page_dimensions()`, `outline()`, `page_xobjects()` to `PdfBackend` trait
- Implement these on `LopdfBackend` (move logic from `pdf_parser.rs`)
- **Change `PdfParser.backend` field** from `LopdfBackend` (concrete) to `Box<dyn PdfBackend>`
- Add factory functions/constructor that accept `Box<dyn PdfBackend>` (keep `LopdfBackend` constructors as convenience)
- Refactor `pdf_parser.rs` to use trait methods only — eliminate all `raw_doc()` calls (~18 sites)
- Remove all `lopdf::` type references from `pdf_parser.rs` function signatures (`lopdf::ObjectId` in `extract_outline_items`, `lopdf::Dictionary` in `get_outline_destination`, `lopdf::Object` in `resolve_destination`, `get_string_from_dict`)
- Replace `PdfParser::is_encrypted()` and `PdfParser::version()` to use `self.backend.metadata()`
- Update password warning log message to remove lopdf-specific text
- `pdf_parser.rs` remains responsible for `parse_pdf_date()` (converting raw date strings from `PdfMetadataRaw` to `chrono::DateTime<Utc>`)
- All existing tests must pass

### Phase 2: Custom Parser (Bottom-Up)

- Implement `src/parser/raw/` modules: tokenizer -> xref -> document -> stream -> content
- Unit tests for each module
- No integration with `PdfBackend` yet

### Phase 3: RawBackend + font.rs

- Extract font code from `backend.rs` to `font.rs`
- Implement `RawBackend` using `RawDocument` + `FontResolver`
- Comparison tests: `LopdfBackend` vs `RawBackend` on all test PDFs (both backends coexist temporarily in this phase only)
- Wire `PdfParser` to use `RawBackend`

### Phase 4: lopdf Removal

- Delete `LopdfBackend`
- Delete `From<lopdf::Error>` in `error.rs`
- Remove `lopdf` from `Cargo.toml`, add `flate2`
- Final verification: all tests, clippy, release build, FFI build, **FFI integration test**

## Error Handling

Existing `Error` enum is reused without modification. Mapping:

| Parser situation | Error variant |
|---|---|
| Invalid xref | `Error::Corrupted(...)` |
| Object not found | `Error::MissingObject(...)` |
| Decompression failure | `Error::PdfParse(...)` |
| Encryption detected | `Error::Encrypted` |
| Unsupported filter | `Error::PdfParse(...)` |

Upper layers (pdf_parser, layout, render) require **no changes**.

## Verification Strategy

| Phase | Verification |
|---|---|
| Phase 1 | `cargo test` all pass, `cargo clippy` clean |
| Phase 2 | Unit tests per module (tokenizer, xref, document, stream, content) |
| Phase 3 | Comparison: LopdfBackend vs RawBackend output on all `test-files/` categories |
| Phase 4 | `cargo test`, `cargo build --release`, `cargo build --release --features ffi`, CLI on real PDFs |

## Dependencies

| Change | Crate | Purpose |
|---|---|---|
| Add | `flate2` | FlateDecode decompression |
| Remove | `lopdf` | PDF parsing (replaced by custom parser) |

Net result: dependency count decreases.

## Estimated Scale

| Component | New LOC |
|---|---|
| tokenizer.rs | ~400 |
| xref.rs | ~450 |
| document.rs | ~200 |
| stream.rs | ~50 |
| content.rs | ~200 |
| font.rs (moved) | ~500 (existing code, restructured) |
| RawBackend integration | ~100 |
| **Total new code** | **~1,400** |
