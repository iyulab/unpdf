// unpdf - PDF content extraction library
// C# P/Invoke bindings

using System;
using System.Runtime.InteropServices;

namespace Unpdf
{
    /// <summary>
    /// Native P/Invoke bindings for the unpdf library.
    /// </summary>
    internal static class UnpdfNative
    {
        private const string LibraryName = "unpdf";

        [StructLayout(LayoutKind.Sequential)]
        internal struct UnpdfResult
        {
            [MarshalAs(UnmanagedType.U1)]
            public bool Success;
            public IntPtr Data;
            public IntPtr Error;
        }

        [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
        internal static extern UnpdfResult unpdf_to_markdown(
            [MarshalAs(UnmanagedType.LPUTF8Str)] string path);

        [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
        internal static extern UnpdfResult unpdf_to_text(
            [MarshalAs(UnmanagedType.LPUTF8Str)] string path);

        [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
        internal static extern UnpdfResult unpdf_to_json(
            [MarshalAs(UnmanagedType.LPUTF8Str)] string path,
            [MarshalAs(UnmanagedType.U1)] bool pretty);

        [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
        internal static extern UnpdfResult unpdf_get_info(
            [MarshalAs(UnmanagedType.LPUTF8Str)] string path);

        [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
        internal static extern int unpdf_get_page_count(
            [MarshalAs(UnmanagedType.LPUTF8Str)] string path);

        [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
        [return: MarshalAs(UnmanagedType.U1)]
        internal static extern bool unpdf_is_pdf(
            [MarshalAs(UnmanagedType.LPUTF8Str)] string path);

        [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
        internal static extern void unpdf_free_result(UnpdfResult result);

        [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
        internal static extern IntPtr unpdf_version();
    }

    /// <summary>
    /// Exception thrown when an unpdf operation fails.
    /// </summary>
    public class UnpdfException : Exception
    {
        public UnpdfException(string message) : base(message) { }
    }

    /// <summary>
    /// PDF content extraction library.
    /// </summary>
    public static class PdfExtractor
    {
        /// <summary>
        /// Get the version of the unpdf library.
        /// </summary>
        public static string Version
        {
            get
            {
                var ptr = UnpdfNative.unpdf_version();
                return Marshal.PtrToStringUTF8(ptr) ?? string.Empty;
            }
        }

        /// <summary>
        /// Convert a PDF file to Markdown.
        /// </summary>
        /// <param name="path">Path to the PDF file.</param>
        /// <returns>The Markdown content.</returns>
        /// <exception cref="UnpdfException">Thrown when the conversion fails.</exception>
        public static string ToMarkdown(string path)
        {
            var result = UnpdfNative.unpdf_to_markdown(path);
            return ProcessResult(result);
        }

        /// <summary>
        /// Convert a PDF file to plain text.
        /// </summary>
        /// <param name="path">Path to the PDF file.</param>
        /// <returns>The text content.</returns>
        /// <exception cref="UnpdfException">Thrown when the conversion fails.</exception>
        public static string ToText(string path)
        {
            var result = UnpdfNative.unpdf_to_text(path);
            return ProcessResult(result);
        }

        /// <summary>
        /// Convert a PDF file to JSON.
        /// </summary>
        /// <param name="path">Path to the PDF file.</param>
        /// <param name="pretty">Whether to format the JSON with indentation.</param>
        /// <returns>The JSON content.</returns>
        /// <exception cref="UnpdfException">Thrown when the conversion fails.</exception>
        public static string ToJson(string path, bool pretty = true)
        {
            var result = UnpdfNative.unpdf_to_json(path, pretty);
            return ProcessResult(result);
        }

        /// <summary>
        /// Get document information as JSON.
        /// </summary>
        /// <param name="path">Path to the PDF file.</param>
        /// <returns>Document metadata as JSON.</returns>
        /// <exception cref="UnpdfException">Thrown when the operation fails.</exception>
        public static string GetInfo(string path)
        {
            var result = UnpdfNative.unpdf_get_info(path);
            return ProcessResult(result);
        }

        /// <summary>
        /// Get the page count of a PDF file.
        /// </summary>
        /// <param name="path">Path to the PDF file.</param>
        /// <returns>Number of pages, or -1 on error.</returns>
        public static int GetPageCount(string path)
        {
            return UnpdfNative.unpdf_get_page_count(path);
        }

        /// <summary>
        /// Check if a file is a valid PDF.
        /// </summary>
        /// <param name="path">Path to the file.</param>
        /// <returns>True if the file is a valid PDF.</returns>
        public static bool IsPdf(string path)
        {
            return UnpdfNative.unpdf_is_pdf(path);
        }

        private static string ProcessResult(UnpdfNative.UnpdfResult result)
        {
            try
            {
                if (!result.Success)
                {
                    var errorMessage = Marshal.PtrToStringUTF8(result.Error) ?? "Unknown error";
                    throw new UnpdfException(errorMessage);
                }

                return Marshal.PtrToStringUTF8(result.Data) ?? string.Empty;
            }
            finally
            {
                UnpdfNative.unpdf_free_result(result);
            }
        }
    }
}
