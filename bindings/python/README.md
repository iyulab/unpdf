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

## API Reference

### `to_markdown(path: str) -> str`
Convert a PDF file to Markdown format.

### `to_text(path: str) -> str`
Convert a PDF file to plain text.

### `to_json(path: str, pretty: bool = False) -> str`
Convert a PDF file to JSON format.

### `get_info(path: str) -> dict`
Get document metadata (title, author, page count, etc.)

### `get_page_count(path: str) -> int`
Get the number of pages in a PDF file.

### `is_pdf(path: str) -> bool`
Check if a file is a valid PDF.

### `version() -> str`
Get the version of the native library.

### `get_extraction_quality(path: str) -> dict`
Document-level extraction diagnostics: `char_count`, `word_count`,
`replacement_char_count`, `encrypted`, `is_scan_pdf`, `suppressed_ocr_pages`.

### `get_page_stats(path: str, page_number: int) -> dict`
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
