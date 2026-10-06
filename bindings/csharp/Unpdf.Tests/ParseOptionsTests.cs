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
    /// Samples unpdf cannot convert (<see cref="PdfFixtures.LabImagePdf"/>) are a buffer
    /// most <c>GetResourceData</c> callers cannot use — they must never be surfaced, opt-in
    /// or not, regardless of <see cref="ParseOptions.MinImageDimension"/>.
    /// </summary>
    [Fact]
    public void ParseBytes_ExtractResources_NeverSurfacesRawUndecodedImages()
    {
        using var doc = UnpdfDocument.ParseBytes(
            PdfFixtures.LabImagePdf(),
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
}
