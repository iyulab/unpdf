# un-pdf

Python bindings for [unpdf](https://github.com/iyulab/unpdf) - High-performance PDF content extraction to Markdown, text, and JSON.

## Installation

```bash
pip install un-pdf
```

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

## License

MIT License
