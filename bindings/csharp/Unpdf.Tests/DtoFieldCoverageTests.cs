using System.Reflection;
using System.Text.Json;
using System.Text.Json.Serialization;
using Xunit;

namespace Unpdf.Tests;

/// <summary>
/// Guards against the field-drift class of bug where the native FFI's JSON payload
/// gains or renames a field and a strongly-typed C# DTO (<see cref="ExtractionQuality"/>,
/// <see cref="PageStats"/>) silently keeps deserializing without it — <c>[JsonPropertyName]</c>
/// deserialization ignores unknown keys by default, so a dropped field produces no error,
/// just a quietly incomplete object.
/// <para>
/// Deliberately reflection-based on both sides rather than a hand-maintained field list:
/// the native side is read from an actual parsed document's JSON at test time (not a
/// copied-in string), and the DTO side is read from the type's declared properties (not a
/// copied-in name list) — a hand-maintained list is exactly the shape that already
/// drifted once in production.
/// </para>
/// </summary>
public class DtoFieldCoverageTests
{
    [Fact]
    public void ExtractionQuality_CoversEveryNativeField()
    {
        using var doc = UnpdfDocument.ParseBytes(PdfFixtures.TextPdf());
        AssertDtoCoversNativeFields(doc.GetExtractionQualityRawJson(), typeof(ExtractionQuality));
    }

    [Fact]
    public void PageStats_CoversEveryNativeField()
    {
        using var doc = UnpdfDocument.ParseBytes(PdfFixtures.TextPdf());
        AssertDtoCoversNativeFields(doc.GetPageStatsRawJson(1), typeof(PageStats));
    }

    private static void AssertDtoCoversNativeFields(string nativeJson, Type dtoType)
    {
        var nativeFields = NativeJsonFieldNames(nativeJson);
        var dtoFields = DeclaredJsonFieldNames(dtoType);

        var missingFromDto = nativeFields.Except(dtoFields).ToArray();
        var extraOnDto = dtoFields.Except(nativeFields).ToArray();

        Assert.True(
            missingFromDto.Length == 0,
            $"{dtoType.Name} is missing field(s) the native payload sends: " +
                $"[{string.Join(", ", missingFromDto)}]. Native JSON: {nativeJson}");
        Assert.True(
            extraOnDto.Length == 0,
            $"{dtoType.Name} declares field(s) the native payload does not send: " +
                $"[{string.Join(", ", extraOnDto)}]. Native JSON: {nativeJson}");
    }

    private static HashSet<string> NativeJsonFieldNames(string json)
    {
        using var document = JsonDocument.Parse(json);
        return document.RootElement.EnumerateObject().Select(p => p.Name).ToHashSet();
    }

    private static HashSet<string> DeclaredJsonFieldNames(Type dtoType)
    {
        return dtoType
            .GetProperties(BindingFlags.Public | BindingFlags.Instance)
            .Select(prop => prop.GetCustomAttribute<JsonPropertyNameAttribute>()?.Name ?? prop.Name)
            .ToHashSet();
    }
}
