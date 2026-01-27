# unpdf

[![Crates.io](https://img.shields.io/crates/v/unpdf.svg)](https://crates.io/crates/unpdf)
[![Documentation](https://docs.rs/unpdf/badge.svg)](https://docs.rs/unpdf)
[![CI](https://github.com/iyulab/unpdf/actions/workflows/ci.yml/badge.svg)](https://github.com/iyulab/unpdf/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

A high-performance Rust library for extracting content from PDF documents to structured Markdown, plain text, and JSON.

## Features

- **Comprehensive PDF support**: PDF 1.0-2.0, including compressed object streams
- **Multiple output formats**: Markdown, Plain Text, JSON (with full metadata)
- **Structure preservation**: Headings, paragraphs, lists, tables, inline formatting
- **CJK text support**: Smart spacing for Korean, Chinese, Japanese content
- **Asset extraction**: Images, fonts, and embedded resources
- **Text cleanup**: Multiple presets for LLM training data preparation
- **Self-update**: Built-in update mechanism via GitHub releases
- **C-ABI FFI**: Native library for C#, Python, and other languages
- **Parallel processing**: Uses Rayon for multi-page documents

---

## Table of Contents

- [Installation](#installation)
- [CLI Usage](#cli-usage)
- [Rust Library Usage](#rust-library-usage)
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
# Download and extract
Invoke-WebRequest -Uri "https://github.com/iyulab/unpdf/releases/latest/download/unpdf-cli-x86_64-pc-windows-msvc.zip" -OutFile "unpdf.zip"
Expand-Archive -Path "unpdf.zip" -DestinationPath "."

# Move to a directory in PATH (optional)
Move-Item -Path "unpdf.exe" -Destination "$env:LOCALAPPDATA\Microsoft\WindowsApps\"

# Verify installation
unpdf version
```

#### Linux (x64)

```bash
# Download and extract
curl -LO https://github.com/iyulab/unpdf/releases/latest/download/unpdf-cli-x86_64-unknown-linux-gnu.tar.gz
tar -xzf unpdf-cli-x86_64-unknown-linux-gnu.tar.gz

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
# Intel Mac
curl -LO https://github.com/iyulab/unpdf/releases/latest/download/unpdf-cli-x86_64-apple-darwin.tar.gz
tar -xzf unpdf-cli-x86_64-apple-darwin.tar.gz

# Apple Silicon (M1/M2/M3/M4)
curl -LO https://github.com/iyulab/unpdf/releases/latest/download/unpdf-cli-aarch64-apple-darwin.tar.gz
tar -xzf unpdf-cli-aarch64-apple-darwin.tar.gz

# Install
sudo mv unpdf /usr/local/bin/

# Verify
unpdf version
```

#### Available Binaries

| Platform | Architecture | File |
|----------|--------------|------|
| Windows | x64 | `unpdf-cli-x86_64-pc-windows-msvc.zip` |
| Linux | x64 | `unpdf-cli-x86_64-unknown-linux-gnu.tar.gz` |
| macOS | Intel | `unpdf-cli-x86_64-apple-darwin.tar.gz` |
| macOS | Apple Silicon | `unpdf-cli-aarch64-apple-darwin.tar.gz` |

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
# Extract all formats (Markdown, text, JSON) + images to output directory
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
├── extract.txt     # Plain text output
├── content.json    # Full structured JSON
└── images/         # Extracted images
    ├── page1_img1.png
    └── page2_img1.jpg
```

### Commands

```bash
unpdf <file> [output]              # Extract all formats (default)
unpdf convert <file> [OPTIONS]     # Same as above, explicit command
unpdf markdown <file> [OPTIONS]    # Convert to Markdown only (alias: md)
unpdf text <file> [OPTIONS]        # Convert to plain text only
unpdf json <file> [OPTIONS]        # Convert to JSON only
unpdf info <file>                  # Show document information
unpdf extract <file> [OPTIONS]     # Extract images only
unpdf update [OPTIONS]             # Self-update to latest version
unpdf version                      # Show version information
```

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

# Extract text from scanned PDF (requires OCR feature)
unpdf text scanned.pdf --ocr -o output.txt

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

### Render Options

```rust
use unpdf::render::{RenderOptions, CleanupPreset, TableFallback};

let options = RenderOptions::new()
    .with_frontmatter(true)
    .with_table_fallback(TableFallback::Html)
    .with_cleanup_preset(CleanupPreset::Aggressive)
    .with_max_heading(3)
    .with_page_range(1..=10);

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

```rust
use unpdf::{parse_file_with_password, Error};

match parse_file("encrypted.pdf") {
    Ok(doc) => println!("Document parsed"),
    Err(Error::Encrypted) => {
        // Try with password
        let doc = parse_file_with_password("encrypted.pdf", "secret")?;
        println!("Document decrypted and parsed");
    }
    Err(e) => return Err(e),
}
```

---

## C# / .NET Integration

unpdf provides C-ABI compatible bindings for integration with C# and .NET applications.

### Getting the Native Library

Download from [GitHub Releases](https://github.com/iyulab/unpdf/releases):

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

```csharp
using Unpdf;

// Parse and convert to Markdown
string markdown = UnpdfNative.ToMarkdown("document.pdf");

// Parse and convert to plain text
string text = UnpdfNative.ToText("document.pdf");

// Parse and convert to JSON
string json = UnpdfNative.ToJson("document.pdf");

// From byte array
byte[] data = File.ReadAllBytes("document.pdf");
string markdown = UnpdfNative.ToMarkdownFromBytes(data);

// With options
var options = new ConversionOptions
{
    EnableCleanup = true,
    CleanupPreset = CleanupPreset.Aggressive,
    IncludeFrontmatter = true,
    PageRange = "1-10"
};
string markdown = UnpdfNative.ToMarkdown("document.pdf", options);
```

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

        using var ms = new MemoryStream();
        await file.CopyToAsync(ms);

        try
        {
            var markdown = UnpdfNative.ToMarkdownFromBytes(
                ms.ToArray(),
                enableCleanup: true
            );
            return Ok(new { markdown });
        }
        catch (UnpdfException ex)
        {
            return BadRequest(new { error = ex.Message });
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
| Compressed object streams | Supported |
| Text extraction | Supported |
| Table detection | Supported |
| Image extraction | Supported |
| Hyperlinks | Supported |
| Bookmarks/Outlines | Supported |
| Password-protected (user password) | Supported |
| Password-protected (owner password) | Supported |
| AES-256 encryption | Supported |
| Digital signatures | Metadata only |
| Form fields (AcroForms) | Planned |
| XFA forms | Planned |
| OCR (image-based PDFs) | Planned (feature flag) |

---

## Feature Flags

| Feature | Description | Default |
|---------|-------------|---------|
| `default` | Core PDF parsing and rendering | Yes |
| `ffi` | C-ABI foreign function interface | No |
| `async` | Async I/O with Tokio | No |
| `ocr` | OCR support via Tesseract | No |

```toml
# Cargo.toml - enable features
[dependencies]
unpdf = { version = "0.1", features = ["ffi", "async"] }
```

---

## Performance

- Parallel page processing with Rayon
- Efficient PDF parsing with lopdf
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
