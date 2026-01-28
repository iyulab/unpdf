namespace Unpdf;

/// <summary>
/// Options for PDF conversion operations.
/// </summary>
public class PdfOptions
{
    /// <summary>
    /// Enable image extraction during conversion.
    /// </summary>
    public bool ExtractImages { get; set; }

    /// <summary>
    /// Directory to save extracted images. If null, images are not saved to disk.
    /// </summary>
    public string? ImageOutputDir { get; set; }

    /// <summary>
    /// Include YAML frontmatter with document metadata in output.
    /// </summary>
    public bool IncludeFrontmatter { get; set; }

    /// <summary>
    /// Enable lenient parsing mode (default: true).
    /// When enabled, parsing continues despite minor errors.
    /// </summary>
    public bool Lenient { get; set; } = true;
}
