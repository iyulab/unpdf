using System.Text.Json.Serialization;

namespace Unpdf;

/// <summary>
/// Extraction quality diagnostics for a parsed document.
/// </summary>
/// <remarks>
/// Lets consumers tell <em>why</em> extraction produced little or no text:
/// <see cref="IsScanPdf"/> flags an image-only (scanned) document that needs OCR,
/// while <see cref="CharCount"/> == 0 without it points at a genuinely empty or
/// unsupported document. For page-level discrimination in mixed documents use
/// <see cref="UnpdfDocument.GetPageStats"/>.
/// </remarks>
public sealed class ExtractionQuality
{
    /// <summary>Total number of characters in the extracted text.</summary>
    [JsonPropertyName("char_count")]
    public long CharCount { get; init; }

    /// <summary>Total number of whitespace-delimited words.</summary>
    [JsonPropertyName("word_count")]
    public long WordCount { get; init; }

    /// <summary>Number of U+FFFD replacement characters (decoding failures).</summary>
    [JsonPropertyName("replacement_char_count")]
    public long ReplacementCharCount { get; init; }

    /// <summary>Whether the source PDF was encrypted.</summary>
    [JsonPropertyName("encrypted")]
    public bool Encrypted { get; init; }

    /// <summary>
    /// Whether the PDF appears to be a scanned image (no text layer).
    /// Detected by sampling content-stream operators across the first few pages:
    /// image draws present with no text-showing operators.
    /// </summary>
    [JsonPropertyName("is_scan_pdf")]
    public bool IsScanPdf { get; init; }

    /// <summary>Number of pages whose unreadable OCR text layer was dropped.</summary>
    [JsonPropertyName("suppressed_ocr_pages")]
    public long SuppressedOcrPages { get; init; }

    /// <summary>
    /// Whether pages are known to be missing from the output.
    /// <para>
    /// <c>true</c> means the parser recovered what it could from a damaged document and
    /// some pages never made it — extraction "succeeded" over an incomplete page set.
    /// Check this before indexing or archiving: a page that silently never arrived is
    /// indistinguishable from a page that never existed.
    /// </para>
    /// Always <c>false</c> for intact documents, and unaffected by page-range selection.
    /// </summary>
    [JsonPropertyName("pages_incomplete")]
    public bool PagesIncomplete { get; init; }

    /// <summary>
    /// Page count the document declares (root <c>Pages</c> <c>/Count</c>), or <c>null</c>
    /// when that declaration was itself unreadable — which is a damage signal in its own
    /// right, reported via <see cref="UnresolvedPageNodes"/>.
    /// </summary>
    [JsonPropertyName("declared_page_count")]
    public long? DeclaredPageCount { get; init; }

    /// <summary>
    /// Page-tree nodes that could not be read, so their pages never reached the output.
    /// <para>
    /// Any non-zero value means the page set is incomplete. It is deliberately not a
    /// count of lost pages: one unusable intermediate node drops its whole subtree, so a
    /// single unresolved node can cost one page or a hundred. Do not surface it to users
    /// as "N pages lost".
    /// </para>
    /// </summary>
    [JsonPropertyName("unresolved_page_nodes")]
    public long UnresolvedPageNodes { get; init; }

    /// <summary>
    /// Objects the cross-reference table pointed at that could not be loaded. A damage
    /// indicator only — most skipped objects (fonts, annotations, metadata) cost no page,
    /// so this does not imply missing text. <see cref="PagesIncomplete"/> is the signal
    /// for lost content.
    /// </summary>
    [JsonPropertyName("skipped_object_count")]
    public long SkippedObjectCount { get; init; }
}

/// <summary>
/// Per-page content-stream operator statistics.
/// </summary>
/// <remarks>
/// <see cref="TextOpCount"/> == 0 with <see cref="ImageOpCount"/> &gt; 0 identifies an
/// image-only (scanned) page — OCR required. Both 0 means a genuinely blank page.
/// <para>
/// A <em>searchable</em> scan (page image plus an invisible OCR text layer) reports
/// <see cref="TextOpCount"/> &gt; 0 — combine the check with <see cref="OcrTextSuppressed"/>,
/// which flags pages whose unreadable OCR layer was dropped.
/// </para>
/// </remarks>
public sealed class PageStats
{
    /// <summary>Page number (1-indexed).</summary>
    [JsonPropertyName("page")]
    public int Page { get; init; }

    /// <summary>Number of text-showing operators (Tj/TJ/'/") on the page.</summary>
    [JsonPropertyName("text_op_count")]
    public uint TextOpCount { get; init; }

    /// <summary>
    /// Number of XObject Do invocations on the page — mostly images, but form
    /// XObjects may be included.
    /// </summary>
    [JsonPropertyName("image_op_count")]
    public uint ImageOpCount { get; init; }

    /// <summary>Whether this page's unreadable OCR text layer was dropped.</summary>
    [JsonPropertyName("ocr_text_suppressed")]
    public bool OcrTextSuppressed { get; init; }
}
