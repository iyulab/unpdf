# Replace lopdf with Custom PDF Parser — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the `lopdf` dependency and replace it with a purpose-built PDF parser, reducing external dependencies while gaining full control over PDF parsing.

**Architecture:** Phase 1 extends the `PdfBackend` trait and abstracts away all `lopdf` types from `pdf_parser.rs`. Phase 2 builds the custom parser bottom-up (tokenizer → xref → document → stream → content). Phase 3 extracts font code to `font.rs`, implements `RawBackend`, and replaces `LopdfBackend`. Phase 4 removes `lopdf` entirely.

**Tech Stack:** Rust, flate2 (FlateDecode decompression)

**Spec:** `docs/superpowers/specs/2026-03-19-replace-lopdf-design.md`

---

## File Map

### New Files
| File | Responsibility |
|---|---|
| `src/parser/raw/mod.rs` | Public API: `RawDocument` struct and `load`/`load_mem` constructors |
| `src/parser/raw/tokenizer.rs` | PDF byte stream → `PdfObject` values (all 10 types) |
| `src/parser/raw/xref.rs` | xref table/stream/ObjStm parsing, incremental updates |
| `src/parser/raw/document.rs` | Object resolution, reference tracking, page tree traversal |
| `src/parser/raw/stream.rs` | FlateDecode decompression (flate2), raw deflate fallback |
| `src/parser/raw/content.rs` | Content stream → `Vec<ContentOp>` |
| `src/parser/font.rs` | ToUnicode CMap, TrueType cmap, text decoding (from backend.rs) |

### Modified Files
| File | Changes |
|---|---|
| `src/parser/backend.rs` | Extend `PdfBackend` trait with 4 new methods, add supporting types, implement `RawBackend` |
| `src/parser/pdf_parser.rs` | Change `backend` field to `Box<dyn PdfBackend>`, remove all `lopdf::` type references |
| `src/parser/mod.rs` | Add `pub mod raw;` and `pub mod font;` |
| `src/error.rs` | Remove `From<lopdf::Error>` impl |
| `Cargo.toml` | Remove `lopdf`, add `flate2` |

---

## Phase 1: PdfBackend Abstraction

### Task 1: Add supporting types and extend PdfBackend trait

**Files:**
- Modify: `src/parser/backend.rs:39-60` (trait definition)

- [ ] **Step 1: Add supporting types before the trait definition**

In `src/parser/backend.rs`, add these types after `ContentOp` (after line 38) and before the `PdfBackend` trait (line 40):

```rust
/// Raw metadata from the PDF backend.
#[derive(Debug, Clone, Default)]
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

/// A raw outline (bookmark) item from the PDF.
#[derive(Debug, Clone)]
pub struct RawOutlineItem {
    pub title: String,
    pub page: Option<u32>,
    pub level: u8,
    pub children: Vec<RawOutlineItem>,
}

/// A raw XObject (image) extracted from a PDF page.
#[derive(Debug, Clone)]
pub struct RawXObject {
    pub name: String,
    pub subtype: String,
    pub data: Vec<u8>,
    pub filter: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub bits_per_component: Option<u8>,
    pub color_space: Option<String>,
}
```

- [ ] **Step 2: Extend the PdfBackend trait**

Add 4 new methods to the `PdfBackend` trait:

```rust
pub trait PdfBackend {
    // ... existing 5 methods ...

    /// Return raw metadata (version, info dict fields, encryption status).
    fn metadata(&self) -> PdfMetadataRaw;

    /// Return page dimensions (width, height) in points.
    /// Falls back to Letter size (612, 792) if MediaBox is absent.
    fn page_dimensions(&self, page: PageId) -> (f32, f32);

    /// Return the document outline (bookmarks) as a tree.
    /// Implementations must handle cycle detection and depth limits.
    fn outline(&self) -> Result<Vec<RawOutlineItem>>;

    /// Return XObjects (images) from a page.
    fn page_xobjects(&self, page: PageId) -> Result<Vec<RawXObject>>;
}
```

- [ ] **Step 3: Verify compilation fails (LopdfBackend doesn't implement new methods yet)**

Run: `cargo check 2>&1 | head -20`
Expected: compilation error about missing trait methods on `LopdfBackend`

---

### Task 2: Implement new trait methods on LopdfBackend

**Files:**
- Modify: `src/parser/backend.rs:826-978` (PdfBackend impl block)

- [ ] **Step 1: Implement `metadata()` on LopdfBackend**

Add inside the `impl PdfBackend for LopdfBackend` block (after `decode_text`, around line 977):

```rust
    fn metadata(&self) -> PdfMetadataRaw {
        let mut meta = PdfMetadataRaw {
            version: self.doc.version.to_string(),
            encrypted: self.doc.is_encrypted(),
            ..Default::default()
        };

        // Extract Info dictionary
        if let Ok(info) = self.doc.trailer.get(b"Info") {
            if let Ok(info_ref) = info.as_reference() {
                if let Ok(info_dict) = self.doc.get_dictionary(info_ref) {
                    meta.title = Self::get_string_from_lopdf_dict(info_dict, b"Title");
                    meta.author = Self::get_string_from_lopdf_dict(info_dict, b"Author");
                    meta.subject = Self::get_string_from_lopdf_dict(info_dict, b"Subject");
                    meta.keywords = Self::get_string_from_lopdf_dict(info_dict, b"Keywords");
                    meta.creator = Self::get_string_from_lopdf_dict(info_dict, b"Creator");
                    meta.producer = Self::get_string_from_lopdf_dict(info_dict, b"Producer");
                    meta.creation_date = Self::get_string_from_lopdf_dict(info_dict, b"CreationDate");
                    meta.mod_date = Self::get_string_from_lopdf_dict(info_dict, b"ModDate");
                }
            }
        }

        meta
    }
```

Add the `get_string_from_lopdf_dict` helper as an associated function on `LopdfBackend` (in the `impl LopdfBackend` block after line 980):

```rust
    /// Extract a string value from a lopdf dictionary.
    fn get_string_from_lopdf_dict(dict: &lopdf::Dictionary, key: &[u8]) -> Option<String> {
        dict.get(key).ok().and_then(|obj| match obj {
            Object::String(bytes, _) => {
                if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
                    let utf16: Vec<u16> = bytes[2..]
                        .chunks(2)
                        .filter_map(|c| {
                            if c.len() == 2 {
                                Some(u16::from_be_bytes([c[0], c[1]]))
                            } else {
                                None
                            }
                        })
                        .collect();
                    String::from_utf16(&utf16).ok()
                } else {
                    String::from_utf8(bytes.clone())
                        .ok()
                        .or_else(|| Some(bytes.iter().map(|&b| b as char).collect()))
                }
            }
            Object::Name(bytes) => String::from_utf8(bytes.clone()).ok(),
            _ => None,
        })
    }
```

- [ ] **Step 2: Implement `page_dimensions()` on LopdfBackend**

```rust
    fn page_dimensions(&self, page: PageId) -> (f32, f32) {
        if let Ok(page_dict) = self.doc.get_dictionary(page) {
            if let Ok(media_box) = page_dict.get(b"MediaBox") {
                if let Ok(array) = media_box.as_array() {
                    if array.len() >= 4 {
                        let width = array[2].as_float().unwrap_or(612.0);
                        let height = array[3].as_float().unwrap_or(792.0);
                        return (width, height);
                    }
                }
            }
        }
        (612.0, 792.0)
    }
```

- [ ] **Step 3: Implement `outline()` on LopdfBackend**

This moves the outline logic from `pdf_parser.rs` into the backend. Add a constant and the implementation:

```rust
    fn outline(&self) -> Result<Vec<RawOutlineItem>> {
        const MAX_DEPTH: u8 = 64;
        let mut items = Vec::new();
        let mut visited = std::collections::HashSet::new();

        if let Ok(catalog) = self.doc.catalog() {
            if let Ok(outlines) = catalog.get(b"Outlines") {
                if let Ok(outlines_ref) = outlines.as_reference() {
                    if let Ok(outlines_dict) = self.doc.get_dictionary(outlines_ref) {
                        if let Ok(first) = outlines_dict.get(b"First") {
                            if let Ok(first_ref) = first.as_reference() {
                                self.collect_outline_items(first_ref, 0, MAX_DEPTH, &mut items, &mut visited);
                            }
                        }
                    }
                }
            }
        }

        Ok(items)
    }
```

Add the recursive helper on `impl LopdfBackend`:

```rust
    fn collect_outline_items(
        &self,
        item_ref: ObjectId,
        level: u8,
        max_depth: u8,
        items: &mut Vec<RawOutlineItem>,
        visited: &mut std::collections::HashSet<ObjectId>,
    ) {
        if !visited.insert(item_ref) || level > max_depth {
            return;
        }

        if let Ok(item_dict) = self.doc.get_dictionary(item_ref) {
            let title = Self::get_string_from_lopdf_dict(item_dict, b"Title").unwrap_or_default();

            // Resolve destination page
            let page = self.resolve_outline_dest(item_dict);

            let mut outline_item = RawOutlineItem {
                title,
                page,
                level,
                children: Vec::new(),
            };

            // Process children
            if let Ok(first) = item_dict.get(b"First") {
                if let Ok(first_ref) = first.as_reference() {
                    self.collect_outline_items(first_ref, level + 1, max_depth, &mut outline_item.children, visited);
                }
            }

            items.push(outline_item);

            // Process siblings
            if let Ok(next) = item_dict.get(b"Next") {
                if let Ok(next_ref) = next.as_reference() {
                    self.collect_outline_items(next_ref, level, max_depth, items, visited);
                }
            }
        }
    }

    fn resolve_outline_dest(&self, item_dict: &lopdf::Dictionary) -> Option<u32> {
        let pages = self.doc.get_pages();

        // Try Dest
        if let Ok(dest) = item_dict.get(b"Dest") {
            if let Ok(dest_array) = dest.as_array() {
                if let Some(first) = dest_array.first() {
                    if let Ok(page_ref) = first.as_reference() {
                        for (num, id) in pages.iter() {
                            if *id == page_ref {
                                return Some(*num);
                            }
                        }
                    }
                }
            }
        }

        // Try A (action) dictionary
        if let Ok(action) = item_dict.get(b"A") {
            if let Ok(action_ref) = action.as_reference() {
                if let Ok(action_dict) = self.doc.get_dictionary(action_ref) {
                    if let Ok(dest) = action_dict.get(b"D") {
                        if let Ok(dest_array) = dest.as_array() {
                            if let Some(first) = dest_array.first() {
                                if let Ok(page_ref) = first.as_reference() {
                                    for (num, id) in pages.iter() {
                                        if *id == page_ref {
                                            return Some(*num);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        None
    }
```

- [ ] **Step 4: Implement `page_xobjects()` on LopdfBackend**

```rust
    fn page_xobjects(&self, page: PageId) -> Result<Vec<RawXObject>> {
        let mut xobjects = Vec::new();

        let page_dict = self.doc.get_dictionary(page)
            .map_err(|e| Error::PdfParse(e.to_string()))?;

        let res = match page_dict.get(b"Resources") {
            Ok(r) => r,
            Err(_) => return Ok(xobjects),
        };

        let res_dict = match res {
            Object::Reference(r) => match self.doc.get_dictionary(*r) {
                Ok(d) => d,
                Err(_) => return Ok(xobjects),
            },
            Object::Dictionary(d) => d,
            _ => return Ok(xobjects),
        };

        let xobj_entry = match res_dict.get(b"XObject") {
            Ok(x) => x,
            Err(_) => return Ok(xobjects),
        };

        let xobj_dict = match xobj_entry {
            Object::Reference(r) => match self.doc.get_dictionary(*r) {
                Ok(d) => d,
                Err(_) => return Ok(xobjects),
            },
            Object::Dictionary(d) => d,
            _ => return Ok(xobjects),
        };

        for (name, obj) in xobj_dict.iter() {
            if let Ok(obj_ref) = obj.as_reference() {
                if let Ok(Object::Stream(stream)) = self.doc.get_object(obj_ref) {
                    let dict = &stream.dict;

                    let subtype = dict.get(b"Subtype")
                        .ok()
                        .and_then(|s| s.as_name().ok())
                        .map(|n| String::from_utf8_lossy(n).to_string())
                        .unwrap_or_default();

                    if subtype != "Image" {
                        continue;
                    }

                    let filter = dict.get(b"Filter")
                        .ok()
                        .and_then(|f| f.as_name().ok())
                        .map(|n| String::from_utf8_lossy(n).to_string());

                    let data = match filter.as_deref() {
                        Some("DCTDecode") | Some("JPXDecode") => stream.content.clone(),
                        _ => safe_decompress(stream),
                    };

                    let width = dict.get(b"Width").ok().and_then(|w| w.as_i64().ok()).map(|w| w as u32);
                    let height = dict.get(b"Height").ok().and_then(|h| h.as_i64().ok()).map(|h| h as u32);
                    let bits = dict.get(b"BitsPerComponent").ok().and_then(|b| b.as_i64().ok()).map(|b| b as u8);

                    let color_space = dict.get(b"ColorSpace").ok().and_then(|cs| match cs {
                        Object::Name(n) => Some(String::from_utf8_lossy(n).to_string()),
                        Object::Array(arr) => arr.first()
                            .and_then(|o| o.as_name().ok())
                            .map(|n| String::from_utf8_lossy(n).to_string()),
                        _ => None,
                    });

                    xobjects.push(RawXObject {
                        name: String::from_utf8_lossy(name).to_string(),
                        subtype,
                        data,
                        filter,
                        width,
                        height,
                        bits_per_component: bits,
                        color_space,
                    });
                }
            }
        }

        Ok(xobjects)
    }
```

- [ ] **Step 5: Verify compilation succeeds**

Run: `cargo check`
Expected: compiles successfully

- [ ] **Step 6: Run tests**

Run: `cargo test`
Expected: all existing tests pass

- [ ] **Step 7: Commit**

```bash
git add src/parser/backend.rs
git commit -m "feat: extend PdfBackend trait with metadata, dimensions, outline, xobjects"
```

---

### Task 3: Refactor PdfParser to use Box<dyn PdfBackend>

**Files:**
- Modify: `src/parser/pdf_parser.rs` (entire file — remove all `lopdf::` references)
- Modify: `src/parser/backend.rs` (add re-exports for new types)

- [ ] **Step 1: Change PdfParser.backend field and constructors**

In `src/parser/pdf_parser.rs`, change:

```rust
use super::backend::LopdfBackend;
```
to:
```rust
use super::backend::{LopdfBackend, PdfBackend, PdfMetadataRaw, RawOutlineItem, RawXObject};
```

Change the struct:
```rust
pub struct PdfParser {
    backend: LopdfBackend,
    options: ParseOptions,
}
```
to:
```rust
pub struct PdfParser {
    backend: Box<dyn PdfBackend>,
    options: ParseOptions,
}
```

Update all constructors to wrap in `Box::new()`:
```rust
let backend = LopdfBackend::load_file(path)?;
// becomes:
let backend: Box<dyn PdfBackend> = Box::new(LopdfBackend::load_file(path)?);
```

Update password warning messages from `"lopdf doesn't support decryption"` to `"PDF decryption is not supported"`.

Replace `backend.is_encrypted()` calls in constructors with `backend.metadata().encrypted`. Note: the `is_encrypted()` check must happen **after** `Box::new()` wrapping, since it now goes through the trait:
```rust
let backend: Box<dyn PdfBackend> = Box::new(LopdfBackend::load_file(path)?);
if options.password.is_some() && backend.metadata().encrypted {
    log::warn!("Password was provided but PDF decryption is not supported");
}
```

- [ ] **Step 2: Replace raw_doc() calls in parse() and metadata extraction**

Replace `self.backend.raw_doc().get_pages()` with `self.backend.pages()`.

Replace `extract_metadata()` to use `self.backend.metadata()`:
```rust
fn extract_metadata(&self) -> Result<Metadata> {
    let raw = self.backend.metadata();
    let mut metadata = Metadata::with_version(raw.version);
    metadata.title = raw.title;
    metadata.author = raw.author;
    metadata.subject = raw.subject;
    metadata.keywords = raw.keywords;
    metadata.creator = raw.creator;
    metadata.producer = raw.producer;
    metadata.encrypted = raw.encrypted;

    if let Some(date_str) = raw.creation_date {
        metadata.created = parse_pdf_date(&date_str);
    }
    if let Some(date_str) = raw.mod_date {
        metadata.modified = parse_pdf_date(&date_str);
    }

    Ok(metadata)
}
```

- [ ] **Step 3: Replace get_page_dimensions() to use trait method**

```rust
fn get_page_dimensions(&self, page_num: u32) -> Result<(f32, f32)> {
    let pages = self.backend.pages();
    let page_id = pages
        .get(&page_num)
        .ok_or(Error::PageOutOfRange(page_num, pages.len() as u32))?;
    Ok(self.backend.page_dimensions(*page_id))
}
```

- [ ] **Step 4: Replace extract_outline() to use trait method**

```rust
fn extract_outline(&self) -> Result<Outline> {
    let raw_items = self.backend.outline()?;
    let mut outline = Outline::new();
    outline.items = raw_items.into_iter().map(|r| Self::convert_outline_item(r)).collect();
    Ok(outline)
}

fn convert_outline_item(raw: RawOutlineItem) -> OutlineItem {
    let mut item = OutlineItem::new(raw.title, raw.page, raw.level);
    item.children = raw.children.into_iter().map(Self::convert_outline_item).collect();
    item
}
```

Remove the old `extract_outline_items()`, `get_outline_destination()`, and `resolve_destination()` methods.

- [ ] **Step 5: Replace extract_resources() and extract_page_resources() to use trait method**

```rust
fn extract_resources(&self) -> Result<HashMap<String, Resource>> {
    let mut resources = HashMap::new();
    for (page_num, page_id) in self.backend.pages() {
        if let Ok(xobjects) = self.backend.page_xobjects(page_id) {
            for xobj in xobjects {
                let key = format!("page{}_{}", page_num, xobj.name);
                if let Some(resource) = Self::convert_xobject(xobj) {
                    resources.insert(key, resource);
                }
            }
        }
    }
    Ok(resources)
}

fn convert_xobject(xobj: RawXObject) -> Option<Resource> {
    let mime_type = match xobj.filter.as_deref() {
        Some("DCTDecode") => "image/jpeg",
        Some("JPXDecode") => "image/jp2",
        _ => "application/octet-stream",
    };

    let mut resource = Resource::new(xobj.data, mime_type.to_string(), ResourceType::Image);

    if let (Some(w), Some(h)) = (xobj.width, xobj.height) {
        resource = resource.with_dimensions(w, h);
    }
    if let Some(b) = xobj.bits_per_component {
        resource = resource.with_bits_per_component(b);
    }
    if let Some(cs) = xobj.color_space {
        resource = resource.with_color_space(cs);
    }

    Some(resource)
}
```

Remove old `extract_page_resources()` and `extract_xobject()` methods.

- [ ] **Step 6: Replace page_count(), is_encrypted(), version()**

```rust
pub fn page_count(&self) -> u32 {
    self.backend.pages().len() as u32
}

pub fn is_encrypted(&self) -> bool {
    self.backend.metadata().encrypted
}

pub fn version(&self) -> String {
    self.backend.metadata().version
}
```

- [ ] **Step 7: Remove the standalone `get_string_from_dict` helper function**

This function at the bottom of `pdf_parser.rs` uses `lopdf::Dictionary` and `lopdf::Object`. It's now replaced by `LopdfBackend::get_string_from_lopdf_dict()`. Delete it entirely.

- [ ] **Step 8: Verify no `lopdf::` references remain in pdf_parser.rs**

Run: `grep -n "lopdf" src/parser/pdf_parser.rs`
Expected: only `LopdfBackend` import for constructors is acceptable (this import will be replaced by `RawBackend` in Task 13). Clippy may warn about unused imports for `PdfMetadataRaw`/`RawOutlineItem`/`RawXObject` if they end up used only in conversion helpers — suppress with `#[allow(unused_imports)]` temporarily if needed.

- [ ] **Step 9: Verify compilation and tests**

Run: `cargo check && cargo test`
Expected: compiles and all tests pass

- [ ] **Step 10: Run clippy**

Run: `cargo clippy`
Expected: no errors. The `LopdfBackend` import is still used in constructors so no unused-import warning. If any other warnings appear, fix them.

- [ ] **Step 11: Commit**

```bash
git add src/parser/backend.rs src/parser/pdf_parser.rs
git commit -m "refactor: make PdfParser use Box<dyn PdfBackend>, remove all lopdf types from pdf_parser"
```

---

## Phase 2: Custom Parser (Bottom-Up)

### Task 4: Create parser module structure and core types

**Files:**
- Create: `src/parser/raw/mod.rs`
- Create: `src/parser/raw/tokenizer.rs`
- Modify: `src/parser/mod.rs`

- [ ] **Step 1: Create directory and mod.rs**

`src/parser/raw/mod.rs`:
```rust
//! Custom PDF parser — lightweight, purpose-built for text extraction.

pub mod tokenizer;
pub mod xref;
pub mod document;
pub mod stream;
pub mod content;

pub use document::RawDocument;
pub use tokenizer::{PdfObject, PdfDict, PdfStream};
```

- [ ] **Step 2: Add `pub mod raw;` to `src/parser/mod.rs`**

- [ ] **Step 3: Define core types in tokenizer.rs**

`src/parser/raw/tokenizer.rs`:
```rust
//! PDF object tokenizer/parser.

use std::collections::HashMap;
use crate::error::{Error, Result};

/// A PDF object.
#[derive(Debug, Clone)]
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

/// A PDF dictionary.
pub type PdfDict = HashMap<Vec<u8>, PdfObject>;

/// A PDF stream object.
#[derive(Debug, Clone)]
pub struct PdfStream {
    pub dict: PdfDict,
    pub raw_data: Vec<u8>,
}
```

Add helper methods on `PdfObject` for type access:
```rust
impl PdfObject {
    pub fn as_i64(&self) -> Option<i64> { ... }
    pub fn as_f64(&self) -> Option<f64> { ... }
    pub fn as_f32(&self) -> Option<f32> { ... }
    pub fn as_name(&self) -> Option<&[u8]> { ... }
    pub fn as_str_bytes(&self) -> Option<&[u8]> { ... }
    pub fn as_array(&self) -> Option<&[PdfObject]> { ... }
    pub fn as_dict(&self) -> Option<&PdfDict> { ... }
    pub fn as_stream(&self) -> Option<&PdfStream> { ... }
    pub fn as_reference(&self) -> Option<(u32, u16)> { ... }
}
```

Add dict helper:
```rust
/// Get a value from a PdfDict by key.
pub fn dict_get<'a>(dict: &'a PdfDict, key: &[u8]) -> Option<&'a PdfObject> {
    dict.get(key)
}
```

- [ ] **Step 4: Create stub files for remaining modules**

Create empty stubs for `xref.rs`, `document.rs`, `stream.rs`, `content.rs` so the module compiles:
```rust
//! [module description]
```

- [ ] **Step 5: Verify compilation**

Run: `cargo check`
Expected: compiles (stubs are empty, unused warnings OK)

- [ ] **Step 6: Commit**

```bash
git add src/parser/raw/ src/parser/mod.rs
git commit -m "feat: add parser/raw module structure with core PDF object types"
```

---

### Task 5: Implement PDF tokenizer

**Files:**
- Modify: `src/parser/raw/tokenizer.rs`

- [ ] **Step 1: Write tokenizer tests**

Add to the bottom of `tokenizer.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_integer() {
        let (obj, _) = parse_object(b"42 ", 0).unwrap();
        assert_eq!(obj.as_i64(), Some(42));
    }

    #[test]
    fn test_parse_negative_integer() {
        let (obj, _) = parse_object(b"-17 ", 0).unwrap();
        assert_eq!(obj.as_i64(), Some(-17));
    }

    #[test]
    fn test_parse_real() {
        let (obj, _) = parse_object(b"3.14 ", 0).unwrap();
        assert!((obj.as_f64().unwrap() - 3.14).abs() < 0.001);
    }

    #[test]
    fn test_parse_bool() {
        let (obj, _) = parse_object(b"true ", 0).unwrap();
        assert_eq!(obj, PdfObject::Bool(true));
    }

    #[test]
    fn test_parse_null() {
        let (obj, _) = parse_object(b"null ", 0).unwrap();
        assert!(matches!(obj, PdfObject::Null));
    }

    #[test]
    fn test_parse_name() {
        let (obj, _) = parse_object(b"/Type ", 0).unwrap();
        assert_eq!(obj.as_name(), Some(b"Type".as_slice()));
    }

    #[test]
    fn test_parse_name_with_hex_escape() {
        let (obj, _) = parse_object(b"/A#20B ", 0).unwrap();
        assert_eq!(obj.as_name(), Some(b"A B".as_slice()));
    }

    #[test]
    fn test_parse_literal_string() {
        let (obj, _) = parse_object(b"(Hello World) ", 0).unwrap();
        assert_eq!(obj.as_str_bytes(), Some(b"Hello World".as_slice()));
    }

    #[test]
    fn test_parse_literal_string_escaped() {
        let (obj, _) = parse_object(b"(Hello\\nWorld) ", 0).unwrap();
        assert_eq!(obj.as_str_bytes(), Some(b"Hello\nWorld".as_slice()));
    }

    #[test]
    fn test_parse_literal_string_nested_parens() {
        let (obj, _) = parse_object(b"(Hello (World)) ", 0).unwrap();
        assert_eq!(obj.as_str_bytes(), Some(b"Hello (World)".as_slice()));
    }

    #[test]
    fn test_parse_hex_string() {
        let (obj, _) = parse_object(b"<48656C6C6F> ", 0).unwrap();
        assert_eq!(obj.as_str_bytes(), Some(b"Hello".as_slice()));
    }

    #[test]
    fn test_parse_hex_string_odd_length() {
        let (obj, _) = parse_object(b"<ABC> ", 0).unwrap();
        assert_eq!(obj.as_str_bytes(), Some(&[0xAB, 0xC0][..]));
    }

    #[test]
    fn test_parse_array() {
        let (obj, _) = parse_object(b"[1 2 3] ", 0).unwrap();
        let arr = obj.as_array().unwrap();
        assert_eq!(arr.len(), 3);
    }

    #[test]
    fn test_parse_dict() {
        let (obj, _) = parse_object(b"<< /Type /Page /Count 5 >> ", 0).unwrap();
        let dict = obj.as_dict().unwrap();
        assert_eq!(dict_get(dict, b"Type").unwrap().as_name(), Some(b"Page".as_slice()));
        assert_eq!(dict_get(dict, b"Count").unwrap().as_i64(), Some(5));
    }

    #[test]
    fn test_parse_reference() {
        let (obj, _) = parse_object(b"10 0 R ", 0).unwrap();
        assert_eq!(obj.as_reference(), Some((10, 0)));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib parser::raw::tokenizer`
Expected: FAIL (functions not implemented)

- [ ] **Step 3: Implement the tokenizer**

Implement in `tokenizer.rs`:

```rust
/// Parse a PDF object starting at position `pos`.
/// Returns the parsed object and the position after it.
pub fn parse_object(data: &[u8], pos: usize) -> Result<(PdfObject, usize)> { ... }

/// Skip whitespace and comments.
fn skip_whitespace(data: &[u8], pos: usize) -> usize { ... }

/// Check if byte is a PDF whitespace character.
fn is_whitespace(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x00 | 0x0C)
}

/// Check if byte is a PDF delimiter.
fn is_delimiter(b: u8) -> bool {
    matches!(b, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%')
}

/// Parse a number (integer or real).
fn parse_number(data: &[u8], pos: usize) -> Result<(PdfObject, usize)> { ... }

/// Parse a name object (/Name).
fn parse_name(data: &[u8], pos: usize) -> Result<(PdfObject, usize)> { ... }

/// Parse a literal string ((text)).
fn parse_literal_string(data: &[u8], pos: usize) -> Result<(PdfObject, usize)> { ... }

/// Parse a hex string (<hex>) or dictionary (<<...>>).
fn parse_hex_or_dict(data: &[u8], pos: usize) -> Result<(PdfObject, usize)> { ... }

/// Parse a hex string <hex>.
fn parse_hex_string(data: &[u8], pos: usize) -> Result<(PdfObject, usize)> { ... }

/// Parse a dictionary <<...>>.
fn parse_dict(data: &[u8], pos: usize) -> Result<(PdfObject, usize)> { ... }

/// Parse an array [...].
fn parse_array(data: &[u8], pos: usize) -> Result<(PdfObject, usize)> { ... }
```

Key implementation details:
- `parse_object` dispatches based on first non-whitespace byte
- Numbers that look like `N G R` are references (lookahead for 'R')
- Numbers that look like `N G obj ... endobj` are indirect objects (lookahead for 'obj')
- Literal strings handle nested parentheses with depth counting
- Hex strings handle odd-length by padding with 0
- Names handle `#XX` hex escapes
- Dictionaries followed by `stream` keyword capture stream data up to `endstream`

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib parser::raw::tokenizer`
Expected: all PASS

- [ ] **Step 5: Commit**

```bash
git add src/parser/raw/tokenizer.rs
git commit -m "feat: implement PDF object tokenizer with full type support"
```

---

### Task 6: Implement stream decompression

**Files:**
- Modify: `src/parser/raw/stream.rs`
- Modify: `Cargo.toml`

- [ ] **Step 1: Add flate2 dependency**

In `Cargo.toml`, add under `[dependencies]`:
```toml
flate2 = "1.1"
```

- [ ] **Step 2: Write stream tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decompress_uncompressed() {
        let stream = PdfStream {
            dict: HashMap::new(),
            raw_data: b"Hello World".to_vec(),
        };
        let result = decompress(&stream).unwrap();
        assert_eq!(result, b"Hello World");
    }

    #[test]
    fn test_decompress_flate() {
        use flate2::write::ZlibEncoder;
        use flate2::Compression;
        use std::io::Write;

        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(b"Hello Compressed").unwrap();
        let compressed = encoder.finish().unwrap();

        let mut dict = HashMap::new();
        dict.insert(b"Filter".to_vec(), PdfObject::Name(b"FlateDecode".to_vec()));

        let stream = PdfStream { dict, raw_data: compressed };
        let result = decompress(&stream).unwrap();
        assert_eq!(result, b"Hello Compressed");
    }
}
```

- [ ] **Step 3: Implement stream decompression**

```rust
//! PDF stream decompression.

use std::collections::HashMap;
use std::io::Read;
use crate::error::{Error, Result};
use super::tokenizer::{PdfObject, PdfStream, PdfDict, dict_get};

/// Decompress a PDF stream based on its Filter entry.
pub fn decompress(stream: &PdfStream) -> Result<Vec<u8>> {
    let filter = dict_get(&stream.dict, b"Filter");

    match filter {
        None => Ok(stream.raw_data.clone()),
        Some(PdfObject::Name(name)) => decompress_single(name, &stream.raw_data),
        Some(PdfObject::Array(filters)) => {
            let mut data = stream.raw_data.clone();
            for f in filters {
                if let Some(name) = f.as_name() {
                    data = decompress_single(name, &data)?;
                }
            }
            Ok(data)
        }
        _ => Ok(stream.raw_data.clone()),
    }
}

fn decompress_single(filter_name: &[u8], data: &[u8]) -> Result<Vec<u8>> {
    match filter_name {
        b"FlateDecode" | b"Fl" => decompress_flate(data),
        b"ASCIIHexDecode" | b"AHx" => decode_ascii_hex(data),
        _ => Err(Error::PdfParse(format!(
            "unsupported filter: {}",
            String::from_utf8_lossy(filter_name)
        ))),
    }
}

fn decompress_flate(data: &[u8]) -> Result<Vec<u8>> {
    // Try zlib first (most common)
    let mut output = Vec::new();
    if let Ok(()) = flate2::read::ZlibDecoder::new(data).read_to_end(&mut output) {
        return Ok(output);
    }

    // Fallback: raw deflate (some PDF producers omit zlib header)
    output.clear();
    flate2::read::DeflateDecoder::new(data)
        .read_to_end(&mut output)
        .map_err(|e| Error::PdfParse(format!("decompression failed: {}", e)))?;
    Ok(output)
}

fn decode_ascii_hex(data: &[u8]) -> Result<Vec<u8>> {
    let hex: String = data.iter()
        .filter(|b| !b.is_ascii_whitespace())
        .take_while(|&&b| b != b'>')
        .map(|&b| b as char)
        .collect();
    let mut result = Vec::with_capacity(hex.len() / 2);
    let mut chars = hex.chars();
    while let Some(h) = chars.next() {
        let l = chars.next().unwrap_or('0');
        let byte = u8::from_str_radix(&format!("{}{}", h, l), 16)
            .map_err(|_| Error::PdfParse("invalid hex in ASCIIHexDecode".to_string()))?;
        result.push(byte);
    }
    Ok(result)
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib parser::raw::stream`
Expected: all PASS

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml src/parser/raw/stream.rs
git commit -m "feat: implement PDF stream decompression with FlateDecode and raw deflate fallback"
```

---

### Task 7: Implement xref parser

**Files:**
- Modify: `src/parser/raw/xref.rs`

- [ ] **Step 1: Write xref tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_startxref() {
        let data = b"%PDF-1.4\nsome content\nstartxref\n12345\n%%EOF";
        assert_eq!(find_startxref(data).unwrap(), 12345);
    }

    #[test]
    fn test_parse_traditional_xref() {
        let data = b"xref\n0 3\n0000000000 65535 f \n0000000015 00000 n \n0000000100 00000 n \ntrailer\n<< /Size 3 /Root 1 0 R >>\nstartxref\n0\n%%EOF";
        let (table, trailer) = parse_xref(data, 0).unwrap();
        assert_eq!(table.entries.len(), 2); // 2 'n' entries (not counting 'f')
        assert_eq!(table.entries[&(1, 0)], XrefEntry::Uncompressed(15));
        assert_eq!(table.entries[&(2, 0)], XrefEntry::Uncompressed(100));
    }
}
```

- [ ] **Step 2: Implement xref parser**

```rust
//! PDF cross-reference (xref) table parser.

use std::collections::HashMap;
use crate::error::{Error, Result};
use super::tokenizer::{self, PdfObject, PdfDict, dict_get};
use super::stream::decompress;

/// An entry in the xref table.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum XrefEntry {
    /// Object at byte offset in file.
    Uncompressed(usize),
    /// Object stored in ObjStm: (stream_obj_number, index_within_stream).
    Compressed(u32, u32),
}

/// Parsed xref table.
#[derive(Debug, Default)]
pub struct XrefTable {
    pub entries: HashMap<(u32, u16), XrefEntry>,
}

/// Find the startxref offset from end of file.
pub fn find_startxref(data: &[u8]) -> Result<usize> { ... }

/// Parse the complete xref chain (including incremental updates).
/// Returns the merged xref table and the final trailer dictionary.
pub fn parse_xref(data: &[u8], offset: usize) -> Result<(XrefTable, PdfDict)> { ... }

/// Parse a traditional xref table section.
fn parse_traditional_xref(data: &[u8], pos: usize) -> Result<(XrefTable, PdfDict, Option<usize>)> { ... }

/// Parse an xref stream (PDF 1.5+).
fn parse_xref_stream(data: &[u8], pos: usize) -> Result<(XrefTable, PdfDict, Option<usize>)> { ... }
```

Key implementation details:
- `find_startxref`: search backwards from EOF for `startxref` keyword, parse the number after it
- `parse_xref`: check if the data at offset starts with `xref` (traditional) or is an object (xref stream), then parse accordingly. Follow `/Prev` chain for incremental updates. Earlier entries don't override later ones.
- Traditional xref: parse "0 N" subsection headers, 20-byte entries (offset gen n/f)
- Xref stream: decompress stream, read binary entries using `/W [w1 w2 w3]` field widths
  - type=0: free, type=1: uncompressed(offset), type=2: compressed(stream_obj, index)
- `/Prev` in trailer points to previous xref section

- [ ] **Step 3: Run tests**

Run: `cargo test --lib parser::raw::xref`
Expected: all PASS

- [ ] **Step 4: Commit**

```bash
git add src/parser/raw/xref.rs
git commit -m "feat: implement xref parser (traditional + stream + incremental)"
```

---

### Task 8: Implement RawDocument

**Files:**
- Modify: `src/parser/raw/document.rs`

- [ ] **Step 1: Write document tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_simple_pdf() {
        // A minimal valid PDF
        let pdf = include_bytes!("../../../test-files/basic/trivial.pdf");
        let doc = RawDocument::load(pdf).unwrap();
        assert!(doc.page_count() > 0);
    }

    #[test]
    fn test_resolve_reference() {
        let pdf = include_bytes!("../../../test-files/basic/trivial.pdf");
        let doc = RawDocument::load(pdf).unwrap();
        let catalog = doc.catalog().unwrap();
        // Catalog should have a /Pages entry
        assert!(dict_get(catalog, b"Pages").is_some());
    }
}
```

- [ ] **Step 2: Implement RawDocument**

```rust
//! PDF document structure.

use std::collections::{BTreeMap, HashMap};
use crate::error::{Error, Result};
use super::tokenizer::{self, PdfObject, PdfDict, PdfStream, dict_get};
use super::xref::{self, XrefTable, XrefEntry};
use super::stream;

/// A parsed PDF document.
pub struct RawDocument {
    data: Vec<u8>,
    objects: HashMap<(u32, u16), PdfObject>,
    trailer: PdfDict,
    pub version: String,
}

impl RawDocument {
    /// Load a PDF from bytes.
    pub fn load(data: &[u8]) -> Result<Self> { ... }

    /// Resolve a PdfObject reference, returning the resolved object.
    pub fn resolve<'a>(&'a self, obj: &'a PdfObject) -> Result<&'a PdfObject> { ... }

    /// Get an object by ID.
    pub fn get_object(&self, id: (u32, u16)) -> Option<&PdfObject> { ... }

    /// Get the trailer dictionary.
    pub fn trailer(&self) -> &PdfDict { ... }

    /// Get the catalog dictionary.
    pub fn catalog(&self) -> Result<&PdfDict> { ... }

    /// Get all pages as (page_number -> (obj_num, gen_num)).
    pub fn pages(&self) -> BTreeMap<u32, (u32, u16)> { ... }

    /// Get the page count.
    pub fn page_count(&self) -> u32 { ... }

    /// Get a dictionary from an object, resolving references.
    pub fn get_dict(&self, id: (u32, u16)) -> Result<&PdfDict> { ... }
}
```

Key implementation details:
- `load`: parse version from header, `find_startxref`, `parse_xref`, load all objects from xref entries
- For `XrefEntry::Uncompressed(offset)`: parse object at that offset
- For `XrefEntry::Compressed(stream_obj, index)`: parse ObjStm, decompress, extract nth object
- `pages()`: traverse Catalog → Pages → recursive Kids enumeration, track page numbers
- Page tree traversal handles both direct children and nested Pages nodes

- [ ] **Step 3: Run tests**

Run: `cargo test --lib parser::raw::document`
Expected: all PASS

- [ ] **Step 4: Commit**

```bash
git add src/parser/raw/document.rs
git commit -m "feat: implement RawDocument with object resolution and page tree traversal"
```

---

### Task 9: Implement content stream parser

**Files:**
- Modify: `src/parser/raw/content.rs`

- [ ] **Step 1: Write content parser tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::backend::PdfValue;

    #[test]
    fn test_parse_simple_text_ops() {
        let data = b"BT /F1 12 Tf 100 700 Td (Hello World) Tj ET";
        let ops = parse_content_stream(data).unwrap();
        assert_eq!(ops[0].operator, "BT");
        assert_eq!(ops[1].operator, "Tf");
        assert_eq!(ops[2].operator, "Td");
        assert_eq!(ops[3].operator, "Tj");
        assert_eq!(ops[4].operator, "ET");
    }

    #[test]
    fn test_parse_tj_array() {
        let data = b"BT [(Hello) -100 (World)] TJ ET";
        let ops = parse_content_stream(data).unwrap();
        let tj = &ops[1];
        assert_eq!(tj.operator, "TJ");
    }
}
```

- [ ] **Step 2: Implement content parser**

```rust
//! PDF content stream parser.

use crate::error::{Error, Result};
use crate::parser::backend::{ContentOp, PdfValue};

/// Parse a content stream into a sequence of operations.
pub fn parse_content_stream(data: &[u8]) -> Result<Vec<ContentOp>> { ... }
```

Implementation: operand stack approach. Parse tokens, push non-operators onto stack, when an operator is found, pop the stack as operands.

- [ ] **Step 3: Run tests**

Run: `cargo test --lib parser::raw::content`
Expected: all PASS

- [ ] **Step 4: Commit**

```bash
git add src/parser/raw/content.rs
git commit -m "feat: implement PDF content stream parser"
```

---

### Task 10: Integration test — parse real PDFs with RawDocument

**Files:**
- Create: `tests/raw_parser_test.rs`

- [ ] **Step 1: Write integration tests**

```rust
use unpdf::parser::raw::RawDocument;

#[test]
fn test_parse_basic_pdf() {
    let data = std::fs::read("test-files/basic/trivial.pdf").unwrap();
    let doc = RawDocument::load(&data).unwrap();
    assert!(doc.page_count() > 0);
}

#[test]
fn test_parse_scientific_pdf() {
    let data = std::fs::read("test-files/scientific/arxiv-sample.pdf").unwrap();
    let doc = RawDocument::load(&data).unwrap();
    assert!(doc.page_count() > 0);
}

// Test each category in test-files/
```

- [ ] **Step 2: Run integration tests**

Run: `cargo test --test raw_parser_test`
Expected: all PASS

- [ ] **Step 3: Commit**

```bash
git add tests/raw_parser_test.rs
git commit -m "test: add integration tests for custom PDF parser on real documents"
```

---

## Phase 3: RawBackend + font.rs

### Task 11: Extract font code to font.rs

**Files:**
- Create: `src/parser/font.rs`
- Modify: `src/parser/backend.rs` (remove moved code, import from font.rs)
- Modify: `src/parser/mod.rs` (add `pub mod font;`)

- [ ] **Step 1: Create font.rs with types and functions moved from backend.rs**

Move to `src/parser/font.rs`:
- `ToUnicodeMap` struct and its `decode()` method (backend.rs L94-123)
- `parse_hex()`, `hex_to_unicode()`, `parse_to_unicode_cmap()` (backend.rs L126-270)
- `parse_truetype_cmap_table()`, `parse_cmap_format4()`, `parse_cmap_format12()` (backend.rs L570-796)
- `is_likely_binary()` (backend.rs L802-824)
- `decode_text_simple()` (backend.rs L63-86)

Make items that were `fn` or `struct` into `pub(crate)` as needed.

- [ ] **Step 2: Create FontResolver struct**

```rust
use std::cell::RefCell;
use std::collections::HashMap;
use super::backend::PageId;
use super::raw::RawDocument;

pub struct FontResolver {
    cmap_cache: RefCell<HashMap<(u32, u16), Option<ToUnicodeMap>>>,
}

impl FontResolver {
    pub fn new() -> Self {
        Self {
            cmap_cache: RefCell::new(HashMap::new()),
        }
    }

    // Methods: find_font_dict, get_to_unicode_map, parse_font_to_unicode,
    // get_embedded_cmap, is_identity_cid_font, decode_text, page_fonts
    // (adapted from LopdfBackend methods to use RawDocument instead of lopdf)
}
```

- [ ] **Step 3: Update backend.rs to import from font.rs**

Replace the moved code with imports:
```rust
use super::font::{ToUnicodeMap, parse_to_unicode_cmap, decode_text_simple, is_likely_binary, ...};
```

- [ ] **Step 4: Add `pub mod font;` to mod.rs**

- [ ] **Step 5: Verify all tests pass**

Run: `cargo test`
Expected: all PASS (behavior unchanged, code just moved)

- [ ] **Step 6: Commit**

```bash
git add src/parser/font.rs src/parser/backend.rs src/parser/mod.rs
git commit -m "refactor: extract font decoding code from backend.rs to font.rs"
```

---

### Task 12: Implement RawBackend

**Files:**
- Modify: `src/parser/backend.rs`

- [ ] **Step 1: Implement RawBackend struct**

```rust
use super::raw::RawDocument;
use super::font::FontResolver;

pub struct RawBackend {
    doc: RawDocument,
    font_resolver: FontResolver,
}

impl RawBackend {
    pub fn load_file<P: AsRef<std::path::Path>>(path: P) -> Result<Self> {
        let data = std::fs::read(path)?;
        Self::load_bytes(&data)
    }

    pub fn load_bytes(data: &[u8]) -> Result<Self> {
        let doc = RawDocument::load(data)?;
        Ok(Self {
            doc,
            font_resolver: FontResolver::new(),
        })
    }

    pub fn load_reader<R: std::io::Read>(mut reader: R) -> Result<Self> {
        let mut data = Vec::new();
        reader.read_to_end(&mut data)?;
        Self::load_bytes(&data)
    }
}
```

- [ ] **Step 2: Implement PdfBackend for RawBackend**

Implement all 9 trait methods using `RawDocument` and `FontResolver`.

Key implementation notes:
- `pages()`: delegate to `self.doc.pages()`
- `page_content()`: get page dict → Contents → resolve stream → `stream::decompress()`
- `decode_content()`: delegate to `content::parse_content_stream()`
- `decode_text()`: delegate to `self.font_resolver.decode_text()`
- `page_fonts()`: delegate to `self.font_resolver.page_fonts()`
- `metadata()`: build from `self.doc.trailer()` + `self.doc.version`
- `page_dimensions()`: get page dict → MediaBox → parse array
- `outline()`: traverse Catalog → Outlines → recursive First/Next chain with cycle detection
- `page_xobjects()`: get page Resources → XObject dict → for each Image XObject, use `stream::decompress()` for FlateDecode/uncompressed, pass through raw data for DCTDecode/JPXDecode

- [ ] **Step 3: Write comparison test**

```rust
#[cfg(test)]
mod raw_backend_tests {
    use super::*;

    #[test]
    fn test_raw_backend_pages_match_lopdf() {
        let data = std::fs::read("test-files/basic/trivial.pdf").unwrap();
        let lopdf = LopdfBackend::load_bytes(&data).unwrap();
        let raw = RawBackend::load_bytes(&data).unwrap();

        assert_eq!(lopdf.pages().len(), raw.pages().len());
    }

    #[test]
    fn test_raw_backend_text_extraction() {
        let data = std::fs::read("test-files/basic/trivial.pdf").unwrap();
        let raw = RawBackend::load_bytes(&data).unwrap();
        let pages = raw.pages();
        let first_page = *pages.values().next().unwrap();
        let content = raw.page_content(first_page).unwrap();
        assert!(!content.is_empty());
    }
}
```

- [ ] **Step 4: Run comparison tests**

Run: `cargo test raw_backend_tests`
Expected: all PASS

- [ ] **Step 5: Commit**

```bash
git add src/parser/backend.rs
git commit -m "feat: implement RawBackend using custom parser"
```

---

### Task 13: Switch PdfParser to use RawBackend

**Files:**
- Modify: `src/parser/pdf_parser.rs`

- [ ] **Step 1: Change constructors to use RawBackend**

Replace `LopdfBackend::load_file(path)` with `RawBackend::load_file(path)`, etc.

Update import:
```rust
use super::backend::{RawBackend, PdfBackend, PdfMetadataRaw, RawOutlineItem, RawXObject};
```

- [ ] **Step 2: Run full test suite**

Run: `cargo test`
Expected: all PASS with RawBackend

- [ ] **Step 3: Test with CLI on real PDFs**

Run: `cargo run -p unpdf-cli -- test-files/basic/trivial.pdf`
Run: `cargo run -p unpdf-cli -- test-files/scientific/arxiv-sample.pdf`
Expected: valid text output, no panics

- [ ] **Step 4: Commit**

```bash
git add src/parser/pdf_parser.rs
git commit -m "feat: switch PdfParser from LopdfBackend to RawBackend"
```

---

## Phase 4: lopdf Removal

### Task 14: Remove lopdf dependency

**Files:**
- Modify: `src/parser/backend.rs` (delete LopdfBackend, safe_decompress, convert_object)
- Modify: `src/error.rs` (delete From<lopdf::Error>)
- Modify: `Cargo.toml` (remove lopdf)

- [ ] **Step 1: Delete LopdfBackend and all lopdf-specific code from backend.rs**

Remove:
- `use lopdf::{...};` import
- `LopdfBackend` struct and all `impl` blocks
- `safe_decompress()` function
- `convert_object()` function

- [ ] **Step 2: Delete From<lopdf::Error> from error.rs**

Remove lines 81-89 (the entire `From<lopdf::Error>` impl block including closing brace):
```rust
impl From<lopdf::Error> for Error {
    ...
}
```

- [ ] **Step 3: Remove lopdf from Cargo.toml**

Delete `lopdf = "0.39"` from `[dependencies]`.

- [ ] **Step 4: Verify compilation**

Run: `cargo check`
Expected: compiles with no lopdf references

- [ ] **Step 5: Run full test suite**

Run: `cargo test`
Expected: all PASS

- [ ] **Step 6: Run clippy**

Run: `cargo clippy`
Expected: no warnings

- [ ] **Step 7: Build release + FFI**

Run: `cargo build --release && cargo build --release --features ffi`
Expected: both succeed

- [ ] **Step 8: FFI integration test**

Run: `cargo test --features ffi`
If `bindings/` contains runnable tests, execute them as well to verify FFI surface is intact.

- [ ] **Step 9: Commit**

```bash
git add Cargo.toml src/parser/backend.rs src/error.rs
git commit -m "feat: remove lopdf dependency — custom PDF parser is now the sole backend"
```

---

## Verification Checklist

- [ ] All existing tests pass
- [ ] `cargo clippy` clean
- [ ] `cargo build --release` succeeds
- [ ] `cargo build --release --features ffi` succeeds
- [ ] No `lopdf` references in any source file
- [ ] `grep -r "lopdf" src/` returns no results
- [ ] CLI works on basic, scientific, CJK, and complex PDFs from test-files/
