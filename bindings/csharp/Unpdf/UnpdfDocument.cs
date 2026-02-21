using System;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json;

namespace Unpdf;

/// <summary>
/// Options for markdown rendering.
/// </summary>
public class MarkdownOptions
{
    /// <summary>
    /// Include YAML frontmatter with document metadata.
    /// </summary>
    public bool IncludeFrontmatter { get; set; } = false;

    /// <summary>
    /// Escape special markdown characters.
    /// </summary>
    public bool EscapeSpecialChars { get; set; } = false;

    /// <summary>
    /// Add extra spacing between paragraphs.
    /// </summary>
    public bool ParagraphSpacing { get; set; } = false;

    internal int ToFlags()
    {
        int flags = 0;
        if (IncludeFrontmatter) flags |= NativeMethods.UNPDF_FLAG_FRONTMATTER;
        if (EscapeSpecialChars) flags |= NativeMethods.UNPDF_FLAG_ESCAPE_SPECIAL;
        if (ParagraphSpacing) flags |= NativeMethods.UNPDF_FLAG_PARAGRAPH_SPACING;
        return flags;
    }
}

/// <summary>
/// Represents a parsed PDF document.
/// </summary>
/// <remarks>
/// This class provides methods to extract content from PDF documents
/// in various formats (Markdown, plain text, JSON).
/// </remarks>
public class UnpdfDocument : IDisposable
{
    private IntPtr _handle;
    private bool _disposed;

    private UnpdfDocument(IntPtr handle)
    {
        _handle = handle;
    }

    /// <summary>
    /// Get the unpdf library version.
    /// </summary>
    public static string Version
    {
        get
        {
            var ptr = NativeMethods.unpdf_version();
            return Marshal.PtrToStringAnsi(ptr) ?? "unknown";
        }
    }

    /// <summary>
    /// Parse a document from a file path.
    /// </summary>
    /// <param name="path">Path to the PDF file</param>
    /// <returns>Parsed document</returns>
    /// <exception cref="UnpdfException">If parsing fails</exception>
    /// <exception cref="FileNotFoundException">If file doesn't exist</exception>
    public static UnpdfDocument ParseFile(string path)
    {
        if (!System.IO.File.Exists(path))
            throw new System.IO.FileNotFoundException($"File not found: {path}", path);

        var handle = NativeMethods.unpdf_parse_file(path);
        if (handle == IntPtr.Zero)
            throw new UnpdfException($"Failed to parse {path}: {GetLastError()}");

        return new UnpdfDocument(handle);
    }

    /// <summary>
    /// Parse a document from a byte array.
    /// </summary>
    /// <param name="data">Document content as bytes</param>
    /// <returns>Parsed document</returns>
    /// <exception cref="UnpdfException">If parsing fails</exception>
    public static UnpdfDocument ParseBytes(byte[] data)
    {
        var dataPtr = Marshal.AllocHGlobal(data.Length);
        try
        {
            Marshal.Copy(data, 0, dataPtr, data.Length);
            var handle = NativeMethods.unpdf_parse_bytes(dataPtr, (UIntPtr)data.Length);
            if (handle == IntPtr.Zero)
                throw new UnpdfException($"Failed to parse bytes: {GetLastError()}");

            return new UnpdfDocument(handle);
        }
        finally
        {
            Marshal.FreeHGlobal(dataPtr);
        }
    }

    /// <summary>
    /// Convert the document to Markdown.
    /// </summary>
    /// <param name="options">Optional rendering options</param>
    /// <returns>Markdown string</returns>
    public string ToMarkdown(MarkdownOptions? options = null)
    {
        ThrowIfDisposed();
        int flags = options?.ToFlags() ?? 0;
        var ptr = NativeMethods.unpdf_to_markdown(_handle, flags);
        if (ptr == IntPtr.Zero)
            throw new UnpdfException($"Failed to convert to markdown: {GetLastError()}");

        try
        {
            return PtrToStringUtf8(ptr);
        }
        finally
        {
            NativeMethods.unpdf_free_string(ptr);
        }
    }

    /// <summary>
    /// Convert the document to plain text.
    /// </summary>
    /// <returns>Plain text string</returns>
    public string ToText()
    {
        ThrowIfDisposed();
        var ptr = NativeMethods.unpdf_to_text(_handle);
        if (ptr == IntPtr.Zero)
            throw new UnpdfException($"Failed to convert to text: {GetLastError()}");

        try
        {
            return PtrToStringUtf8(ptr);
        }
        finally
        {
            NativeMethods.unpdf_free_string(ptr);
        }
    }

    /// <summary>
    /// Convert the document to JSON.
    /// </summary>
    /// <param name="compact">Use compact JSON format</param>
    /// <returns>JSON string</returns>
    public string ToJson(bool compact = false)
    {
        ThrowIfDisposed();
        int format = compact ? NativeMethods.UNPDF_JSON_COMPACT : NativeMethods.UNPDF_JSON_PRETTY;
        var ptr = NativeMethods.unpdf_to_json(_handle, format);
        if (ptr == IntPtr.Zero)
            throw new UnpdfException($"Failed to convert to JSON: {GetLastError()}");

        try
        {
            return PtrToStringUtf8(ptr);
        }
        finally
        {
            NativeMethods.unpdf_free_string(ptr);
        }
    }

    /// <summary>
    /// Get plain text content (faster than ToText for simple extraction).
    /// </summary>
    /// <returns>Plain text string</returns>
    public string PlainText()
    {
        ThrowIfDisposed();
        var ptr = NativeMethods.unpdf_plain_text(_handle);
        if (ptr == IntPtr.Zero)
            throw new UnpdfException($"Failed to get plain text: {GetLastError()}");

        try
        {
            return PtrToStringUtf8(ptr);
        }
        finally
        {
            NativeMethods.unpdf_free_string(ptr);
        }
    }

    /// <summary>
    /// Get the number of sections (pages) in the document.
    /// </summary>
    public int SectionCount
    {
        get
        {
            ThrowIfDisposed();
            var count = NativeMethods.unpdf_section_count(_handle);
            if (count < 0)
                throw new UnpdfException($"Failed to get section count: {GetLastError()}");
            return count;
        }
    }

    /// <summary>
    /// Get the number of resources in the document.
    /// </summary>
    public int ResourceCount
    {
        get
        {
            ThrowIfDisposed();
            var count = NativeMethods.unpdf_resource_count(_handle);
            if (count < 0)
                throw new UnpdfException($"Failed to get resource count: {GetLastError()}");
            return count;
        }
    }

    /// <summary>
    /// Get the document title, if set.
    /// </summary>
    public string? Title
    {
        get
        {
            ThrowIfDisposed();
            var ptr = NativeMethods.unpdf_get_title(_handle);
            if (ptr == IntPtr.Zero)
                return null;

            try
            {
                return PtrToStringUtf8(ptr);
            }
            finally
            {
                NativeMethods.unpdf_free_string(ptr);
            }
        }
    }

    /// <summary>
    /// Get the document author, if set.
    /// </summary>
    public string? Author
    {
        get
        {
            ThrowIfDisposed();
            var ptr = NativeMethods.unpdf_get_author(_handle);
            if (ptr == IntPtr.Zero)
                return null;

            try
            {
                return PtrToStringUtf8(ptr);
            }
            finally
            {
                NativeMethods.unpdf_free_string(ptr);
            }
        }
    }

    /// <summary>
    /// Get list of resource IDs in the document.
    /// </summary>
    /// <returns>Array of resource ID strings</returns>
    public string[] GetResourceIds()
    {
        ThrowIfDisposed();
        var ptr = NativeMethods.unpdf_get_resource_ids(_handle);
        if (ptr == IntPtr.Zero)
            return Array.Empty<string>();

        try
        {
            var json = PtrToStringUtf8(ptr);
            return JsonSerializer.Deserialize<string[]>(json) ?? Array.Empty<string>();
        }
        finally
        {
            NativeMethods.unpdf_free_string(ptr);
        }
    }

    /// <summary>
    /// Get metadata for a resource.
    /// </summary>
    /// <param name="resourceId">The resource ID</param>
    /// <returns>Resource metadata as JSON, or null if not found</returns>
    public JsonDocument? GetResourceInfo(string resourceId)
    {
        ThrowIfDisposed();
        var ptr = NativeMethods.unpdf_get_resource_info(_handle, resourceId);
        if (ptr == IntPtr.Zero)
            return null;

        try
        {
            var json = PtrToStringUtf8(ptr);
            return JsonDocument.Parse(json);
        }
        finally
        {
            NativeMethods.unpdf_free_string(ptr);
        }
    }

    /// <summary>
    /// Get binary data for a resource.
    /// </summary>
    /// <param name="resourceId">The resource ID</param>
    /// <returns>Resource data as bytes, or null if not found</returns>
    public byte[]? GetResourceData(string resourceId)
    {
        ThrowIfDisposed();
        var ptr = NativeMethods.unpdf_get_resource_data(_handle, resourceId, out var length);
        if (ptr == IntPtr.Zero)
            return null;

        try
        {
            var data = new byte[(int)length];
            Marshal.Copy(ptr, data, 0, data.Length);
            return data;
        }
        finally
        {
            NativeMethods.unpdf_free_bytes(ptr, length);
        }
    }

    private static string GetLastError()
    {
        var ptr = NativeMethods.unpdf_last_error();
        if (ptr == IntPtr.Zero)
            return "Unknown error";
        return Marshal.PtrToStringAnsi(ptr) ?? "Unknown error";
    }

    private static string PtrToStringUtf8(IntPtr ptr)
    {
        if (ptr == IntPtr.Zero)
            return string.Empty;

        // Find null terminator
        int len = 0;
        while (Marshal.ReadByte(ptr, len) != 0)
            len++;

        if (len == 0)
            return string.Empty;

        byte[] buffer = new byte[len];
        Marshal.Copy(ptr, buffer, 0, len);
        return Encoding.UTF8.GetString(buffer);
    }

    private void ThrowIfDisposed()
    {
        if (_disposed)
            throw new ObjectDisposedException(nameof(UnpdfDocument));
    }

    public void Dispose()
    {
        Dispose(true);
        GC.SuppressFinalize(this);
    }

    protected virtual void Dispose(bool disposing)
    {
        if (!_disposed)
        {
            if (_handle != IntPtr.Zero)
            {
                NativeMethods.unpdf_free_document(_handle);
                _handle = IntPtr.Zero;
            }
            _disposed = true;
        }
    }

    ~UnpdfDocument()
    {
        Dispose(false);
    }
}
