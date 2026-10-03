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
    public void GetPageStats_FormDrawnPdf_CountsTheFormsTextAndNotAnImage()
    {
        using var doc = UnpdfDocument.ParseBytes(PdfFixtures.FormXObjectTextPdf());
        var stats = doc.GetPageStats(1);
        Assert.Equal(1u, stats.TextOpCount);
        Assert.Equal(0u, stats.ImageOpCount);
        Assert.Equal(1u, stats.FormOpCount);
        Assert.Contains("Hello from form xobject", doc.PlainText());
        Assert.False(doc.GetExtractionQuality().IsScanPdf);
    }

    [Fact]
    public void GetResourceInfo_ReportsThePageTheResourceCameFrom()
    {
        using var doc = UnpdfDocument.ParseBytes(
            PdfFixtures.JpegPdf(100, 100), new ParseOptions { ExtractResources = true });
        var id = Assert.Single(doc.GetResourceIds());
        using var info = doc.GetResourceInfo(id);
        Assert.NotNull(info);
        Assert.Equal(1, info!.RootElement.GetProperty("page").GetInt32());
    }

    [Fact]
    public void GetPageStats_OutOfRange_Throws()
    {
        using var doc = UnpdfDocument.ParseBytes(PdfFixtures.TextPdf());
        Assert.Throws<UnpdfException>(() => doc.GetPageStats(99));
    }

    /// <summary>
    /// The per-page count is what the document-level total is built from, so a
    /// consumer discriminating causes across pages in a mixed-quality document
    /// needs it on <see cref="PageStats"/> too, not just <see cref="ExtractionQuality"/>.
    /// </summary>
    [Fact]
    public void GetPageStats_UnresolvableFont_ReportsSuppressedTextRuns()
    {
        using var doc = UnpdfDocument.ParseBytes(PdfFixtures.SuppressedTextRunPdf());
        var stats = doc.GetPageStats(1);
        var quality = doc.GetExtractionQuality();

        Assert.True(stats.SuppressedTextRuns > 0);
        Assert.Equal(quality.SuppressedTextRuns, stats.SuppressedTextRuns);
    }

    [Fact]
    public void GetExtractionQuality_IntactPdf_ReportsComplete()
    {
        using var doc = UnpdfDocument.ParseBytes(PdfFixtures.TextPdf());
        var quality = doc.GetExtractionQuality();
        Assert.False(quality.PagesIncomplete);
        Assert.Equal(1, quality.DeclaredPageCount);
        Assert.Equal(0, quality.UnresolvedPageNodes);
        Assert.Equal(0, quality.SkippedObjectCount);
        Assert.Equal(0, quality.UndecodableContentStreams);
    }

    /// <summary>
    /// Lenient parsing keeps a page whose content stream cannot be decoded, as an empty
    /// page — without this count the loss reads exactly like a blank page.
    /// </summary>
    [Fact]
    public void GetPageStats_UndecodableContentStream_ReportsIt()
    {
        using var doc = UnpdfDocument.ParseBytes(PdfFixtures.UndecodableContentPdf());
        var stats = doc.GetPageStats(1);
        var quality = doc.GetExtractionQuality();

        Assert.Equal(1, stats.UndecodableContentStreams);
        Assert.Equal(quality.UndecodableContentStreams, stats.UndecodableContentStreams);
    }

    /// <summary>
    /// 손상 문서가 페이지를 조용히 버리는 것을 소비자가 관측할 수 있어야 한다.
    /// 파싱은 성공하고 SectionCount 는 1을 반환하므로, 이 신호가 없으면
    /// "1페이지 문서를 온전히 추출한 것"과 구별되지 않는다.
    /// </summary>
    [Fact]
    public void GetExtractionQuality_DamagedPageTree_ReportsIncomplete()
    {
        using var doc = UnpdfDocument.ParseBytes(PdfFixtures.LostPagePdf());
        var quality = doc.GetExtractionQuality();

        Assert.Equal(1, doc.SectionCount);
        Assert.True(quality.PagesIncomplete);
        Assert.Equal(2, quality.DeclaredPageCount);
        Assert.True(quality.UnresolvedPageNodes >= 1);
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
    /// <summary>
    /// One page whose content only paints a Form XObject; the text lives in the form,
    /// under the form's own resources.
    /// </summary>
    public static byte[] FormXObjectTextPdf()
    {
        var page = "q /Fm1 Do Q\n";
        var form = "BT /F1 24 Tf 72 700 Td (Hello from form xobject) Tj ET\n";
        return Assemble(new[]
        {
            "<</Type/Catalog/Pages 2 0 R>>",
            "<</Type/Pages/Kids[3 0 R]/Count 1>>",
            "<</Type/Page/Parent 2 0 R/MediaBox[0 0 612 792]" +
                "/Resources<</XObject<</Fm1 6 0 R>>>>/Contents 4 0 R>>",
            StreamObject($"<</Length {page.Length}>>", page),
            "<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>",
            StreamObject(
                "<</Type/XObject/Subtype/Form/BBox[0 0 612 792]" +
                    $"/Resources<</Font<</F1 5 0 R>>>>/Length {form.Length}>>",
                form),
        });
    }

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

    /// <summary>
    /// Declares two pages but its second kid points at an object that is not there —
    /// the shape a damaged page tree takes: one page survives, one is lost, and the
    /// parse still succeeds.
    /// </summary>
    public static byte[] LostPagePdf()
    {
        var content = "BT /F1 12 Tf 72 720 Td (Page one) Tj ET\n";
        return Assemble(new[]
        {
            "<</Type/Catalog/Pages 2 0 R>>",
            "<</Type/Pages/Kids[3 0 R 99 0 R]/Count 2>>",
            "<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]" +
                "/Resources<</Font<</F1 5 0 R>>>>/Contents 4 0 R>>",
            StreamObject($"<</Length {content.Length}>>", content),
            "<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>",
        });
    }

    /// <summary>
    /// One page whose text uses an Identity-H composite font with no
    /// <c>ToUnicode</c> map and no embedded cmap — the decoder has no way to turn
    /// its CIDs into characters, so the run is discarded and counted as suppressed.
    /// Mirrors the Rust fixture in <c>tests/suppression_reporting_test.rs</c>.
    /// </summary>
    public static byte[] SuppressedTextRunPdf()
    {
        // Two CIDs (\x01\x42, \x01\x43) — byte-wise Latin-1 reading would be wrong.
        var content = "BT /F1 12 Tf 72 720 Td (BC) Tj ET\n";
        return Assemble(new[]
        {
            "<</Type/Catalog/Pages 2 0 R>>",
            "<</Type/Pages/Kids[3 0 R]/Count 1>>",
            "<</Type/Page/Parent 2 0 R/MediaBox[0 0 612 792]" +
                "/Resources<</Font<</F1 5 0 R>>>>/Contents 4 0 R>>",
            StreamObject($"<</Length {content.Length}>>", content),
            "<</Type/Font/Subtype/Type0/BaseFont/NoMap/Encoding/Identity-H" +
                "/DescendantFonts[6 0 R]>>",
            "<</Type/Font/Subtype/CIDFontType2/BaseFont/NoMap" +
                "/CIDSystemInfo<</Registry(Adobe)/Ordering(Identity)/Supplement 0>>>>",
        });
    }

    /// <summary>
    /// One page with a single <c>DCTDecode</c>-tagged image XObject of the given pixel size.
    /// The bytes are not a real decodable JPEG — nothing decodes them — but the <c>Filter</c>
    /// entry is what the parser uses to classify a resource as a renderable image format, as
    /// opposed to the raw/undecoded pixel buffer <see cref="ImageOnlyPdf"/> produces.
    /// </summary>
    public static byte[] JpegPdf(int width, int height)
    {
        var content = "q 595 0 0 842 0 0 cm /Im0 Do Q\n";
        return Assemble(new[]
        {
            "<</Type/Catalog/Pages 2 0 R>>",
            "<</Type/Pages/Kids[3 0 R]/Count 1>>",
            "<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]" +
                "/Resources<</XObject<</Im0 5 0 R>>>>/Contents 4 0 R>>",
            StreamObject($"<</Length {content.Length}>>", content),
            StreamObject(
                $"<</Type/XObject/Subtype/Image/Width {width}/Height {height}" +
                "/ColorSpace/DeviceRGB/BitsPerComponent 8/Filter/DCTDecode/Length 4>>",
                "ÿØÿÙ"),
        });
    }

    /// <summary>
    /// One page whose only content stream claims <c>FlateDecode</c> but holds bytes no
    /// decoder accepts (no zlib header begins 0xFF 0xFF). Mirrors
    /// <c>undecodable_content_pdf</c> in <c>tests/common/mod.rs</c>.
    /// </summary>
    public static byte[] UndecodableContentPdf()
    {
        var notFlate = new string('ÿ', 16);
        return Assemble(new[]
        {
            "<</Type/Catalog/Pages 2 0 R>>",
            "<</Type/Pages/Kids[3 0 R]/Count 1>>",
            "<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]" +
                "/Resources<</Font<</F1 5 0 R>>>>/Contents 4 0 R>>",
            StreamObject($"<</Length {notFlate.Length}/Filter/FlateDecode>>", notFlate),
            "<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>",
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
