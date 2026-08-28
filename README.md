# unpdf

[![Crates.io](https://img.shields.io/crates/v/unpdf.svg)](https://crates.io/crates/unpdf)
[![PyPI](https://img.shields.io/pypi/v/unpdf-markdown.svg)](https://pypi.org/project/unpdf-markdown/)
[![NuGet](https://img.shields.io/nuget/v/Unpdf.svg)](https://www.nuget.org/packages/Unpdf/)
[![npm](https://img.shields.io/npm/v/@iyulab/unpdf.svg)](https://www.npmjs.com/package/@iyulab/unpdf)
[![Documentation](https://docs.rs/unpdf/badge.svg)](https://docs.rs/unpdf)
[![CI](https://github.com/iyulab/unpdf/actions/workflows/ci.yml/badge.svg)](https://github.com/iyulab/unpdf/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

A high-performance Rust library for extracting content from PDF documents to structured Markdown, plain text, and JSON.

## Features

- **Comprehensive PDF support**: PDF 1.0-2.0, including compressed object streams
- **Encrypted PDF support**: RC4 and AES-128 decryption (auto-tries empty password)
- **Multiple output formats**: Markdown, Plain Text, JSON (with full metadata)
- **Structure preservation**: Headings, paragraphs, lists, tables, inline formatting
- **CJK text support**: Smart spacing for Korean, Chinese, Japanese with Adobe CMap resources
- **RTL text support**: Arabic and Hebrew with Unicode BiDi reordering
- **Form field extraction**: AcroForm fields (text, checkbox, radio, dropdown) with values
- **Multi-column layout**: Recursive XY-Cut algorithm for N-column detection
- **Asset extraction**: Images, fonts, and embedded resources
- **Extraction quality diagnostics**: Automatic detection and reporting of extraction issues
- **Text cleanup**: Multiple presets for LLM training data preparation
- **Self-update**: Built-in update mechanism via GitHub releases
- **WebAssembly / npm** (0.7.0+): `@iyulab/unpdf` — browser and Node.js via wasm-bindgen; [live playground](https://iyulab.github.io/unpdf/)
- **C-ABI FFI**: Native library for C#, Python, and other languages
- **Parallel processing**: Uses Rayon for multi-page documents
- **Streaming pipeline** (0.4.0+): `PdfParser::for_each_page` yields pages as they parse; peak memory bounded by window size regardless of document size
- **Deterministic page ordering**: Parallel page parsing emits results in `page_num` ASC order via internal reorder buffer
- **Image deduplication** (0.6.0+): Identical images across pages are written to disk only once; duplicate references are resolved to the canonical copy

---

## Table of Contents

- [Installation](#installation)
- [CLI Usage](#cli-usage)
- [Rust Library Usage](#rust-library-usage)
- [WebAssembly / JavaScript](#webassembly--javascript)
- [Python Integration](#python-integration)
- [C# / .NET Integration](#c--net-integration)
- [Output Formats](#output-formats)
- [Feature Flags](#feature-flags)
- [License](#license)

---

## Installation

### Pre-built Binaries (Recommended)

Download the latest release from [GitHub Releases](https://github.com/iyulab/unpdf/releases/latest).

#### Windows (x64)

```powershell
# Download and extract (replace VERSION with the actual version, e.g. v0.7.0)
$VERSION = (Invoke-RestMethod "https://api.github.com/repos/iyulab/unpdf/releases/latest").tag_name
Invoke-WebRequest -Uri "https://github.com/iyulab/unpdf/releases/latest/download/unpdf-windows-x86_64-${VERSION}.zip" -OutFile "unpdf.zip"
Expand-Archive -Path "unpdf.zip" -DestinationPath "."

# Move to a directory in PATH (optional)
Move-Item -Path "unpdf.exe" -Destination "$env:LOCALAPPDATA\Microsoft\WindowsApps\"

# Verify installation
unpdf version
```

#### Linux (x64)

```bash
# Download and extract (replace VERSION with the actual version, e.g. v0.7.0)
VERSION=$(curl -s "https://api.github.com/repos/iyulab/unpdf/releases/latest" | grep '"tag_name"' | cut -d'"' -f4)
curl -LO "https://github.com/iyulab/unpdf/releases/latest/download/unpdf-linux-x86_64-${VERSION}.tar.gz"
tar -xzf "unpdf-linux-x86_64-${VERSION}.tar.gz"

# Install to /usr/local/bin (requires sudo)
sudo mv unpdf /usr/local/bin/

# Or install to user directory
mkdir -p ~/.local/bin
mv unpdf ~/.local/bin/
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc
source ~/.bashrc

# Verify installation
unpdf version
```

#### macOS

```bash
# Intel Mac (replace VERSION with the actual version, e.g. v0.7.0)
VERSION=$(curl -s "https://api.github.com/repos/iyulab/unpdf/releases/latest" | grep '"tag_name"' | cut -d'"' -f4)
curl -LO "https://github.com/iyulab/unpdf/releases/latest/download/unpdf-macos-x86_64-${VERSION}.tar.gz"
tar -xzf "unpdf-macos-x86_64-${VERSION}.tar.gz"

# Apple Silicon (M1/M2/M3/M4)
curl -LO "https://github.com/iyulab/unpdf/releases/latest/download/unpdf-macos-aarch64-${VERSION}.tar.gz"
tar -xzf "unpdf-macos-aarch64-${VERSION}.tar.gz"

# Install
sudo mv unpdf /usr/local/bin/

# Verify
unpdf version
```

#### Available Binaries

| Platform | Architecture | File |
|----------|--------------|------|
| Windows | x64 | `unpdf-windows-x86_64-{version}.zip` |
| Linux | x64 | `unpdf-linux-x86_64-{version}.tar.gz` |
| Linux | x64 (musl) | `unpdf-linux-x86_64-musl-{version}.tar.gz` |
| macOS | Intel | `unpdf-macos-x86_64-{version}.tar.gz` |
| macOS | Apple Silicon | `unpdf-macos-aarch64-{version}.tar.gz` |

### Updating

unpdf includes a built-in self-update mechanism:

```bash
# Check for updates
unpdf update --check

# Update to latest version
unpdf update

# Force reinstall (even if on latest)
unpdf update --force
```

### Install via Cargo

If you have Rust installed:

```bash
# Install CLI
cargo install unpdf-cli

# Add library to your project
cargo add unpdf
```

---

## CLI Usage

### Quick Start

```bash
# Extract to Markdown + images (default)
unpdf document.pdf

# Specify output directory
unpdf document.pdf ./output

# With text cleanup for LLM training
unpdf document.pdf --cleanup aggressive
```

### Output Structure

```
document_output/
├── extract.md      # Markdown output with frontmatter
└── images/         # Extracted images (if any)
    ├── page1_img1.png
    └── page2_img1.jpg
```

Use `unpdf convert <file> --all` to produce all three formats at once:

```
document_output/
├── extract.md      # Markdown output with frontmatter
├── extract.txt     # Plain text output
├── content.json    # Full structured JSON
└── images/
```

### Commands

```bash
unpdf <file> [output]              # Convert to Markdown + extract images (default)
unpdf convert <file> [OPTIONS]     # Convert with full format/streaming control
unpdf markdown <file> [OPTIONS]    # Convert to Markdown only (alias: md)
unpdf text <file> [OPTIONS]        # Convert to plain text only
unpdf json <file> [OPTIONS]        # Convert to JSON only
unpdf info <file>                  # Show document information
unpdf extract <file> [OPTIONS]     # Extract images only
unpdf update [OPTIONS]             # Self-update to latest version
unpdf version                      # Show version information
```

### Convert (multi-format streaming pipeline)

```bash
# Markdown only (default)
unpdf convert document.pdf

# All formats + images
unpdf convert document.pdf --all -o ./output

# Specific formats
unpdf convert document.pdf --formats md,txt,json

# Skip image extraction
unpdf convert document.pdf --no-images

# Custom image directory
unpdf convert document.pdf --image-dir ./assets

# Drop images smaller than 128px (filter decorative icons)
unpdf convert document.pdf --min-image-size 128

# Tune streaming window size (pages in-flight, default: auto)
unpdf convert document.pdf --window 4
```

#### Convert Options

| Option | Description | Default |
|--------|-------------|---------|
| `-o, --output` | Output directory | `<stem>_output/` |
| `--formats` | Comma-separated formats: `md,txt,json` | `md` |
| `--all` | Output all formats (MD + TXT + JSON) | false |
| `--no-images` | Skip image extraction | false |
| `--image-dir` | Custom image output directory | `<out>/images` |
| `--min-image-size` | Min pixel dimension; smaller images skipped | 64 |
| `--window` | Streaming window size (pages in-flight) | auto |
| `--keep-ocr-text` | Keep a scan's OCR text layer even when it recognised nothing readable | false |
| `--cleanup` | Text cleanup: `minimal`, `standard`, `aggressive` | none |
| `--page-markers` | Insert `<!-- page N -->` markers | false |
| `-q, --quiet` | Suppress progress and warnings | false |

### Convert to Markdown

```bash
# Basic conversion (output to stdout)
unpdf markdown document.pdf

# Save to file
unpdf markdown document.pdf -o output.md

# With YAML frontmatter
unpdf markdown document.pdf --frontmatter -o output.md

# With text cleanup for LLM training
unpdf markdown document.pdf --cleanup standard -o cleaned.md

# Table rendering options
unpdf markdown document.pdf --table-mode html -o output.md

# Specify page range
unpdf markdown document.pdf --pages 1-10 -o output.md

# Insert page boundary markers for AI pipeline / RAG use
unpdf markdown document.pdf --page-markers -o output.md
```

#### Markdown Options

| Option | Description | Default |
|--------|-------------|---------|
| `-o, --output` | Output file path | stdout |
| `-f, --frontmatter` | Include YAML frontmatter | false |
| `--table-mode` | Table rendering: `markdown`, `html`, `ascii` | markdown |
| `--cleanup` | Text cleanup: `minimal`, `standard`, `aggressive` | none |
| `--max-heading` | Maximum heading level (1-6) | 6 |
| `--pages` | Page range (e.g., `1-10`, `1,3,5`) | all |
| `--page-markers` | Insert `<!-- page N -->` markers at page boundaries | false |
| `-q, --quiet` | Suppress quality warnings (root-level flag: `unpdf --quiet markdown ...`) | false |

### Convert to Plain Text

```bash
# Basic extraction
unpdf text document.pdf

# With cleanup
unpdf text document.pdf --cleanup standard -o output.txt

# Specific pages
unpdf text document.pdf --pages 1-5 -o output.txt
```

### Convert to JSON

```bash
# Pretty-printed JSON
unpdf json document.pdf -o output.json

# Compact JSON
unpdf json document.pdf --compact -o output.json
```

### Show Document Information

```bash
unpdf info document.pdf
```

Output:
```
Document Information
────────────────────────────────────────
File: document.pdf
Format: PDF 1.7
Pages: 42
Encrypted: No
Title: My Document
Author: John Doe
Creator: Microsoft Word
Producer: Adobe PDF Library
Created: 2025-01-15T10:30:00Z
Modified: 2025-01-20T14:45:00Z

Content Statistics
────────────────────────────────────────
Words: 12500
Characters: 75000
Images: 15
```

### Extract Images

```bash
# Extract to current directory
unpdf extract document.pdf

# Extract to specific directory
unpdf extract document.pdf -o ./images

# Extract specific pages
unpdf extract document.pdf --pages 1-5 -o ./images
```

### Self-Update

```bash
# Check for updates
unpdf update --check

# Update to latest version
unpdf update

# Force reinstall
unpdf update --force
```

### Examples

```bash
# Convert PDF to Markdown with frontmatter
unpdf md report.pdf --frontmatter -o report.md

# Convert with aggressive cleanup for AI training
unpdf md document.pdf --cleanup aggressive -o cleaned.md

# Batch conversion (shell)
for f in *.pdf; do unpdf md "$f" -o "${f%.pdf}.md"; done

# Batch conversion (PowerShell)
Get-ChildItem *.pdf | ForEach-Object { unpdf md $_.FullName -o "$($_.BaseName).md" }
```

---

## Rust Library Usage

### Quick Start

```rust
use unpdf::{parse_file, render};

fn main() -> unpdf::Result<()> {
    // Parse PDF document
    let doc = parse_file("document.pdf")?;

    // Convert to Markdown
    let options = render::RenderOptions::default();
    let markdown = render::to_markdown(&doc, &options)?;
    println!("{}", markdown);

    // Get plain text
    let text = render::to_text(&doc, &options)?;

    // Get JSON
    let json = render::to_json(&doc, render::JsonFormat::Pretty)?;

    Ok(())
}
```

### Builder API

`Unpdf` provides a fluent builder for the common parse-then-render workflow:

```rust
use unpdf::{Unpdf, CleanupPreset, TableFallback, PageSelection};

let markdown = Unpdf::new()
    .lenient()
    .with_frontmatter()
    .with_cleanup(CleanupPreset::Aggressive)
    .with_table_fallback(TableFallback::Html)
    .with_pages(PageSelection::Range(1..=20))
    .parse("document.pdf")?
    .to_markdown()?;
```

### Convenience Functions

```rust
// Parse from bytes or a reader
let doc = unpdf::parse_bytes(&pdf_bytes)?;
let doc = unpdf::parse_reader(std::fs::File::open("doc.pdf")?)?;

// Password-protected PDF
let doc = unpdf::parse_file_with_password("protected.pdf", "secret")?;

// One-shot conversions without building a Document first
let text     = unpdf::extract_text("document.pdf")?;
let markdown = unpdf::to_markdown("document.pdf")?;
let json     = unpdf::to_json("document.pdf", unpdf::JsonFormat::Pretty)?;
```

### Render Options

```rust
use unpdf::render::{RenderOptions, CleanupPreset, TableFallback};
use unpdf::PageMarkerStyle;

let options = RenderOptions::new()
    .with_frontmatter(true)
    .with_table_fallback(TableFallback::Html)
    .with_cleanup_preset(CleanupPreset::Aggressive)
    .with_max_heading(3)
    .with_page_range(1..=10)
    .with_page_markers(PageMarkerStyle::Comment); // <!-- page N --> at each page boundary

let markdown = render::to_markdown(&doc, &options)?;
```

### Working with Document Structure

```rust
use unpdf::parse_file;

let doc = parse_file("document.pdf")?;

// Access metadata
println!("Title: {:?}", doc.metadata.title);
println!("Author: {:?}", doc.metadata.author);
println!("Pages: {}", doc.page_count());
println!("PDF Version: {}", doc.metadata.pdf_version);

// Iterate pages
for (page_num, page) in doc.pages.iter().enumerate() {
    println!("Page {}: {} blocks", page_num + 1, page.elements.len());
    for element in &page.elements {
        // Process paragraphs, tables, images, etc.
    }
}

// Extract images
for (id, resource) in &doc.resources {
    if resource.is_image() {
        let filename = resource.suggested_filename(id);
        std::fs::write(&filename, &resource.data)?;
    }
}
```

### Page Range Selection

```rust
use unpdf::{parse_file, PageSelection};

// Parse only specific pages
let doc = parse_file_with_options("large.pdf", ParseOptions {
    pages: PageSelection::Range(1..=10),
    ..Default::default()
})?;

// Or parse all and render specific pages
let doc = parse_file("document.pdf")?;
let options = RenderOptions::new()
    .with_pages(vec![1, 3, 5, 7]);
let markdown = render::to_markdown(&doc, &options)?;
```

### Handling Encrypted PDFs

unpdf automatically decrypts PDFs that use empty user passwords (owner-password-only protection). For password-protected PDFs, provide the password:

```rust
use unpdf::{parse_file, parse_file_with_options, ParseOptions};

// Auto-decrypts owner-password-only PDFs
let doc = parse_file("restricted.pdf")?;

// Provide a password for user-password-protected PDFs
let options = ParseOptions::new().with_password("secret");
let doc = parse_file_with_options("protected.pdf", options)?;

// Check extraction quality
if let Some(warning) = doc.extraction_quality.warning_message() {
    eprintln!("{}", warning);
}
```

### Working with Form Fields

```rust
use unpdf::parse_file;

let doc = parse_file("form.pdf")?;
for field in &doc.form_fields {
    println!("{}: {}", field.name, field.display_value());
}
```

### Classifying Failures

`Error::kind()` returns a stable `ErrorKind` so you can branch on *why* a call failed
without matching on the message — useful when several failures reach the user as the
same "extraction failed":

```rust
use unpdf::{parse_file, ErrorKind};

match parse_file("document.pdf") {
    Ok(doc) => println!("{}", doc.page_count()),
    Err(e) => match e.kind() {
        ErrorKind::Encrypted | ErrorKind::InvalidPassword => eprintln!("Password required"),
        ErrorKind::Corrupted | ErrorKind::PdfParse => eprintln!("The file is damaged"),
        ErrorKind::UnknownFormat => eprintln!("Not a PDF"),
        _ => eprintln!("Extraction failed: {e}"),
    },
}
```

There is one `ErrorKind` variant per `Error` variant. The discriminants are explicit
and part of the public contract — they cross the C ABI as `unpdf_last_error_kind`
return values, so existing values are never renumbered.

### Detecting Incomplete Extraction

A damaged PDF does not always fail. When the cross-reference table survives but the
objects it points at do not, the parser recovers the pages it can read and returns
them — a success over an incomplete page set. `extraction_quality` reports that:

```rust
let doc = unpdf::parse_file("document.pdf")?;
let q = &doc.extraction_quality;

if q.pages_incomplete {
    // Extraction succeeded, but pages are missing from the output.
    eprintln!(
        "incomplete: extracted {} page(s), document declares {:?}",
        doc.page_count(),
        q.declared_page_count
    );
}
```

| Field | Meaning |
|-------|---------|
| `pages_incomplete` | Pages are known to be missing. The one bit to branch on. |
| `declared_page_count` | Page count the document declares (`/Count`), or `None` if that was unreadable too. |
| `unresolved_page_nodes` | Unreadable page-tree *nodes*. Non-zero means incomplete — **not** a count of lost pages. |
| `skipped_object_count` | Objects that could not be loaded. Most cost no page (fonts, annotations), so this alone does not imply missing text. |
| `suppressed_text_runs` | Text runs the font decoder could not read and discarded. Non-zero means text is missing from an otherwise successful extraction. |
| `unsupported_image_count` | Embedded images recognized as image XObjects but not extractable (unsupported color space/bit depth), so dropped. Distinguishes "no image" from "image present but couldn't be extracted". |

`suppressed_text_runs` covers a second way content goes missing without an error. When a
font's character codes cannot be resolved — a composite (Type0/CID) font with no usable
`ToUnicode` map, most often — the decoder discards the run rather than emit the raw bytes
as mojibake. That is the right call, but the discarded text is content the document had
and the output does not. The count is in *runs* (a `Tj` operand, or one element of a `TJ`
array), not characters: text that was never decoded has no knowable length. Treat any
non-zero value as "incomplete"; compare magnitudes between documents, not against a total.

Worth surfacing wherever extraction feeds an index or archive: a page that silently
never arrived is indistinguishable from a page that never existed, so the omission
shows up later as a search result that isn't there rather than as an error.
`warning_message()` already includes this case, ahead of the other warnings.

---

## WebAssembly / JavaScript

```bash
npm install @iyulab/unpdf
```

**[Live playground →](https://iyulab.github.io/unpdf/)**  Drag and drop a PDF in the browser; no install required.

### Browser / Bundler (webpack, vite)

```js
import init, { parse, ParseOptions } from '@iyulab/unpdf';

await init();

const response = await fetch('document.pdf');
const bytes = new Uint8Array(await response.arrayBuffer());

const doc = parse(bytes);
console.log(doc.toMarkdown());
console.log(doc.toText());
console.log(`Pages: ${doc.pageCount()}`);
```

### Node.js

```js
const { parse } = require('@iyulab/unpdf');
const fs = require('fs');

const bytes = new Uint8Array(fs.readFileSync('document.pdf'));
const doc = parse(bytes);
console.log(doc.toText());
```

### With Options

```js
import init, { parseWithOptions, ParseOptions } from '@iyulab/unpdf';

await init();

const opts = new ParseOptions()
  .lenient()
  .withPassword('secret')
  .withPages(1, 10);

const doc = parseWithOptions(bytes, opts);
console.log(doc.toMarkdown());
```

### API Reference

**Functions**

| Function | Signature | Description |
|----------|-----------|-------------|
| `parse` | `(data: Uint8Array) => PdfDocument` | Parse PDF bytes |
| `parseWithOptions` | `(data: Uint8Array, opts: ParseOptions) => PdfDocument` | Parse with options |

**PdfDocument**

| Method | Returns | Description |
|--------|---------|-------------|
| `PdfDocument.fromBytes(data)` | `PdfDocument` | Parse PDF bytes (static; same as `parse`) |
| `toMarkdown()` | `string` | Convert to Markdown |
| `toText()` | `string` | Convert to plain text |
| `toJson()` | `string` | Convert to JSON |
| `pageCount()` | `number` | Total page count |
| `metadata()` | `string` | Metadata as JSON string |
| `extractionQuality()` | `string` | Extraction diagnostics as JSON string — check `pages_incomplete` before indexing the result |

**ParseOptions**

| Method | Description |
|--------|-------------|
| `new()` | Default options |
| `lenient()` | Ignore recoverable errors |
| `textOnly()` | Skip image extraction |
| `withPassword(pw: string)` | Set decryption password |
| `withPages(from: number, to: number)` | Page range (1-indexed) |

---

## Python Integration

Install the Python package:

```bash
pip install unpdf-markdown
```

### Basic Usage

```python
from unpdf import to_markdown, to_text, to_json, get_info

# Convert PDF to Markdown
markdown = to_markdown("document.pdf")

# Convert to plain text
text = to_text("document.pdf")

# Convert to JSON
json_data = to_json("document.pdf")

# Get document information
info = get_info("document.pdf")
print(f"Title: {info.get('title')}")
print(f"Pages: {info.get('section_count')}")
```

`get_info` returns only the keys it found: `title` and `author` are absent when the
document does not set them, and the page count is `section_count`.

### Working with Paths and Bytes

Every function accepts a path (`str` or any `os.PathLike`, so `pathlib.Path` works) or
the PDF's own bytes. The two are told apart by type, so there is no ambiguity:

```python
from pathlib import Path
from unpdf import to_markdown

to_markdown("document.pdf")           # str path
to_markdown(Path("document.pdf"))     # pathlib.Path
to_markdown(pdf_bytes)                # bytes — parsed in memory, no temp file
```

Bytes input goes through the native in-memory parser, so uploads and blobs need no
detour through the filesystem.

### Check PDF Validity

```python
from unpdf import is_pdf

# Check if file is a valid PDF
if is_pdf("document.pdf"):
    markdown = to_markdown("document.pdf")
```

### Detecting Scanned (Image-only) PDFs

Empty extraction output can mean a scanned document (no text layer), a genuinely
blank page, or a parse failure. The introspection surface tells them apart:

```python
from unpdf import get_extraction_quality, get_page_stats

# Document level
quality = get_extraction_quality("scan.pdf")
if quality["is_scan_pdf"]:
    print("Scanned document - OCR required")

# Page level (works for mixed documents too)
stats = get_page_stats("scan.pdf", 1)
if stats["text_op_count"] == 0 and stats["image_op_count"] > 0:
    print("Page 1 is image-only (scanned)")
elif stats["text_op_count"] == 0:
    print("Page 1 is genuinely blank")
```

Note: a *searchable* scan (page image plus an invisible OCR text layer) reports
`text_op_count > 0` — combine the check with `ocr_text_suppressed`, which flags
pages whose unreadable OCR layer was dropped.

The same call reports whether the document was damaged badly enough to lose pages —
extraction can succeed over an incomplete page set:

```python
quality = get_extraction_quality("document.pdf")
if quality["pages_incomplete"]:
    print(f"incomplete - document declares {quality['declared_page_count']} page(s)")
```

`unresolved_page_nodes` counts unreadable page-tree *nodes*, not lost pages — one
unreadable node can cost a whole subtree, so treat any non-zero value as "incomplete"
and nothing more.

### Handling Failures

The checks above need a parsed document. When parsing itself fails, `UnpdfError`
carries a `kind` so you can branch on the reason instead of matching on message text:

```python
from unpdf import to_text, ErrorKind, UnpdfError

try:
    text = to_text("document.pdf")
except UnpdfError as e:
    if e.kind == ErrorKind.ENCRYPTED:
        print("Password required")
    elif e.kind in (ErrorKind.CORRUPTED, ErrorKind.PDF_PARSE):
        print("The file is damaged")
    elif e.kind == ErrorKind.UNKNOWN_FORMAT:
        print("Not a PDF")
    else:
        print(f"Extraction failed ({e.kind.name}): {e}")
```

`UnpdfError` subclasses `RuntimeError`, so existing `except RuntimeError` handlers
keep working. `ErrorKind` values are part of the native ABI: new reasons take new
numbers and existing ones are never renumbered, so treat an unrecognised value as a
generic failure.

---

## C# / .NET Integration

unpdf provides C-ABI compatible bindings for integration with C# and .NET applications.

### Installation via NuGet

```bash
dotnet add package Unpdf
```

Or via Package Manager Console:

```powershell
Install-Package Unpdf
```

### Getting the Native Library (Manual)

Alternatively, download from [GitHub Releases](https://github.com/iyulab/unpdf/releases):

| Platform | Library File |
|----------|-------------|
| Windows x64 | `unpdf.dll` |
| Linux x64 | `libunpdf.so` |
| macOS | `libunpdf.dylib` |

Or build from source:

```bash
cargo build --release --features ffi
```

### C# Wrapper Usage

The API is handle-based: parse once into an `UnpdfDocument`, then read from it. The
handle owns native memory, so dispose it (`using`).

```csharp
using Unpdf;

using var doc = UnpdfDocument.ParseFile("document.pdf");

string markdown = doc.ToMarkdown();
string text = doc.ToText();
string json = doc.ToJson(compact: false);
string plain = doc.PlainText();

// Document facts
Console.WriteLine($"Title: {doc.Title}, Pages: {doc.SectionCount}");

// Markdown options
string withFrontmatter = doc.ToMarkdown(new MarkdownOptions
{
    IncludeFrontmatter = true,
    EscapeSpecialChars = true,
    PageMarkers = true,
});

// A single page (1-indexed)
string page1 = doc.PageToMarkdown(1);

// Embedded resources (images and the like), by id
foreach (var id in doc.GetResourceIds())
{
    byte[]? bytes = doc.GetResourceData(id);
    if (bytes is not null)
        File.WriteAllBytes(Path.Combine("./images", id), bytes);
}
```

Parsing from memory works the same way: `UnpdfDocument.ParseBytes(byte[])`.

Note: `SectionCount` is the page count. `ResourceCount` counts embedded resources,
which is not the same as the number of images — filter with `GetResourceInfo(id)` if
you need images specifically.

### Detecting Scanned (Image-only) PDFs

Empty extraction output can mean a scanned document (no text layer), a genuinely
blank page, or a parse failure. The introspection surface tells them apart:

```csharp
using Unpdf;

using var doc = UnpdfDocument.ParseFile("scan.pdf");

// Document level
var quality = doc.GetExtractionQuality();
if (quality.IsScanPdf)
    Console.WriteLine("Scanned document - OCR required");

// Page level (works for mixed documents too)
var stats = doc.GetPageStats(1);
if (stats.TextOpCount == 0 && stats.ImageOpCount > 0)
    Console.WriteLine("Page 1 is image-only (scanned)");
else if (stats.TextOpCount == 0)
    Console.WriteLine("Page 1 is genuinely blank");
```

Note: a *searchable* scan (page image plus an invisible OCR text layer) reports
`TextOpCount > 0` — combine the check with `OcrTextSuppressed`, which flags
pages whose unreadable OCR layer was dropped.

The same object reports whether the document was damaged badly enough to lose pages.
Extraction can succeed over an incomplete page set, and a page that silently never
arrived looks exactly like a page that never existed:

```csharp
var quality = doc.GetExtractionQuality();
if (quality.PagesIncomplete)
    Console.WriteLine(
        $"incomplete: got {doc.SectionCount} page(s), " +
        $"document declares {quality.DeclaredPageCount}");
```

`UnresolvedPageNodes` counts unreadable page-tree *nodes*, not lost pages — one
unreadable node can cost a whole subtree. Treat any non-zero value as "incomplete".

### Handling Failures

The checks above need a parsed document. When parsing itself fails,
`UnpdfException.Kind` says why, so you can branch on the reason instead of matching
on `Message`:

```csharp
try
{
    using var doc = UnpdfDocument.ParseFile("document.pdf");
    Console.WriteLine(doc.ToMarkdown());
}
catch (UnpdfException e)
{
    switch (e.Kind)
    {
        case UnpdfErrorKind.Encrypted:
            Console.WriteLine("Password required");
            break;
        case UnpdfErrorKind.Corrupted:
        case UnpdfErrorKind.PdfParse:
            Console.WriteLine("The file is damaged");
            break;
        case UnpdfErrorKind.UnknownFormat:
            Console.WriteLine("Not a PDF");
            break;
        default:
            Console.WriteLine($"Extraction failed ({e.Kind}): {e.Message}");
            break;
    }
}
```

`UnpdfErrorKind` values are part of the native ABI: new reasons take new numbers and
existing ones are never renumbered, so treat an unrecognised value as a generic
failure. A failure raised by the managed wrapper rather than the native library
reports `UnpdfErrorKind.Other`.

### ASP.NET Core Example

```csharp
[ApiController]
[Route("api/[controller]")]
public class PdfController : ControllerBase
{
    [HttpPost("convert")]
    public async Task<IActionResult> ConvertPdf(IFormFile file)
    {
        if (file == null) return BadRequest("No file");

        // Parse straight from the uploaded bytes — no temp file needed.
        using var buffer = new MemoryStream();
        await file.CopyToAsync(buffer);

        try
        {
            using var doc = UnpdfDocument.ParseBytes(buffer.ToArray());
            var markdown = doc.ToMarkdown(new MarkdownOptions { IncludeFrontmatter = true });

            // Success is not the same as complete: report a damaged page set instead of
            // returning a short document as if it were whole.
            var quality = doc.GetExtractionQuality();
            return Ok(new
            {
                markdown,
                pages = doc.SectionCount,
                incomplete = quality.PagesIncomplete,
                declaredPages = quality.DeclaredPageCount,
            });
        }
        catch (UnpdfException ex)
        {
            return BadRequest(new { error = ex.Message, kind = ex.Kind.ToString() });
        }
    }

    [HttpPost("extract-images")]
    public async Task<IActionResult> ExtractImages(IFormFile file)
    {
        if (file == null) return BadRequest("No file");

        using var buffer = new MemoryStream();
        await file.CopyToAsync(buffer);

        try
        {
            using var doc = UnpdfDocument.ParseBytes(buffer.ToArray());
            var ids = doc.GetResourceIds();
            return Ok(new { count = ids.Length, ids });
        }
        catch (UnpdfException ex)
        {
            return BadRequest(new { error = ex.Message, kind = ex.Kind.ToString() });
        }
    }
}
```

---

## Output Formats

### Markdown

Structured Markdown with preserved formatting:

- **Headings**: Document headings (detected from font size/style) -> `#`, `##`, `###`
- **Paragraphs**: Text blocks with proper spacing
- **Lists**: Detected ordered and unordered lists
- **Tables**: Markdown tables (with HTML/ASCII fallback for complex layouts)
- **Inline styles**: Bold (`**`), italic (`*`)
- **Hyperlinks**: Preserved as Markdown links
- **Images**: Reference-style image links

### Plain Text

Pure text content without formatting markers.

### JSON

Complete document structure with metadata:

```json
{
  "metadata": {
    "title": "Document Title",
    "author": "Author Name",
    "creator": "Application Name",
    "producer": "PDF Library",
    "pdf_version": "1.7",
    "page_count": 42,
    "created": "2025-01-15T10:30:00Z",
    "modified": "2025-01-20T14:45:00Z"
  },
  "pages": [...],
  "resources": [...]
}
```

---

## Supported PDF Features

| Feature | Status |
|---------|--------|
| PDF 1.0 - 2.0 | Supported |
| Compressed object streams (ObjStm) | Supported |
| Cross-reference streams (XRef streams) | Supported |
| Linearized PDFs | Supported |
| Encrypted PDFs (RC4, AES-128) | Supported |
| Text extraction | Supported |
| CJK text (Korean, Chinese, Japanese) | Supported (Adobe CMap) |
| RTL text (Arabic, Hebrew) | Supported (BiDi) |
| CIDFont / ToUnicode CMap decoding | Supported |
| Embedded TrueType font decoding | Supported |
| Multi-column layout detection | Supported (XY-Cut) |
| Table detection | Supported |
| Form fields (AcroForms) | Supported |
| Image extraction (JPEG, JP2) | Supported |
| Bookmarks/Outlines | Supported |
| Extraction quality diagnostics | Supported |
| AES-256 encryption (R5-R6) | Not yet supported |
| Digital signatures | Metadata only |
| OCR (image-based PDFs) | Planned |

---

## Feature Flags

| Feature | Description | Default |
|---------|-------------|---------|
| `fast-parse` | Enable optimised nom-based PDF tokeniser | Yes |
| `ffi` | C-ABI foreign function interface | No |
| `async` | Async I/O with Tokio | No |

```toml
# Cargo.toml - enable features
[dependencies]
unpdf = { version = "0.7", features = ["ffi", "async"] }
```

---

## Performance

- Custom zero-dependency PDF parser (no external C libraries)
- Parallel page processing with Rayon
- Memory-efficient handling of large documents
- Streaming support for very large files

---

## License

MIT License - see [LICENSE](LICENSE) for details.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## Related Projects

- [unhwp](https://github.com/iyulab/unhwp) - Korean HWP document extraction
- [undoc](https://github.com/iyulab/undoc) - Microsoft Office document extraction (DOCX, XLSX, PPTX)
