using Xunit;

namespace Unpdf.Tests;

public class PdfTests
{
    [Fact]
    public void Version_ReturnsNonEmptyString()
    {
        var version = Pdf.Version;
        Assert.NotNull(version);
        Assert.NotEmpty(version);
    }

    [Fact]
    public void IsPdf_NonExistentFile_ReturnsFalse()
    {
        var result = Pdf.IsPdf("non_existent_file.pdf");
        Assert.False(result);
    }

    [Fact]
    public void GetPageCount_NonExistentFile_ReturnsNegativeOne()
    {
        var result = Pdf.GetPageCount("non_existent_file.pdf");
        Assert.Equal(-1, result);
    }

    [Fact]
    public void ToMarkdown_NonExistentFile_ThrowsException()
    {
        Assert.Throws<UnpdfException>(() => Pdf.ToMarkdown("non_existent_file.pdf"));
    }

    [Fact]
    public void ToText_NonExistentFile_ThrowsException()
    {
        Assert.Throws<UnpdfException>(() => Pdf.ToText("non_existent_file.pdf"));
    }

    [Fact]
    public void ToJson_NonExistentFile_ThrowsException()
    {
        Assert.Throws<UnpdfException>(() => Pdf.ToJson("non_existent_file.pdf"));
    }

    [Fact]
    public void GetInfo_NonExistentFile_ThrowsException()
    {
        Assert.Throws<UnpdfException>(() => Pdf.GetInfo("non_existent_file.pdf"));
    }
}
