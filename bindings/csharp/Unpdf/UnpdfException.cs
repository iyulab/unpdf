namespace Unpdf;

/// <summary>
/// Exception thrown when unpdf operations fail.
/// </summary>
public class UnpdfException : Exception
{
    /// <summary>
    /// Creates a new UnpdfException with the specified message.
    /// </summary>
    /// <param name="message">The error message.</param>
    public UnpdfException(string message) : base(message)
    {
    }

    /// <summary>
    /// Creates a new UnpdfException with the specified message and inner exception.
    /// </summary>
    /// <param name="message">The error message.</param>
    /// <param name="innerException">The inner exception.</param>
    public UnpdfException(string message, Exception innerException) : base(message, innerException)
    {
    }
}
