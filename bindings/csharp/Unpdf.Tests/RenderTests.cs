using Xunit;

namespace Unpdf.Tests;

/// <summary>
/// <see cref="UnpdfDocument.RenderPage"/>: a page of the parsed document as a PNG.
/// </summary>
public class RenderTests
{
    private static uint BigEndian(byte[] data, int at) =>
        (uint)(data[at] << 24 | data[at + 1] << 16 | data[at + 2] << 8 | data[at + 3]);

    [Fact]
    public void RenderPage_ImageOnlyPdf_IsAPngOfThePage()
    {
        using var doc = UnpdfDocument.ParseBytes(PdfFixtures.ImageOnlyPdf());
        var page = doc.RenderPage(1, new RenderPageOptions { Dpi = 72 });

        Assert.Equal(new byte[] { 0x89, (byte)'P', (byte)'N', (byte)'G' }, page.Png[..4]);
        // An A4 page: 595 x 842 points, one pixel each at 72 dpi.
        Assert.Equal(595, page.Width);
        Assert.Equal(842, page.Height);
        Assert.Equal(595u, BigEndian(page.Png, 16));
        Assert.Equal(842u, BigEndian(page.Png, 20));
        Assert.True(page.Gaps.IsEmpty);
    }

    [Fact]
    public void RenderPage_AStandardFontThatIsNotEmbedded_IsDrawnInAStandInFace()
    {
        using var doc = UnpdfDocument.ParseBytes(PdfFixtures.TextPdf());
        var page = doc.RenderPage(1);
        Assert.Equal(1240, page.Width); // 150 dpi by default
        Assert.Equal(0u, page.Gaps.TextRuns);
        Assert.True(page.SubstitutedTextRuns >= 1);
    }

    [Fact]
    public void RenderPage_PageOutOfRange_ThrowsWithItsKind()
    {
        using var doc = UnpdfDocument.ParseBytes(PdfFixtures.ImageOnlyPdf());
        var ex = Assert.Throws<UnpdfException>(() => doc.RenderPage(2));
        Assert.Equal(UnpdfErrorKind.PageOutOfRange, ex.Kind);
    }
}
