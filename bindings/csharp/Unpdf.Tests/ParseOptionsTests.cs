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
    /// The undecoded pixel buffer <see cref="PdfFixtures.ImageOnlyPdf"/> produces is a format
    /// most <c>GetResourceData</c> callers cannot use — it must never be surfaced, opt-in or
    /// not, regardless of <see cref="ParseOptions.MinImageDimension"/>.
    /// </summary>
    [Fact]
    public void ParseBytes_ExtractResources_NeverSurfacesRawUndecodedImages()
    {
        using var doc = UnpdfDocument.ParseBytes(
            PdfFixtures.ImageOnlyPdf(),
            new ParseOptions { ExtractResources = true, MinImageDimension = 0 });

        Assert.Equal(0, doc.ResourceCount);
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
