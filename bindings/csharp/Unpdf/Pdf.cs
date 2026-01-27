using System.Runtime.InteropServices;
using System.Text.Json;

namespace Unpdf;

/// <summary>
/// High-level API for PDF content extraction.
/// </summary>
public static class Pdf
{
    /// <summary>
    /// Convert a PDF file to Markdown format.
    /// </summary>
    /// <param name="path">Path to the PDF file.</param>
    /// <returns>The extracted content as Markdown.</returns>
    /// <exception cref="UnpdfException">Thrown when conversion fails.</exception>
    public static string ToMarkdown(string path)
    {
        var result = NativeMethods.ToMarkdown(path);
        return HandleResult(result);
    }

    /// <summary>
    /// Convert a PDF file to plain text.
    /// </summary>
    /// <param name="path">Path to the PDF file.</param>
    /// <returns>The extracted content as plain text.</returns>
    /// <exception cref="UnpdfException">Thrown when conversion fails.</exception>
    public static string ToText(string path)
    {
        var result = NativeMethods.ToText(path);
        return HandleResult(result);
    }

    /// <summary>
    /// Convert a PDF file to JSON format.
    /// </summary>
    /// <param name="path">Path to the PDF file.</param>
    /// <param name="pretty">If true, format JSON with indentation.</param>
    /// <returns>The extracted content as JSON string.</returns>
    /// <exception cref="UnpdfException">Thrown when conversion fails.</exception>
    public static string ToJson(string path, bool pretty = false)
    {
        var result = NativeMethods.ToJson(path, pretty);
        return HandleResult(result);
    }

    /// <summary>
    /// Get document metadata from a PDF file.
    /// </summary>
    /// <param name="path">Path to the PDF file.</param>
    /// <returns>Document information object.</returns>
    /// <exception cref="UnpdfException">Thrown when extraction fails.</exception>
    public static DocumentInfo GetInfo(string path)
    {
        var result = NativeMethods.GetInfo(path);
        var json = HandleResult(result);
        return JsonSerializer.Deserialize<DocumentInfo>(json, JsonOptions)
            ?? throw new UnpdfException("Failed to parse document info");
    }

    /// <summary>
    /// Get the number of pages in a PDF file.
    /// </summary>
    /// <param name="path">Path to the PDF file.</param>
    /// <returns>The number of pages, or -1 on error.</returns>
    public static int GetPageCount(string path)
    {
        return NativeMethods.GetPageCount(path);
    }

    /// <summary>
    /// Check if a file is a valid PDF.
    /// </summary>
    /// <param name="path">Path to the file.</param>
    /// <returns>True if the file is a valid PDF, false otherwise.</returns>
    public static bool IsPdf(string path)
    {
        return NativeMethods.IsPdf(path);
    }

    /// <summary>
    /// Get the version of the native unpdf library.
    /// </summary>
    public static string Version
    {
        get
        {
            var ptr = NativeMethods.GetVersion();
            if (ptr == IntPtr.Zero)
                return "unknown";
            return Marshal.PtrToStringUTF8(ptr) ?? "unknown";
        }
    }

    private static string HandleResult(UnpdfResult result)
    {
        try
        {
            if (result.Success)
            {
                if (result.Data == IntPtr.Zero)
                    return string.Empty;
                return Marshal.PtrToStringUTF8(result.Data) ?? string.Empty;
            }
            else
            {
                var errorMsg = "Unknown error";
                if (result.Error != IntPtr.Zero)
                    errorMsg = Marshal.PtrToStringUTF8(result.Error) ?? errorMsg;
                throw new UnpdfException(errorMsg);
            }
        }
        finally
        {
            NativeMethods.FreeResult(result);
        }
    }

    private static readonly JsonSerializerOptions JsonOptions = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
        PropertyNameCaseInsensitive = true
    };
}
