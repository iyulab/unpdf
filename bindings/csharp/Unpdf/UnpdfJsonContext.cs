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
}

/// <summary>
/// Source-generated serialization metadata for every type the binding exchanges with the
/// native library as JSON. Using it instead of reflection keeps the binding working in
/// trimmed, Native AOT, and other apps that disable reflection-based serialization.
/// </summary>
[JsonSourceGenerationOptions(DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull)]
[JsonSerializable(typeof(ParseOptionsPayload))]
[JsonSerializable(typeof(ExtractionQuality))]
[JsonSerializable(typeof(PageStats))]
[JsonSerializable(typeof(string[]))]
internal sealed partial class UnpdfJsonContext : JsonSerializerContext
{
}
