namespace Unpdf;

/// <summary>
/// Why an unpdf call failed, so callers can branch on the reason instead of
/// matching on message text.
/// </summary>
/// <remarks>
/// Values 1–17 mirror the library's own failure reasons one-to-one; values 100+ are
/// raised at the interop boundary and have no library-side counterpart. The numbers
/// are part of the native ABI (<c>UnpdfErrorKind</c> in <c>unpdf.h</c>): a new reason
/// takes the next free number and existing ones are never renumbered, so treat an
/// unrecognised value as a generic failure rather than as an error.
/// </remarks>
public enum UnpdfErrorKind
{
    /// <summary>The last call succeeded — no error is recorded.</summary>
    None = 0,

    /// <summary>A failure with no more specific classification.</summary>
    Other = 1,

    /// <summary>An I/O failure, such as a missing or unreadable file.</summary>
    Io = 2,

    /// <summary>The input is not a valid PDF.</summary>
    UnknownFormat = 3,

    /// <summary>The PDF version is not supported.</summary>
    UnsupportedVersion = 4,

    /// <summary>The PDF structure could not be parsed.</summary>
    PdfParse = 5,

    /// <summary>The document is encrypted and needs a password.</summary>
    Encrypted = 6,

    /// <summary>The supplied password is incorrect.</summary>
    InvalidPassword = 7,

    /// <summary>The PDF structure is corrupted or malformed.</summary>
    Corrupted = 8,

    /// <summary>A required PDF object is missing.</summary>
    MissingObject = 9,

    /// <summary>Font data could not be decoded.</summary>
    FontDecode = 10,

    /// <summary>An image could not be extracted.</summary>
    ImageExtract = 11,

    /// <summary>Rendering to Markdown, text, or JSON failed.</summary>
    Render = 12,

    /// <summary>Text content could not be extracted.</summary>
    TextExtract = 13,

    /// <summary>The requested page number is out of range.</summary>
    PageOutOfRange = 14,

    /// <summary>The page range specification is invalid.</summary>
    InvalidPageRange = 15,

    /// <summary>The requested resource is not present in the document.</summary>
    ResourceNotFound = 16,

    /// <summary>A text encoding failure.</summary>
    Encoding = 17,

    /// <summary>An argument was null or not valid UTF-8.</summary>
    InvalidArgument = 100,

    /// <summary>A panic was caught at the interop boundary.</summary>
    Panic = 101,

    /// <summary>The produced output holds an interior NUL byte and cannot cross the ABI.</summary>
    InvalidOutput = 102,
}
