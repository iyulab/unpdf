# Unpdf

.NET bindings for [unpdf](https://github.com/iyulab/unpdf) - High-performance PDF content extraction to Markdown, text, and JSON.

## Installation

```bash
dotnet add package Unpdf
```

## Quick Start

```csharp
using Unpdf;

// Convert PDF to Markdown
string markdown = Pdf.ToMarkdown("document.pdf");
Console.WriteLine(markdown);

// Convert PDF to plain text
string text = Pdf.ToText("document.pdf");
Console.WriteLine(text);

// Convert PDF to JSON
string json = Pdf.ToJson("document.pdf", pretty: true);
Console.WriteLine(json);

// Get document information
var info = Pdf.GetInfo("document.pdf");
Console.WriteLine($"Title: {info.Title}");
Console.WriteLine($"Pages: {info.PageCount}");

// Get page count
int pages = Pdf.GetPageCount("document.pdf");
Console.WriteLine($"Total pages: {pages}");

// Check if file is a valid PDF
bool isValid = Pdf.IsPdf("document.pdf");
Console.WriteLine($"Is valid PDF: {isValid}");
```

## Advanced Usage

### Convert with Options

```csharp
using Unpdf;

// Convert with frontmatter and image extraction
var options = new PdfOptions
{
    IncludeFrontmatter = true,
    ExtractImages = true,
    ImageOutputDir = "./images",
    Lenient = true
};

string markdown = Pdf.ToMarkdown("document.pdf", options);
Console.WriteLine(markdown);
```

### Extract Images

```csharp
using Unpdf;

// Extract all images from PDF
var images = Pdf.ExtractImages("document.pdf", "./output/images");

foreach (var image in images)
{
    Console.WriteLine($"Image: {image.Filename}");
    Console.WriteLine($"  Path: {image.Path}");
    Console.WriteLine($"  Type: {image.MimeType}");
    Console.WriteLine($"  Size: {image.Width}x{image.Height}");
    Console.WriteLine($"  Bytes: {image.SizeBytes}");
}
```

## API Reference

### `Pdf.ToMarkdown(string path)`
Convert a PDF file to Markdown format.

### `Pdf.ToMarkdown(string path, PdfOptions options)`
Convert a PDF file to Markdown format with options.

### `Pdf.ToText(string path)`
Convert a PDF file to plain text.

### `Pdf.ToJson(string path, bool pretty = false)`
Convert a PDF file to JSON format.

### `Pdf.GetInfo(string path)`
Get document metadata (title, author, page count, etc.)

### `Pdf.GetPageCount(string path)`
Get the number of pages in a PDF file.

### `Pdf.IsPdf(string path)`
Check if a file is a valid PDF.

### `Pdf.ExtractImages(string path, string outputDir)`
Extract all images from a PDF file to the specified directory.

### `Pdf.Version`
Get the version of the native library.

## PdfOptions

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `ExtractImages` | `bool` | `false` | Enable image extraction during conversion |
| `ImageOutputDir` | `string?` | `null` | Directory to save extracted images |
| `IncludeFrontmatter` | `bool` | `false` | Include YAML frontmatter with metadata |
| `Lenient` | `bool` | `true` | Continue parsing despite minor errors |

## License

MIT License
