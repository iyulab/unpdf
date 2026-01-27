using System.Runtime.InteropServices;

namespace Unpdf;

/// <summary>
/// Result structure returned by FFI functions.
/// </summary>
[StructLayout(LayoutKind.Sequential)]
internal struct UnpdfResult
{
    [MarshalAs(UnmanagedType.I1)]
    public bool Success;
    public IntPtr Data;
    public IntPtr Error;
}

/// <summary>
/// P/Invoke declarations for the native unpdf library.
/// </summary>
internal static class NativeMethods
{
    private const string LibraryName = "unpdf";

    [DllImport(LibraryName, EntryPoint = "unpdf_to_markdown", CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern UnpdfResult ToMarkdown([MarshalAs(UnmanagedType.LPUTF8Str)] string path);

    [DllImport(LibraryName, EntryPoint = "unpdf_to_text", CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern UnpdfResult ToText([MarshalAs(UnmanagedType.LPUTF8Str)] string path);

    [DllImport(LibraryName, EntryPoint = "unpdf_to_json", CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern UnpdfResult ToJson([MarshalAs(UnmanagedType.LPUTF8Str)] string path, [MarshalAs(UnmanagedType.I1)] bool pretty);

    [DllImport(LibraryName, EntryPoint = "unpdf_get_info", CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern UnpdfResult GetInfo([MarshalAs(UnmanagedType.LPUTF8Str)] string path);

    [DllImport(LibraryName, EntryPoint = "unpdf_get_page_count", CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern int GetPageCount([MarshalAs(UnmanagedType.LPUTF8Str)] string path);

    [DllImport(LibraryName, EntryPoint = "unpdf_is_pdf", CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    [return: MarshalAs(UnmanagedType.I1)]
    internal static extern bool IsPdf([MarshalAs(UnmanagedType.LPUTF8Str)] string path);

    [DllImport(LibraryName, EntryPoint = "unpdf_free_result", CallingConvention = CallingConvention.Cdecl)]
    internal static extern void FreeResult(UnpdfResult result);

    [DllImport(LibraryName, EntryPoint = "unpdf_version", CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr GetVersion();
}
