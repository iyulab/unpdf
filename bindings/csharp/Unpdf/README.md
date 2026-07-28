# Unpdf

.NET bindings for [unpdf](https://github.com/iyulab/unpdf) - High-performance PDF content extraction to Markdown, text, and JSON.

## Installation

```bash
dotnet add package Unpdf
```

## Quick Start

The API is handle-based: parse once into an `UnpdfDocument`, query it, then dispose it.

```csharp
using Unpdf;

using var doc = UnpdfDocument.ParseFile("document.pdf");

// Markdown
Console.WriteLine(doc.ToMarkdown());

// Plain text
Console.WriteLine(doc.ToText());

// JSON
Console.WriteLine(doc.ToJson(compact: false));

// Metadata
Console.WriteLine($"Title: {doc.Title}");
Console.WriteLine($"Author: {doc.Author}");
Console.WriteLine($"Pages: {doc.SectionCount}");

// Native library version
Console.WriteLine(UnpdfDocument.Version);
```

Parse from memory instead of a path with `UnpdfDocument.ParseBytes(byte[])`.

## Advanced Usage

### Markdown options

```csharp
using var doc = UnpdfDocument.ParseFile("document.pdf");

var markdown = doc.ToMarkdown(new MarkdownOptions
{
    IncludeFrontmatter = true,
    EscapeSpecialChars = true,
    ParagraphSpacing = true,
});
```

### Per-page extraction

```csharp
using var doc = UnpdfDocument.ParseFile("document.pdf");

for (int page = 1; page <= doc.SectionCount; page++)
{
    Console.WriteLine(doc.PageToText(page));
}
```

### Detecting scanned (image-only) PDFs

Empty output can mean a scanned document, a genuinely blank page, or a parse
failure. The introspection surface tells them apart:

```csharp
using var doc = UnpdfDocument.ParseFile("scan.pdf");

// Document level
if (doc.GetExtractionQuality().IsScanPdf)
    Console.WriteLine("Scanned document - OCR required");

// Page level (works for mixed documents too)
var stats = doc.GetPageStats(1);
if (stats.TextOpCount == 0 && stats.ImageOpCount > 0)
    Console.WriteLine("Page 1 is image-only (scanned)");
else if (stats.TextOpCount == 0)
    Console.WriteLine("Page 1 is genuinely blank");
```

Note: a *searchable* scan (page image plus an invisible OCR text layer) reports
`TextOpCount > 0` — combine the check with `OcrTextSuppressed`, which flags pages
whose unreadable OCR layer was dropped.

### Handling failures

`UnpdfException.Kind` says why a call failed, so you can branch on the reason
instead of matching on `Message`:

```csharp
try
{
    using var doc = UnpdfDocument.ParseFile("document.pdf");
    Console.WriteLine(doc.ToMarkdown());
}
catch (UnpdfException e) when (e.Kind == UnpdfErrorKind.Encrypted)
{
    Console.WriteLine("Password required");
}
catch (UnpdfException e)
{
    Console.WriteLine($"Extraction failed ({e.Kind}): {e.Message}");
}
```

`UnpdfErrorKind` values are part of the native ABI: new reasons take new numbers and
existing ones are never renumbered, so treat an unrecognised value as a generic
failure. A failure raised by the managed wrapper rather than the native library
reports `UnpdfErrorKind.Other`.

### Embedded resources

Resource extraction is off by default in this binding's parse path (it bounds peak
memory on large PDFs), so `ResourceCount` is 0 and `GetResourceIds()` is empty for
documents parsed through `ParseFile` / `ParseBytes`. Use `GetPageStats` to detect
image-only pages rather than `ResourceCount`.

## API Reference

### `UnpdfDocument`

| Member | Description |
|--------|-------------|
| `static UnpdfDocument ParseFile(string path)` | Parse a PDF from disk. |
| `static UnpdfDocument ParseBytes(byte[] data)` | Parse a PDF from memory. |
| `static string Version` | Native library version. |
| `string ToMarkdown(MarkdownOptions? options = null)` | Render the whole document as Markdown. |
| `string ToText()` | Render the whole document as plain text. |
| `string ToJson(bool compact = false)` | Render the whole document as JSON. |
| `string PlainText()` | Text without the render pipeline — faster, simpler output. |
| `string PageToMarkdown(int pageNumber, MarkdownOptions? options = null)` | Render one page (1-indexed). |
| `string PageToText(int pageNumber)` | Text of one page (1-indexed). |
| `string? Title` / `string? Author` | Document metadata, or `null` when unset. |
| `int SectionCount` | Number of pages. |
| `int ResourceCount` | Size of the extracted-resource inventory — see above. |
| `ExtractionQuality GetExtractionQuality()` | Document-level extraction diagnostics. |
| `PageStats GetPageStats(int pageNumber)` | Per-page content-stream operator counts. |
| `string[] GetResourceIds()` | Ids in the resource inventory. |
| `JsonDocument? GetResourceInfo(string resourceId)` | Metadata for one resource. |
| `byte[]? GetResourceData(string resourceId)` | Raw bytes of one resource. |
| `void Dispose()` | Release the native handle. |

### `MarkdownOptions`

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `IncludeFrontmatter` | `bool` | `false` | Emit YAML frontmatter with document metadata |
| `EscapeSpecialChars` | `bool` | `false` | Escape Markdown special characters |
| `ParagraphSpacing` | `bool` | `false` | Add extra spacing between paragraphs |

### `ExtractionQuality`

`CharCount`, `WordCount`, `ReplacementCharCount`, `Encrypted`, `IsScanPdf`,
`SuppressedOcrPages`.

### `PageStats`

`Page`, `TextOpCount`, `ImageOpCount`, `OcrTextSuppressed`.

### `UnpdfException`

`Message` plus `Kind` (`UnpdfErrorKind`) — see "Handling failures".

## License

MIT License
