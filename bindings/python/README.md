# unpdf

Python bindings for [unpdf](https://github.com/iyulab/unpdf) - High-performance PDF content extraction to Markdown, text, and JSON.

## Installation

```bash
pip install unpdf-markdown
```

The distribution is `unpdf-markdown`; the import name is `unpdf`.

## Quick Start

```python
import unpdf

# Convert PDF to Markdown
markdown = unpdf.to_markdown("document.pdf")
print(markdown)

# Convert PDF to plain text
text = unpdf.to_text("document.pdf")
print(text)

# Convert PDF to JSON
json_data = unpdf.to_json("document.pdf", pretty=True)
print(json_data)

# Get document information
info = unpdf.get_info("document.pdf")
print(info)

# Get page count
pages = unpdf.get_page_count("document.pdf")
print(f"Total pages: {pages}")

# Check if file is a valid PDF
is_valid = unpdf.is_pdf("document.pdf")
print(f"Is valid PDF: {is_valid}")
```

## Input: path or bytes

Every function takes its PDF as `PdfSource` — a path (`str`, or any `os.PathLike`
such as `pathlib.Path`) or the PDF's own bytes. Types are unambiguous: `str` is
always a path, `bytes` is always content, and bytes go through the native
in-memory parser rather than a temporary file.

```python
from pathlib import Path
import unpdf

unpdf.to_markdown("document.pdf")
unpdf.to_markdown(Path("document.pdf"))
unpdf.to_markdown(pdf_bytes)
```

## API Reference

### `to_markdown(source: PdfSource, flags: int = 0) -> str`
Convert a PDF file to Markdown format.

### `to_text(source: PdfSource) -> str`
Convert a PDF file to plain text.

### `to_json(source: PdfSource, pretty: bool = False) -> str`
Convert a PDF file to JSON format.

### `get_info(source: PdfSource) -> dict`
Get document metadata. Keys: `section_count` (the page count), `resource_count`,
plus `title` / `author` only when the document sets them.

### `get_page_count(source: PdfSource) -> int`
Get the number of pages in a PDF file.

### `is_pdf(source: PdfSource) -> bool`
Check if a file is a valid PDF.

### `version() -> str`
Get the version of the native library.

### `get_extraction_quality(source: PdfSource) -> dict`
Document-level extraction diagnostics: `char_count`, `word_count`,
`replacement_char_count`, `encrypted`, `is_scan_pdf`, `suppressed_ocr_pages`,
`pages_incomplete`, `declared_page_count`, `unresolved_page_nodes`,
`skipped_object_count`. See "Incomplete extraction" below.

### `get_page_stats(source: PdfSource, page_number: int) -> dict`
Per-page content-stream operator counts (1-indexed): `page`, `text_op_count`,
`image_op_count`, `ocr_text_suppressed`.

## Detecting scanned (image-only) PDFs

Empty output can mean a scanned document, a genuinely blank page, or a parse
failure. The introspection surface tells them apart:

```python
import unpdf

if unpdf.get_extraction_quality("scan.pdf")["is_scan_pdf"]:
    print("Scanned document - OCR required")

stats = unpdf.get_page_stats("scan.pdf", 1)
if stats["text_op_count"] == 0 and stats["image_op_count"] > 0:
    print("Page 1 is image-only (scanned)")
elif stats["text_op_count"] == 0:
    print("Page 1 is genuinely blank")
```

Note: a *searchable* scan (page image plus an invisible OCR text layer) reports
`text_op_count > 0` — combine the check with `ocr_text_suppressed`, which flags
pages whose unreadable OCR layer was dropped.

## Incomplete extraction

A damaged PDF does not always fail. When the cross-reference table survives but the
objects it points at do not, the parser returns the pages it could read — a success
over an incomplete page set. Check before indexing or archiving, because a page that
silently never arrived is indistinguishable from a page that never existed:

```python
quality = unpdf.get_extraction_quality("document.pdf")
if quality["pages_incomplete"]:
    print(f"incomplete - document declares {quality['declared_page_count']} page(s)")
```

| Field | Meaning |
|-------|---------|
| `pages_incomplete` | Pages are known to be missing. The one field to branch on. |
| `declared_page_count` | Page count the document declares, or `None` if unreadable. |
| `unresolved_page_nodes` | Unreadable page-tree *nodes* — non-zero means incomplete, **not** a page count. |
| `skipped_object_count` | Objects that could not be loaded. Most cost no page. |

Also note that `get_info()["resource_count"]` counts the extracted-resource
inventory, which this binding's parse path leaves empty (resource extraction is off
by default to bound peak memory) — it is not a count of images on the page. Use
`get_page_stats` for scan detection.

## Handling failures

`UnpdfError` carries a `kind` so you can branch on the reason a call failed instead
of matching on message text:

```python
from unpdf import to_text, ErrorKind, UnpdfError

try:
    text = to_text("document.pdf")
except UnpdfError as e:
    if e.kind == ErrorKind.ENCRYPTED:
        print("Password required")
    elif e.kind in (ErrorKind.CORRUPTED, ErrorKind.PDF_PARSE):
        print("The file is damaged")
    else:
        print(f"Extraction failed ({e.kind.name}): {e}")
```

`UnpdfError` subclasses `RuntimeError`, so existing `except RuntimeError` handlers
keep working. `ErrorKind` values are part of the native ABI: new reasons take new
numbers and existing ones are never renumbered, so treat an unrecognised value as a
generic failure.

## License

MIT License
