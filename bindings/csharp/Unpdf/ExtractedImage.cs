using System.Text.Json.Serialization;

namespace Unpdf;

/// <summary>
/// Information about an extracted image from a PDF.
/// </summary>
public class ExtractedImage
{
    /// <summary>
    /// Unique identifier for the image within the PDF.
    /// </summary>
    [JsonPropertyName("id")]
    public string Id { get; set; } = string.Empty;

    /// <summary>
    /// Filename of the extracted image.
    /// </summary>
    [JsonPropertyName("filename")]
    public string Filename { get; set; } = string.Empty;

    /// <summary>
    /// Full path where the image was saved.
    /// </summary>
    [JsonPropertyName("path")]
    public string Path { get; set; } = string.Empty;

    /// <summary>
    /// MIME type of the image (e.g., "image/jpeg", "image/png").
    /// </summary>
    [JsonPropertyName("mime_type")]
    public string MimeType { get; set; } = string.Empty;

    /// <summary>
    /// Width of the image in pixels (if available).
    /// </summary>
    [JsonPropertyName("width")]
    public int? Width { get; set; }

    /// <summary>
    /// Height of the image in pixels (if available).
    /// </summary>
    [JsonPropertyName("height")]
    public int? Height { get; set; }

    /// <summary>
    /// Size of the image data in bytes.
    /// </summary>
    [JsonPropertyName("size_bytes")]
    public long SizeBytes { get; set; }
}
