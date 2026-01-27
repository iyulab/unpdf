//! Error types for unpdf library.

use std::io;
use thiserror::Error;

/// Result type alias for unpdf operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Error types that can occur during PDF processing.
#[derive(Error, Debug)]
pub enum Error {
    /// I/O error when reading or writing files.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    /// The file format is not recognized as PDF.
    #[error("Unknown file format: not a valid PDF")]
    UnknownFormat,

    /// The PDF version is not supported.
    #[error("Unsupported PDF version: {0}")]
    UnsupportedVersion(String),

    /// Error parsing PDF structure.
    #[error("PDF parsing error: {0}")]
    PdfParse(String),

    /// The PDF document is encrypted and requires a password.
    #[error("Document is encrypted")]
    Encrypted,

    /// The provided password is incorrect.
    #[error("Invalid password")]
    InvalidPassword,

    /// The PDF structure is corrupted or malformed.
    #[error("Corrupted PDF structure: {0}")]
    Corrupted(String),

    /// A required PDF object is missing.
    #[error("Missing required object: {0}")]
    MissingObject(String),

    /// Error decoding font data.
    #[error("Font decoding error: {0}")]
    FontDecode(String),

    /// Error extracting images from PDF.
    #[error("Image extraction error: {0}")]
    ImageExtract(String),

    /// Error during rendering (Markdown, text, JSON).
    #[error("Rendering error: {0}")]
    Render(String),

    /// Error extracting text content.
    #[error("Text extraction error: {0}")]
    TextExtract(String),

    /// Page number is out of range.
    #[error("Page {0} is out of range (document has {1} pages)")]
    PageOutOfRange(u32, u32),

    /// Invalid page range specification.
    #[error("Invalid page range: {0}")]
    InvalidPageRange(String),

    /// Resource not found in document.
    #[error("Resource not found: {0}")]
    ResourceNotFound(String),

    /// Encoding error.
    #[error("Encoding error: {0}")]
    Encoding(String),

    /// Generic error with message.
    #[error("{0}")]
    Other(String),
}

impl From<lopdf::Error> for Error {
    fn from(err: lopdf::Error) -> Self {
        match err {
            lopdf::Error::IO(e) => Error::Io(e),
            lopdf::Error::Decryption(_) => Error::Encrypted,
            _ => Error::PdfParse(err.to_string()),
        }
    }
}

impl From<pdf_extract::OutputError> for Error {
    fn from(err: pdf_extract::OutputError) -> Self {
        Error::TextExtract(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = Error::Encrypted;
        assert_eq!(err.to_string(), "Document is encrypted");

        let err = Error::PageOutOfRange(10, 5);
        assert_eq!(
            err.to_string(),
            "Page 10 is out of range (document has 5 pages)"
        );
    }

    #[test]
    fn test_io_error_conversion() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "file not found");
        let err: Error = io_err.into();
        assert!(matches!(err, Error::Io(_)));
    }
}
