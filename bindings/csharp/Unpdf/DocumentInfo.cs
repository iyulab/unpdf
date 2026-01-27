using System.Text.Json.Serialization;

namespace Unpdf;

/// <summary>
/// Document metadata information.
/// </summary>
public class DocumentInfo
{
    /// <summary>
    /// Document title.
    /// </summary>
    [JsonPropertyName("title")]
    public string? Title { get; set; }

    /// <summary>
    /// Document author.
    /// </summary>
    [JsonPropertyName("author")]
    public string? Author { get; set; }

    /// <summary>
    /// Document subject.
    /// </summary>
    [JsonPropertyName("subject")]
    public string? Subject { get; set; }

    /// <summary>
    /// Document keywords.
    /// </summary>
    [JsonPropertyName("keywords")]
    public string? Keywords { get; set; }

    /// <summary>
    /// Application that created the document.
    /// </summary>
    [JsonPropertyName("creator")]
    public string? Creator { get; set; }

    /// <summary>
    /// PDF producer application.
    /// </summary>
    [JsonPropertyName("producer")]
    public string? Producer { get; set; }

    /// <summary>
    /// Document creation date.
    /// </summary>
    [JsonPropertyName("created")]
    public string? Created { get; set; }

    /// <summary>
    /// Document modification date.
    /// </summary>
    [JsonPropertyName("modified")]
    public string? Modified { get; set; }

    /// <summary>
    /// PDF version.
    /// </summary>
    [JsonPropertyName("pdf_version")]
    public string? PdfVersion { get; set; }

    /// <summary>
    /// Number of pages in the document.
    /// </summary>
    [JsonPropertyName("page_count")]
    public int PageCount { get; set; }

    /// <summary>
    /// Whether the document is encrypted.
    /// </summary>
    [JsonPropertyName("encrypted")]
    public bool Encrypted { get; set; }
}
