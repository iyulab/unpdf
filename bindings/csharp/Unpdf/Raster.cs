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

/// <summary>
/// A page's size as displayed, in points (1/72 inch): the region a render covers, the page's
/// <c>/Rotate</c> applied. <see cref="Pixels"/> gives the pixels a render at a dpi has.
/// </summary>
public sealed class PageSize
{
    /// <summary>Width in points.</summary>
    [JsonPropertyName("width")]
    public float Width { get; init; }

    /// <summary>Height in points.</summary>
    [JsonPropertyName("height")]
    public float Height { get; init; }

    /// <summary>
    /// The pixels <see cref="UnpdfDocument.RenderPage"/> paints this size in at
    /// <paramref name="dpi"/>, by the native arithmetic painting sizes its bitmap with: each
    /// side <c>points * (dpi / 72)</c> in single precision, rounded half away from zero, at
    /// least 1. That can be a pixel off exact arithmetic (396 points at 93 dpi paint 511
    /// pixels), so <c>72 * edge / points</c> is only a candidate dpi for a side of
    /// <c>edge</c> pixels, to check here before rendering.
    /// </summary>
    public (uint Width, uint Height) Pixels(float dpi)
    {
        _ = NativeMethods.unpdf_page_pixels(Width, Height, dpi, out var width, out var height);
        return (width, height);
    }
}

/// <summary>Options for <see cref="UnpdfDocument.RenderPage"/>.</summary>
public sealed class RenderPageOptions
{
    /// <summary>Resolution in dots per inch; a page point is <c>Dpi / 72</c> pixels. Default 150.</summary>
    public float Dpi { get; init; } = 150f;

    /// <summary>The box painted. Default <see cref="PageRegion.Crop"/>.</summary>
    public PageRegion Region { get; init; } = PageRegion.Crop;

    /// <summary>
    /// The most work painting the page may take, in the pixels each step of painting is
    /// charged: the page's own pixels, then what each fill, stroke, glyph, image, clip and
    /// layer covers. A page that would take more fails with
    /// <see cref="UnpdfErrorKind.RasterStopped"/>. <c>null</c> (the default) is unbounded.
    /// </summary>
    public ulong? MaxWork { get; init; }
}

/// <summary>
/// What a rendered page could not show, by reason. All zero means everything the page's
/// content asks for was painted; otherwise the rest of the page was still painted.
/// </summary>
public sealed class RenderGaps
{
    /// <summary>
    /// Text runs not painted, or painted in part: the font is not embedded and no standard
    /// face stands in for it, a code selects no glyph in it or one with no outline whose text
    /// does not decode or is more than whitespace and default-ignorable characters, a Type 3
    /// glyph is nested more than four deep, or the text is shown before any font is set.
    /// </summary>
    [JsonPropertyName("text_runs")]
    public uint TextRuns { get; init; }

    /// <summary>Images in a codec or color space the renderer does not decode.</summary>
    [JsonPropertyName("images")]
    public uint Images { get; init; }

    /// <summary>Inline images (<c>BI … EI</c>) whose dictionary does not read; one in a codec or
    /// color space the renderer does not decode is counted in <see cref="Images"/>.</summary>
    [JsonPropertyName("inline_images")]
    public uint InlineImages { get; init; }

    /// <summary>Shadings and pattern colors not painted: mesh and function-based shadings, and a
    /// tiling pattern nested more than three deep. Axial and radial shadings and tiling patterns
    /// are painted. Text in such a pattern, or in a tiling pattern whose cell was not painted in
    /// full, is painted in black and counted here.</summary>
    [JsonPropertyName("shadings")]
    public uint Shadings { get; init; }

    /// <summary>Soft masks not applied as set: the mask group did not decode or is nested more
    /// than three deep (what follows is painted unmasked), or the mask names a transfer function
    /// the renderer does not apply (the mask is applied without it).</summary>
    [JsonPropertyName("soft_masks")]
    public uint SoftMasks { get; init; }

    /// <summary>Content streams of the page that could not be decoded, and Form XObjects nested
    /// more than 32 deep, which are not painted.</summary>
    [JsonPropertyName("undecodable_content_streams")]
    public uint UndecodableContentStreams { get; init; }

    /// <summary>Whether anything the page asked for was left unpainted.</summary>
    [JsonIgnore]
    public bool IsEmpty =>
        TextRuns == 0 && Images == 0 && InlineImages == 0 && Shadings == 0
        && SoftMasks == 0 && UndecodableContentStreams == 0;
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

    /// <summary>
    /// Text runs painted in a standard face standing in for a font the PDF does not embed:
    /// readable, but not the page's own typeface. Not a gap — the text is there.
    /// </summary>
    public uint SubstitutedTextRuns { get; init; }
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

    [JsonPropertyName("substituted_text_runs")]
    public uint SubstitutedTextRuns { get; init; }
}

/// <summary>Wire shape of <see cref="RenderPageOptions"/>.</summary>
internal sealed class RenderOptionsPayload
{
    [JsonPropertyName("dpi")]
    public float Dpi { get; init; }

    [JsonPropertyName("region")]
    public string Region { get; init; } = "crop";

    [JsonPropertyName("max_work")]
    public ulong? MaxWork { get; init; }
}
