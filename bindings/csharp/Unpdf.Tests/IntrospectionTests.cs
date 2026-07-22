using System.Text;
using Xunit;

namespace Unpdf.Tests;

/// <summary>
/// Extraction-quality / per-page stats introspection surface.
/// 소비자(FileFlux 등)가 "빈 텍스트"의 원인 — 스캔본(no text layer)인지
/// 진짜 빈 페이지인지 — 를 구분할 수 있어야 한다.
/// </summary>
public class IntrospectionTests
{
    [Fact]
    public void GetExtractionQuality_ImageOnlyPdf_ReportsScanPdf()
    {
        using var doc = UnpdfDocument.ParseBytes(PdfFixtures.ImageOnlyPdf());
        var quality = doc.GetExtractionQuality();
        Assert.True(quality.IsScanPdf);
        Assert.Equal(0, quality.CharCount);
    }

    [Fact]
    public void GetExtractionQuality_TextPdf_ReportsText()
    {
        using var doc = UnpdfDocument.ParseBytes(PdfFixtures.TextPdf());
        var quality = doc.GetExtractionQuality();
        Assert.False(quality.IsScanPdf);
        Assert.True(quality.CharCount > 0);
    }

    [Fact]
    public void GetPageStats_ImageOnlyPdf_CountsImageOpsOnly()
    {
        using var doc = UnpdfDocument.ParseBytes(PdfFixtures.ImageOnlyPdf());
        var stats = doc.GetPageStats(1);
        Assert.Equal(0u, stats.TextOpCount);
        Assert.True(stats.ImageOpCount >= 1);
        Assert.False(stats.OcrTextSuppressed);
    }

    [Fact]
    public void GetPageStats_TextPdf_CountsTextOps()
    {
        using var doc = UnpdfDocument.ParseBytes(PdfFixtures.TextPdf());
        var stats = doc.GetPageStats(1);
        Assert.True(stats.TextOpCount >= 1);
        Assert.Equal(0u, stats.ImageOpCount);
    }

    [Fact]
    public void GetPageStats_OutOfRange_Throws()
    {
        using var doc = UnpdfDocument.ParseBytes(PdfFixtures.TextPdf());
        Assert.Throws<UnpdfException>(() => doc.GetPageStats(99));
    }
}

/// <summary>
/// Minimal synthetic PDF builders — Rust 통합 테스트(tests/common/mod.rs)와 동일 구조.
/// </summary>
internal static class PdfFixtures
{
    /// <summary>One page with a single line of visible Helvetica text.</summary>
    public static byte[] TextPdf()
    {
        var content = "BT /F1 12 Tf 72 720 Td (Hello World) Tj ET\n";
        return Assemble(new[]
        {
            "<</Type/Catalog/Pages 2 0 R>>",
            "<</Type/Pages/Kids[3 0 R]/Count 1>>",
            "<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]" +
                "/Resources<</Font<</F1 5 0 R>>>>/Contents 4 0 R>>",
            StreamObject($"<</Length {content.Length}>>", content),
            "<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>",
        });
    }

    /// <summary>One page drawn as a single full-page image, no text operators.</summary>
    public static byte[] ImageOnlyPdf()
    {
        var content = "q 595 0 0 842 0 0 cm /Im0 Do Q\n";
        return Assemble(new[]
        {
            "<</Type/Catalog/Pages 2 0 R>>",
            "<</Type/Pages/Kids[3 0 R]/Count 1>>",
            "<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]" +
                "/Resources<</XObject<</Im0 5 0 R>>>>/Contents 4 0 R>>",
            StreamObject($"<</Length {content.Length}>>", content),
            // 1×1 grey image — the CTM it is drawn with does the scaling.
            StreamObject(
                "<</Type/XObject/Subtype/Image/Width 1/Height 1/ColorSpace/DeviceGray" +
                "/BitsPerComponent 8/Length 1>>",
                "\u0080"),
        });
    }

    private static string StreamObject(string dict, string data)
        => dict + "\nstream\n" + data + "\nendstream";

    private static byte[] Assemble(string[] objects)
    {
        var latin1 = Encoding.Latin1;
        var pdf = new List<byte>(latin1.GetBytes("%PDF-1.4\n"));
        var offsets = new List<int>();
        for (var i = 0; i < objects.Length; i++)
        {
            offsets.Add(pdf.Count);
            pdf.AddRange(latin1.GetBytes($"{i + 1} 0 obj\n"));
            pdf.AddRange(latin1.GetBytes(objects[i]));
            pdf.AddRange(latin1.GetBytes("\nendobj\n"));
        }

        var xrefStart = pdf.Count;
        var size = objects.Length + 1;
        pdf.AddRange(latin1.GetBytes($"xref\n0 {size}\n0000000000 65535 f \n"));
        foreach (var offset in offsets)
            pdf.AddRange(latin1.GetBytes($"{offset:D10} 00000 n \n"));
        pdf.AddRange(latin1.GetBytes(
            $"trailer\n<</Size {size}/Root 1 0 R>>\nstartxref\n{xrefStart}\n%%EOF\n"));
        return pdf.ToArray();
    }
}
