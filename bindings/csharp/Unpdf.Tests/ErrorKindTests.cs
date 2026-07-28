using System;
using System.Text;
using Xunit;

namespace Unpdf.Tests;

/// <summary>
/// Error classification surface: <see cref="UnpdfException.Kind"/>.
/// 파싱/조회가 실패했을 때 소비자가 사유를 메시지 문자열 매칭 없이 분기할 수 있어야 한다.
/// </summary>
public class ErrorKindTests
{
    [Fact]
    public void ParseBytes_NonPdfInput_ClassifiesTheFailure()
    {
        var junk = Encoding.UTF8.GetBytes("this is plainly not a PDF at all");
        var ex = Assert.Throws<UnpdfException>(() => UnpdfDocument.ParseBytes(junk));

        // 어떤 사유든 분류되어야 한다 — 메시지는 있는데 Kind 가 None 이면 표면이 무의미하다.
        Assert.NotEqual(UnpdfErrorKind.None, ex.Kind);
    }

    [Fact]
    public void GetPageStats_OutOfRangePage_ClassifiesAsPageOutOfRange()
    {
        using var doc = UnpdfDocument.ParseBytes(PdfFixtures.TextPdf());
        var ex = Assert.Throws<UnpdfException>(() => doc.GetPageStats(99));
        Assert.Equal(UnpdfErrorKind.PageOutOfRange, ex.Kind);
    }

    [Fact]
    public void PageToText_OutOfRangePage_ClassifiesAsPageOutOfRange()
    {
        using var doc = UnpdfDocument.ParseBytes(PdfFixtures.TextPdf());
        var ex = Assert.Throws<UnpdfException>(() => doc.PageToText(99));
        Assert.Equal(UnpdfErrorKind.PageOutOfRange, ex.Kind);
    }

    /// <summary>
    /// Kind 없이 만든 예외는 Other 로 떨어진다 — 분류를 모른다고 None(성공)으로
    /// 보고하면 소비자가 실패를 성공으로 오독한다.
    /// </summary>
    [Fact]
    public void UnpdfException_WithoutKind_DefaultsToOther()
    {
        var ex = new UnpdfException("something went wrong");
        Assert.Equal(UnpdfErrorKind.Other, ex.Kind);
    }

    /// <summary>
    /// 이 값들은 네이티브 ABI(<c>UnpdfErrorKind</c> in unpdf.h)를 손으로 복제한 것이다.
    /// 고정해 두어야 코어 쪽 번호가 바뀌었을 때 조용한 오분류 대신 실패로 드러난다.
    /// </summary>
    [Fact]
    public void ErrorKindValues_MatchTheNativeAbi()
    {
        Assert.Equal(0, (int)UnpdfErrorKind.None);
        Assert.Equal(1, (int)UnpdfErrorKind.Other);
        Assert.Equal(2, (int)UnpdfErrorKind.Io);
        Assert.Equal(3, (int)UnpdfErrorKind.UnknownFormat);
        Assert.Equal(4, (int)UnpdfErrorKind.UnsupportedVersion);
        Assert.Equal(5, (int)UnpdfErrorKind.PdfParse);
        Assert.Equal(6, (int)UnpdfErrorKind.Encrypted);
        Assert.Equal(7, (int)UnpdfErrorKind.InvalidPassword);
        Assert.Equal(8, (int)UnpdfErrorKind.Corrupted);
        Assert.Equal(9, (int)UnpdfErrorKind.MissingObject);
        Assert.Equal(10, (int)UnpdfErrorKind.FontDecode);
        Assert.Equal(11, (int)UnpdfErrorKind.ImageExtract);
        Assert.Equal(12, (int)UnpdfErrorKind.Render);
        Assert.Equal(13, (int)UnpdfErrorKind.TextExtract);
        Assert.Equal(14, (int)UnpdfErrorKind.PageOutOfRange);
        Assert.Equal(15, (int)UnpdfErrorKind.InvalidPageRange);
        Assert.Equal(16, (int)UnpdfErrorKind.ResourceNotFound);
        Assert.Equal(17, (int)UnpdfErrorKind.Encoding);
        Assert.Equal(100, (int)UnpdfErrorKind.InvalidArgument);
        Assert.Equal(101, (int)UnpdfErrorKind.Panic);
        Assert.Equal(102, (int)UnpdfErrorKind.InvalidOutput);

        // 코어(src/error.rs)와 unpdf.h 가 값을 늘렸는데 여기가 따라오지 않으면
        // 이 개수 검사가 먼저 깨진다 — 대표값만 고정하면 놓치는 경로다.
        Assert.Equal(21, Enum.GetValues<UnpdfErrorKind>().Length);
    }
}
