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

## API Reference

### `Pdf.ToMarkdown(string path)`
Convert a PDF file to Markdown format.

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

### `Pdf.Version`
Get the version of the native library.

## License

MIT License
