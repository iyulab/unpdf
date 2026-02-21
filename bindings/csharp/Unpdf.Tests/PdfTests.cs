using Xunit;

namespace Unpdf.Tests;

public class UnpdfDocumentTests
{
    [Fact]
    public void Version_ReturnsNonEmptyString()
    {
        var version = UnpdfDocument.Version;
        Assert.NotNull(version);
        Assert.NotEmpty(version);
    }

    [Fact]
    public void ParseFile_NonExistentFile_ThrowsFileNotFoundException()
    {
        Assert.Throws<FileNotFoundException>(() => UnpdfDocument.ParseFile("non_existent_file.pdf"));
    }

    [Fact]
    public void ParseBytes_EmptyBytes_ThrowsUnpdfException()
    {
        Assert.Throws<UnpdfException>(() => UnpdfDocument.ParseBytes(Array.Empty<byte>()));
    }
}
