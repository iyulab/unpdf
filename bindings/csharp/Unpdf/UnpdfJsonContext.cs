using System.Text.Json.Serialization;

namespace Unpdf;

/// <summary>
/// Wire shape of <see cref="ParseOptions"/> as the native library reads it. Unset options are
/// omitted so the native defaults apply.
/// </summary>
internal sealed class ParseOptionsPayload
{
    [JsonPropertyName("error_mode")]
    public string? ErrorMode { get; init; }

    [JsonPropertyName("extract_text")]
    public bool? ExtractText { get; init; }

    [JsonPropertyName("extract_resources")]
    public bool? ExtractResources { get; init; }

    [JsonPropertyName("min_image_dimension")]
    public uint? MinImageDimension { get; init; }

    [JsonPropertyName("parallel")]
    public bool? Parallel { get; init; }

    [JsonPropertyName("password")]
    public string? Password { get; init; }

    [JsonPropertyName("suppress_low_confidence_ocr")]
    public bool? SuppressLowConfidenceOcr { get; init; }

    [JsonPropertyName("omit_running_heads")]
    public bool? OmitRunningHeads { get; init; }

    [JsonPropertyName("pages")]
    public PageListPayload? Pages { get; init; }

    [JsonPropertyName("ai_base_url")]
    public string? AiBaseUrl { get; init; }

    [JsonPropertyName("ai_api_key")]
    public string? AiApiKey { get; init; }

    [JsonPropertyName("ai_model")]
    public string? AiModel { get; init; }

    [JsonPropertyName("ai_image_scope")]
    public string? AiImageScope { get; init; }
}

/// <summary>
/// Wire shape of <see cref="MarkdownOptions"/> as <c>unpdf_to_markdown_with_options</c> reads it.
/// </summary>
internal sealed class MarkdownOptionsPayload
{
    [JsonPropertyName("image_path_prefix")]
    public string ImagePathPrefix { get; init; } = "";

    [JsonPropertyName("table_fallback")]
    public string TableFallback { get; init; } = "markdown";

    [JsonPropertyName("max_heading_level")]
    public int MaxHeadingLevel { get; init; }

    [JsonPropertyName("include_frontmatter")]
    public bool IncludeFrontmatter { get; init; }

    [JsonPropertyName("preserve_line_breaks")]
    public bool PreserveLineBreaks { get; init; }

    [JsonPropertyName("list_marker")]
    public string ListMarker { get; init; } = "-";

    [JsonPropertyName("escape_special_chars")]
    public bool EscapeSpecialChars { get; init; }

    [JsonPropertyName("cleanup_preset")]
    public string CleanupPreset { get; init; } = "standard";

    [JsonPropertyName("refine")]
    public bool Refine { get; init; }

    [JsonPropertyName("line_width")]
    public uint LineWidth { get; init; }

    [JsonPropertyName("page_markers")]
    public string PageMarkers { get; init; } = "none";

    [JsonPropertyName("pages")]
    public PageListPayload? Pages { get; init; }
}

/// <summary>
/// <c>{"pages": [n, ...]}</c> — the page-list form of the <c>pages</c> parse and render options.
/// </summary>
internal sealed class PageListPayload
{
    [JsonPropertyName("pages")]
    public int[] Pages { get; init; } = [];
}

/// <summary>
/// Source-generated serialization metadata for every type the binding exchanges with the
/// native library as JSON. Using it instead of reflection keeps the binding working in
/// trimmed, Native AOT, and other apps that disable reflection-based serialization.
/// </summary>
[JsonSourceGenerationOptions(DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull)]
[JsonSerializable(typeof(ParseOptionsPayload))]
[JsonSerializable(typeof(MarkdownOptionsPayload))]
[JsonSerializable(typeof(ExtractionQuality))]
[JsonSerializable(typeof(PageStats))]
[JsonSerializable(typeof(TableText[]))]
[JsonSerializable(typeof(PageSize))]
[JsonSerializable(typeof(RenderInfoPayload))]
[JsonSerializable(typeof(RenderOptionsPayload))]
[JsonSerializable(typeof(string[]))]
internal sealed partial class UnpdfJsonContext : JsonSerializerContext
{
}
