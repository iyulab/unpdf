using System.Text.Json.Serialization;

namespace Unpdf;

/// <summary>Which of a page's boxes a rendered image covers.</summary>
public enum PageRegion
{
    /// <summary>The crop box — what a viewer shows, and what a printed page holds.</summary>
    Crop,

    /// <summary>The whole media box, including what the crop box cuts away.</summary>
    Media,
}

/// <summary>Options for <see cref="UnpdfDocument.RenderPage"/>.</summary>
public sealed class RenderPageOptions
{
    /// <summary>Resolution in dots per inch; a page point is <c>Dpi / 72</c> pixels. Default 150.</summary>
    public float Dpi { get; init; } = 150f;

    /// <summary>The box painted. Default <see cref="PageRegion.Crop"/>.</summary>
    public PageRegion Region { get; init; } = PageRegion.Crop;
}

/// <summary>
/// What a rendered page could not show, by reason. All zero means everything the page's
/// content asks for was painted; otherwise the rest of the page was still painted.
/// </summary>
public sealed class RenderGaps
{
    /// <summary>
    /// Text runs not painted, or painted in part: the font is not embedded, or is Type 1 or
    /// Type 3, or a code selects no glyph in it.
    /// </summary>
    [JsonPropertyName("text_runs")]
    public uint TextRuns { get; init; }

    /// <summary>Images in a codec or color space the renderer does not decode.</summary>
    [JsonPropertyName("images")]
    public uint Images { get; init; }

    /// <summary>Inline images (<c>BI … EI</c>) not painted.</summary>
    [JsonPropertyName("inline_images")]
    public uint InlineImages { get; init; }

    /// <summary>Shadings and pattern colors not painted.</summary>
    [JsonPropertyName("shadings")]
    public uint Shadings { get; init; }

    /// <summary>Content streams of the page that could not be decoded.</summary>
    [JsonPropertyName("undecodable_content_streams")]
    public uint UndecodableContentStreams { get; init; }

    /// <summary>Whether anything the page asked for was left unpainted.</summary>
    [JsonIgnore]
    public bool IsEmpty =>
        TextRuns == 0 && Images == 0 && InlineImages == 0 && Shadings == 0
        && UndecodableContentStreams == 0;
}

/// <summary>A rendered page: a PNG, its size in pixels, and what it could not show.</summary>
public sealed class RenderedPage
{
    /// <summary>The page as a PNG (8-bit RGB), its <c>/Rotate</c> applied.</summary>
    public required byte[] Png { get; init; }

    /// <summary>Width in pixels.</summary>
    public int Width { get; init; }

    /// <summary>Height in pixels.</summary>
    public int Height { get; init; }

    /// <summary>What the renderer could not paint.</summary>
    public required RenderGaps Gaps { get; init; }
}

/// <summary>Wire shape of the native render report.</summary>
internal sealed class RenderInfoPayload
{
    [JsonPropertyName("width")]
    public int Width { get; init; }

    [JsonPropertyName("height")]
    public int Height { get; init; }

    [JsonPropertyName("gaps")]
    public RenderGaps Gaps { get; init; } = new();
}

/// <summary>Wire shape of <see cref="RenderPageOptions"/>.</summary>
internal sealed class RenderOptionsPayload
{
    [JsonPropertyName("dpi")]
    public float Dpi { get; init; }

    [JsonPropertyName("region")]
    public string Region { get; init; } = "crop";
}
