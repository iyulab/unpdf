using Xunit;

namespace Unpdf.Tests;

/// <summary>
/// <see cref="ParseOptions"/> — the C ABI surface for <c>ParseOptions</c>, previously
/// unreachable from this binding (docket #125). Omitting <see cref="ParseOptions"/> entirely
/// must behave exactly like before; passing one must actually reach the native parser.
/// </summary>
public class ParseOptionsTests
{
    [Fact]
    public void ParseBytes_NoOptions_MatchesPreviousBehavior()
    {
        using var doc = UnpdfDocument.ParseBytes(PdfFixtures.JpegPdf(100, 100));
        Assert.Equal(0, doc.ResourceCount);
    }

    [Fact]
    public void ParseBytes_OmitRunningHeads_ReachesTheNativeOptionsJson()
    {
        // Accepted by the native side (an unknown key would fail the parse); off by default.
        using var doc = UnpdfDocument.ParseBytes(
            PdfFixtures.JpegPdf(100, 100),
            new ParseOptions { OmitRunningHeads = true });

        Assert.Equal(1, doc.PageCount);
    }

    [Fact]
    public void ParseBytes_ExtractResources_PopulatesResourceInventory()
    {
        using var doc = UnpdfDocument.ParseBytes(
            PdfFixtures.JpegPdf(100, 100),
            new ParseOptions { ExtractResources = true });

        Assert.Equal(
            1,
            doc.ResourceCount);
    }

    /// <summary>
    /// Samples unpdf cannot convert (<see cref="PdfFixtures.UnconvertedColorImagePdf"/>) are
    /// a buffer most <c>GetResourceData</c> callers cannot use — they must never be surfaced,
    /// opt-in or not, regardless of <see cref="ParseOptions.MinImageDimension"/>.
    /// </summary>
    [Fact]
    public void ParseBytes_ExtractResources_NeverSurfacesRawUndecodedImages()
    {
        using var doc = UnpdfDocument.ParseBytes(
            PdfFixtures.UnconvertedColorImagePdf(),
            new ParseOptions { ExtractResources = true, MinImageDimension = 0 });

        Assert.Equal(0, doc.ResourceCount);
    }

    /// <summary>
    /// An image with no filter is samples like a <c>FlateDecode</c> one, and is re-encoded
    /// as PNG the same way.
    /// </summary>
    [Fact]
    public void ParseBytes_ExtractResources_SurfacesUnfilteredSamplesAsPng()
    {
        using var doc = UnpdfDocument.ParseBytes(
            PdfFixtures.ImageOnlyPdf(),
            new ParseOptions { ExtractResources = true, MinImageDimension = 0 });

        var id = Assert.Single(doc.GetResourceIds());
        using var info = doc.GetResourceInfo(id);
        Assert.Equal("image/png", info!.RootElement.GetProperty("mime_type").GetString());
    }

    [Fact]
    public void ParseBytes_MinImageDimension_DropsSmallImagesByDefault()
    {
        using var doc = UnpdfDocument.ParseBytes(
            PdfFixtures.JpegPdf(10, 10),
            new ParseOptions { ExtractResources = true });

        Assert.Equal(0, doc.ResourceCount);
    }

    [Fact]
    public void ParseBytes_MinImageDimensionZero_KeepsSmallImages()
    {
        using var doc = UnpdfDocument.ParseBytes(
            PdfFixtures.JpegPdf(10, 10),
            new ParseOptions { ExtractResources = true, MinImageDimension = 0 });

        Assert.Equal(1, doc.ResourceCount);
    }

    [Fact]
    public void ParseFile_ExtractResources_PopulatesResourceInventory()
    {
        var path = System.IO.Path.Combine(
            System.IO.Path.GetTempPath(),
            $"unpdf-csharp-parseoptions-test-{System.Environment.ProcessId}.pdf");
        System.IO.File.WriteAllBytes(path, PdfFixtures.JpegPdf(100, 100));
        try
        {
            using var doc = UnpdfDocument.ParseFile(
                path, new ParseOptions { ExtractResources = true });
            Assert.Equal(1, doc.ResourceCount);
        }
        finally
        {
            System.IO.File.Delete(path);
        }
    }

    /// <summary>
    /// Every field the C ABI's parse options take is reachable from here. <see cref="ParseOptions.Pages"/>
    /// selects what is parsed: a page outside the document is refused, not ignored.
    /// </summary>
    [Fact]
    public void Pages_SelectsWhatIsParsed()
    {
        using var all = UnpdfDocument.ParseBytes(PdfFixtures.TextPdf());
        using var first = UnpdfDocument.ParseBytes(
            PdfFixtures.TextPdf(), new ParseOptions { Pages = [1] });
        Assert.Equal(all.SectionCount, first.SectionCount);

        var ex = Assert.Throws<UnpdfException>(() => UnpdfDocument.ParseBytes(
            PdfFixtures.TextPdf(), new ParseOptions { Pages = [99] }));
        Assert.Equal(UnpdfErrorKind.PageOutOfRange, ex.Kind);
    }

    /// <summary>
    /// The AI settings reach the native parser under the names it reads, and an incomplete
    /// configuration is refused there -- whether or not the library was built with the
    /// <c>ai</c> feature, it names the setting rather than ignoring it.
    /// </summary>
    [Fact]
    public void AiSettings_ReachTheNativeParser()
    {
        var json = new ParseOptions
        {
            AiBaseUrl = "http://localhost:1/v1",
            AiApiKey = "key",
            AiModel = "model",
            AiImageScope = AiImageScope.LowConfidencePagesOnly,
        }.ToJson();
        Assert.Equal(
            """{"ai_base_url":"http://localhost:1/v1","ai_api_key":"key","ai_model":"model","ai_image_scope":"low_confidence_pages_only"}""",
            json);

        var ex = Assert.Throws<UnpdfException>(() => UnpdfDocument.ParseBytes(
            PdfFixtures.TextPdf(), new ParseOptions { AiModel = "model" }));
        Assert.Equal(UnpdfErrorKind.InvalidArgument, ex.Kind);
        Assert.Contains("ai_", ex.Message);
    }
}
