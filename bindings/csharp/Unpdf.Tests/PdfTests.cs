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

    /// <summary>
    /// 경로는 UTF-8로 마샬링되어야 한다. ANSI 로 넘어가면 비-ASCII 파일명이
    /// 네이티브 측에서 다른 경로가 되어 "파일 없음"으로 실패한다 —
    /// 오류 메시지 디코딩에서 실제로 발생했던 결함(0.10.0 수정)의 반대 방향.
    /// </summary>
    [Fact]
    public void ParseFile_NonAsciiPath_ParsesTheSameDocument()
    {
        var dir = Path.Combine(Path.GetTempPath(), "unpdf-경로-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(dir);
        var path = Path.Combine(dir, "문서-テスト-Ünïcode.pdf");
        try
        {
            var bytes = PdfFixtures.TextPdf();
            File.WriteAllBytes(path, bytes);

            using var fromPath = UnpdfDocument.ParseFile(path);
            using var fromBytes = UnpdfDocument.ParseBytes(bytes);

            Assert.Equal(fromBytes.SectionCount, fromPath.SectionCount);
            Assert.Equal(fromBytes.ToMarkdown(), fromPath.ToMarkdown());
        }
        finally
        {
            Directory.Delete(dir, recursive: true);
        }
    }
}
