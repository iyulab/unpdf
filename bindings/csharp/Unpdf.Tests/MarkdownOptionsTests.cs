using Xunit;

namespace Unpdf.Tests;

/// <summary>
/// A render option the core has had for several releases is only real to a .NET caller
/// once <see cref="MarkdownOptions"/> can name it and the flag it maps to reaches the
/// renderer. Nothing about a call reports an option that was silently ignored — the
/// render succeeds and simply does not do what was asked.
/// </summary>
public class MarkdownOptionsTests
{
    [Fact]
    public void PageMarkers_Off_ByDefault()
    {
        using var doc = UnpdfDocument.ParseBytes(PdfFixtures.TextPdf());
        Assert.DoesNotContain("<!-- page ", doc.ToMarkdown());
    }

    [Fact]
    public void PageMarkers_MarksEachPageBoundary()
    {
        using var doc = UnpdfDocument.ParseBytes(PdfFixtures.TextPdf());

        var markdown = doc.ToMarkdown(new MarkdownOptions { PageMarkers = true });

        Assert.Contains("<!-- page 1 -->", markdown);
    }

    /// <summary>
    /// Options are independent: asking for one must not switch on another.
    /// </summary>
    [Fact]
    public void PageMarkers_DoesNotImplyFrontmatter()
    {
        using var doc = UnpdfDocument.ParseBytes(PdfFixtures.TextPdf());

        var markdown = doc.ToMarkdown(new MarkdownOptions { PageMarkers = true });

        Assert.False(markdown.StartsWith("---"), markdown);
    }

    [Fact]
    public void PageMarkers_CombinesWithFrontmatter()
    {
        using var doc = UnpdfDocument.ParseBytes(PdfFixtures.TextPdf());

        var markdown = doc.ToMarkdown(new MarkdownOptions
        {
            IncludeFrontmatter = true,
            PageMarkers = true,
        });

        Assert.StartsWith("---", markdown);
        Assert.Contains("<!-- page 1 -->", markdown);
    }

    [Fact]
    public void Refine_Off_ByDefault()
    {
        var opts = new MarkdownOptions();
        Assert.False(opts.Refine);
    }

    [Fact]
    public void Refine_DoesNotErrorWhenEnabled()
    {
        using var doc = UnpdfDocument.ParseBytes(PdfFixtures.TextPdf());

        var markdown = doc.ToMarkdown(new MarkdownOptions { Refine = true });

        Assert.NotEmpty(markdown);
    }
}
