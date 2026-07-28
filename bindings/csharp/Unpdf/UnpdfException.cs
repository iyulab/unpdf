namespace Unpdf;

/// <summary>
/// Exception thrown when unpdf operations fail.
/// </summary>
public class UnpdfException : Exception
{
    /// <summary>
    /// Why the call failed — lets a caller branch on the reason (ask for a password
    /// on <see cref="UnpdfErrorKind.Encrypted"/>, report a damaged file on
    /// <see cref="UnpdfErrorKind.Corrupted"/>) without matching on
    /// <see cref="Exception.Message"/>.
    /// </summary>
    /// <remarks>
    /// <see cref="UnpdfErrorKind.Other"/> when the failure did not come from the
    /// native library and so carries no classification.
    /// </remarks>
    public UnpdfErrorKind Kind { get; }

    /// <summary>
    /// Creates a new UnpdfException with the specified message.
    /// </summary>
    /// <param name="message">The error message.</param>
    public UnpdfException(string message) : this(message, UnpdfErrorKind.Other)
    {
    }

    /// <summary>
    /// Creates a new UnpdfException with the specified message and failure reason.
    /// </summary>
    /// <param name="message">The error message.</param>
    /// <param name="kind">Why the call failed.</param>
    public UnpdfException(string message, UnpdfErrorKind kind) : base(message)
    {
        Kind = kind;
    }

    /// <summary>
    /// Creates a new UnpdfException with the specified message and inner exception.
    /// </summary>
    /// <param name="message">The error message.</param>
    /// <param name="innerException">The inner exception.</param>
    public UnpdfException(string message, Exception innerException) : base(message, innerException)
    {
        Kind = UnpdfErrorKind.Other;
    }
}
