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
    /// Number of text runs the font decoder could not read and discarded.
    /// <para>
    /// A run is one text string handed to the decoder — a <c>Tj</c> operand, or a
    /// single element of a <c>TJ</c> array. The decoder drops a run when the font's
    /// character codes cannot be resolved — emitting the raw bytes would produce
    /// mojibake rather than text. The dropped runs are content the document had and
    /// the output does not, so any non-zero value means the extraction is incomplete
    /// in a way no other property shows.
    /// </para>
    /// Counted in runs, not characters: the discarded text was never decoded, so its
    /// length is unknowable. Treat non-zero as "incomplete"; the magnitude only
    /// compares documents to each other.
    /// </summary>
    [JsonPropertyName("suppressed_text_runs")]
    public long SuppressedTextRuns { get; init; }

    /// <summary>
    /// Page content streams that could not be decoded.
    /// <para>
    /// Lenient parsing (the default) leaves such a stream out and keeps the rest of the
    /// page — an empty page when it was the page's only stream, which otherwise reads
    /// exactly like a blank page. Any non-zero value means content is missing from an
    /// otherwise successful extraction. Strict parsing fails instead.
    /// </para>
    /// </summary>
    [JsonPropertyName("undecodable_content_streams")]
    public long UndecodableContentStreams { get; init; }

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

    /// <summary>
    /// Embedded images recognized as image XObjects but not extractable in the current
    /// output format (an unsupported color space or bit depth), so they were dropped.
    /// <para>
    /// Distinguishes "this page has no image" from "this page has an image we couldn't
    /// materialize" — a resource inventory with zero entries looks the same either way
    /// unless this is checked.
    /// </para>
    /// </summary>
    [JsonPropertyName("unsupported_image_count")]
    public long UnsupportedImageCount { get; init; }

    /// <summary>
    /// Times VLM image understanding fell back to the non-AI result — a transport
    /// failure, a non-success status, a truncated or malformed response, or retries
    /// exhausted. The cause is not distinguished.
    /// <para>
    /// AI parse options must be configured for this to ever be non-zero, and extraction
    /// always succeeds regardless: a fallback lowers this count, it never turns the
    /// result into an error. Non-zero therefore means "the output is the non-AI result
    /// for N images", which is a quality signal rather than a failure.
    /// </para>
    /// The render-side AI refine pass is not counted here — it runs after this record is
    /// final and reports its own fallbacks through a warning log instead.
    /// </summary>
    [JsonPropertyName("ai_fallback_count")]
    public long AiFallbackCount { get; init; }
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

    /// <summary>The page's <c>/Rotate</c>: 0, 90, 180 or 270 degrees clockwise.</summary>
    [JsonPropertyName("rotation")]
    public int Rotation { get; init; }

    /// <summary>Number of text-showing operators (Tj/TJ/'/") on the page.</summary>
    [JsonPropertyName("text_op_count")]
    public uint TextOpCount { get; init; }

    /// <summary>
    /// Number of image paints on the page (<c>Do</c> of anything but a Form XObject, and
    /// inline images), including those inside the forms the page paints.
    /// </summary>
    [JsonPropertyName("image_op_count")]
    public uint ImageOpCount { get; init; }

    /// <summary>
    /// Number of Form XObject paints on the page. A form's content is part of the page —
    /// its text is in <see cref="TextOpCount"/>, its images in <see cref="ImageOpCount"/> —
    /// so this says how the page was assembled, not what it shows.
    /// </summary>
    [JsonPropertyName("form_op_count")]
    public uint FormOpCount { get; init; }

    /// <summary>Whether this page's unreadable OCR text layer was dropped.</summary>
    [JsonPropertyName("ocr_text_suppressed")]
    public bool OcrTextSuppressed { get; init; }

    /// <summary>
    /// Number of text runs the font decoder could not read and discarded on this page.
    /// Same signal as <see cref="ExtractionQuality.SuppressedTextRuns"/>, broken down
    /// per page — the document-level total is the sum of this field across all pages.
    /// </summary>
    [JsonPropertyName("suppressed_text_runs")]
    public long SuppressedTextRuns { get; init; }

    /// <summary>
    /// The fonts behind <see cref="SuppressedTextRuns"/>: one entry per font and reason, in
    /// the order each first lost a run on this page. Empty when nothing was discarded. The
    /// <see cref="UnreadableFont.Runs"/> of all entries sum to <see cref="SuppressedTextRuns"/>.
    /// </summary>
    [JsonPropertyName("unreadable_fonts")]
    public IReadOnlyList<UnreadableFont> UnreadableFonts { get; init; } = System.Array.Empty<UnreadableFont>();

    /// <summary>
    /// Content streams of this page that could not be decoded. This page's share of
    /// <see cref="ExtractionQuality.UndecodableContentStreams"/>.
    /// </summary>
    [JsonPropertyName("undecodable_content_streams")]
    public long UndecodableContentStreams { get; init; }

    /// <summary>
    /// Share of the page painted by images, 0 to 1: the union of the image paints'
    /// rectangles, clipped to what the page shows (its crop box and clipping paths). Tells a full-page scan (near 1) from a logo (a few
    /// hundredths) when both report one image paint.
    /// </summary>
    [JsonPropertyName("image_coverage")]
    public double ImageCoverage { get; init; }

    /// <summary>
    /// Text runs not set horizontally left to right (rotated, vertical or upside-down). The
    /// reading order treats them as horizontal, so their order may be wrong.
    /// </summary>
    [JsonPropertyName("rotated_text_runs")]
    public uint RotatedTextRuns { get; init; }

    /// <summary>Ruling-line grids drawn on the page.</summary>
    [JsonPropertyName("ruled_grids")]
    public uint RuledGrids { get; init; }

    /// <summary>
    /// Tables built from the page's ruling-line grids. Fewer than <see cref="RuledGrids"/>
    /// means a drawn grid produced no table.
    /// </summary>
    [JsonPropertyName("ruled_tables")]
    public uint RuledTables { get; init; }

    /// <summary>Text regions the reading order read one after another.</summary>
    [JsonPropertyName("reading_regions")]
    public uint ReadingRegions { get; init; }

    /// <summary>
    /// The most text regions set side by side at any height: 1 for a single column, 2 for
    /// two columns, 0 for a page with no text.
    /// </summary>
    [JsonPropertyName("column_count")]
    public uint ColumnCount { get; init; }

    /// <summary>
    /// Regions read line by line across although their text looked like two columns —
    /// where the reading order had to guess.
    /// </summary>
    [JsonPropertyName("ambiguous_layout_regions")]
    public uint AmbiguousLayoutRegions { get; init; }
}

/// <summary>
/// A font whose text runs were discarded on a page because its character codes could not
/// be turned into text. Lets a consumer say which font could not be read and why.
/// </summary>
public sealed class UnreadableFont
{
    /// <summary>
    /// The font's <c>/BaseFont</c> as written (a subset prefix such as <c>ABCDEF+</c> is
    /// kept), or its resource name in the page's <c>/Font</c> when it has no <c>/BaseFont</c>.
    /// </summary>
    [JsonPropertyName("name")]
    public string Name { get; init; } = "";

    /// <summary>
    /// Why the font's runs were discarded. Known values: <c>composite_unresolved</c> (a
    /// Type0/CID font whose codes map to no text) and <c>binary_density</c> (a simple font
    /// whose decode was binary noise). More values may be added: treat an unknown one as
    /// a generic "unreadable".
    /// </summary>
    [JsonPropertyName("reason")]
    public string Reason { get; init; } = "";

    /// <summary>Text runs of this font discarded for this reason on the page.</summary>
    [JsonPropertyName("runs")]
    public long Runs { get; init; }
}

/// <summary>
/// One table of the document as delimited text — see <see cref="UnpdfDocument.GetTables"/>.
/// </summary>
public sealed class TableText
{
    /// <summary>The number of the page the table is on (1-indexed).</summary>
    [JsonPropertyName("page")]
    public int Page { get; init; }

    /// <summary>The table's place among that page's tables (from 1).</summary>
    [JsonPropertyName("index")]
    public int Index { get; init; }

    /// <summary>
    /// The table as CSV (RFC 4180), or tab-separated text: a merged cell's text in its
    /// top-left position and the positions it covers empty, records ended with CRLF.
    /// </summary>
    [JsonPropertyName("text")]
    public string Text { get; init; } = "";
}
