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

Every function below also accepts an optional `options: dict` keyword argument —
see [Parsing options](#parsing-options).

### `to_markdown(source: PdfSource, flags: int = 0, options: dict | None = None) -> str`
Convert a PDF file to Markdown format. `flags` is a bitwise OR of:

| Constant | Effect |
|---|---|
| `UNPDF_FLAG_FRONTMATTER` | Emit YAML frontmatter with document metadata |
| `UNPDF_FLAG_ESCAPE_SPECIAL` | Escape Markdown special characters |
| `UNPDF_FLAG_PAGE_MARKERS` | Mark each page boundary with `<!-- page N -->` |

```python
import unpdf

markdown = unpdf.to_markdown(
    "document.pdf",
    unpdf.UNPDF_FLAG_FRONTMATTER | unpdf.UNPDF_FLAG_PAGE_MARKERS,
)
```

### `to_text(source: PdfSource, options: dict | None = None) -> str`
Convert a PDF file to plain text.

### `to_json(source: PdfSource, pretty: bool = False, options: dict | None = None) -> str`
Convert a PDF file to JSON format.

### `get_info(source: PdfSource, options: dict | None = None) -> dict`
Get document metadata. Keys: `section_count` (the page count), `resource_count`,
plus `title` / `author` only when the document sets them.

### `get_page_count(source: PdfSource, options: dict | None = None) -> int`
Get the number of pages in a PDF file.

### `is_pdf(source: PdfSource, options: dict | None = None) -> bool`
Check if a file is a valid PDF.

### `version() -> str`
Get the version of the native library.

### `get_extraction_quality(source: PdfSource, options: dict | None = None) -> dict`
Document-level extraction diagnostics: `char_count`, `word_count`,
`replacement_char_count`, `encrypted`, `is_scan_pdf`, `suppressed_ocr_pages`,
`suppressed_text_runs`,
`pages_incomplete`, `declared_page_count`, `unresolved_page_nodes`,
`skipped_object_count`. See "Incomplete extraction" below.

### `get_page_stats(source: PdfSource, page_number: int, options: dict | None = None) -> dict`
Per-page content-stream operator counts (1-indexed): `page`, `text_op_count`,
`image_op_count`, `ocr_text_suppressed`.

### `get_resource_ids(source: PdfSource, options: dict | None = None) -> list[str]`
### `get_resource_info(source: PdfSource, resource_id: str, options: dict | None = None) -> dict`
### `get_resource_data(source: PdfSource, resource_id: str, options: dict | None = None) -> bytes`
List and retrieve extracted embedded resources (images). See
[Embedded resources](#embedded-resources) — these only return anything once
`extract_resources` is enabled via `options`.

## Parsing options

Every function accepts `options: dict | None = None`. Every key is optional; an
absent key keeps unpdf's own default:

| Key | Type | Default | Meaning |
|---|---|---|---|
| `error_mode` | `"strict"` \| `"lenient"` | `"lenient"` | Fail on any parse error, or skip invalid content and continue. |
| `extract_mode` | `"full"` \| `"text_only"` \| `"structure_only"` | `"full"` | What to extract. |
| `extract_resources` | `bool` | `False` | Populate the resource inventory `get_resource_ids` etc. read from. Off by default — bounds peak memory on large PDFs. |
| `min_image_dimension` | `int` | `64` | Images below this on either axis are dropped as decorative (logos, rule lines, tracking pixels). `0` keeps every image. |
| `parallel` | `bool` | `True` | Multi-threaded page processing. |
| `password` | `str` | — | Password for encrypted documents. |
| `suppress_low_confidence_ocr` | `bool` | `True` | Drop an invisible OCR text layer whose recognized text is not readable. |

```python
info = unpdf.get_info("document.pdf", options={"extract_resources": True})
print(info["resource_count"])
```

## Embedded resources

```python
options = {"extract_resources": True, "min_image_dimension": 0}
data = open("document.pdf", "rb").read()

for resource_id in unpdf.get_resource_ids(data, options=options):
    info = unpdf.get_resource_info(data, resource_id, options=options)
    image_bytes = unpdf.get_resource_data(data, resource_id, options=options)
    # ...
```

Each call above re-parses the document — `options` must be passed identically to
every call in the sequence, since Python has no persistent document handle the way
the C#/Rust APIs do.

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
inventory, which is empty unless `options={"extract_resources": True}` (see
[Parsing options](#parsing-options)) — it is not a count of images on the page. Use
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
