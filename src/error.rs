//! Error types for unpdf library.

use std::io;
use thiserror::Error;

/// Result type alias for unpdf operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Stable classification of an [`Error`], for consumers that must branch on the
/// *reason* a call failed rather than match on its message text.
///
/// Each variant corresponds one-to-one with an [`Error`] variant, so the mapping
/// needs no judgement and stays obvious as the error type grows. The discriminants
/// are explicit and part of the public contract: they cross the C-ABI boundary as
/// `unpdf_last_error_kind` return values, so **existing values must never be
/// renumbered** — a new error reason takes the next free number instead.
///
/// Values `100` and above are reserved for FFI-boundary reasons that have no core
/// `Error` counterpart (null arguments, caught panics); see the `unpdf_error_*`
/// constants in the `ffi` module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ErrorKind {
    /// [`Error::Other`] — an error with no more specific classification.
    Other = 1,
    /// [`Error::Io`]
    Io = 2,
    /// [`Error::UnknownFormat`]
    UnknownFormat = 3,
    /// [`Error::UnsupportedVersion`]
    UnsupportedVersion = 4,
    /// [`Error::PdfParse`]
    PdfParse = 5,
    /// [`Error::Encrypted`]
    Encrypted = 6,
    /// [`Error::InvalidPassword`]
    InvalidPassword = 7,
    /// [`Error::Corrupted`]
    Corrupted = 8,
    /// [`Error::MissingObject`]
    MissingObject = 9,
    /// [`Error::FontDecode`]
    FontDecode = 10,
    /// [`Error::ImageExtract`]
    ImageExtract = 11,
    /// [`Error::Render`]
    Render = 12,
    /// [`Error::TextExtract`]
    TextExtract = 13,
    /// [`Error::PageOutOfRange`]
    PageOutOfRange = 14,
    /// [`Error::InvalidPageRange`]
    InvalidPageRange = 15,
    /// [`Error::ResourceNotFound`]
    ResourceNotFound = 16,
    /// [`Error::Encoding`]
    Encoding = 17,
}

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

impl Error {
    /// Classify this error into a stable [`ErrorKind`].
    ///
    /// Lets callers branch on the failure reason without parsing the message —
    /// e.g. telling an encrypted document apart from a corrupted one when both
    /// surface as "extraction failed".
    pub fn kind(&self) -> ErrorKind {
        match self {
            Error::Other(_) => ErrorKind::Other,
            Error::Io(_) => ErrorKind::Io,
            Error::UnknownFormat => ErrorKind::UnknownFormat,
            Error::UnsupportedVersion(_) => ErrorKind::UnsupportedVersion,
            Error::PdfParse(_) => ErrorKind::PdfParse,
            Error::Encrypted => ErrorKind::Encrypted,
            Error::InvalidPassword => ErrorKind::InvalidPassword,
            Error::Corrupted(_) => ErrorKind::Corrupted,
            Error::MissingObject(_) => ErrorKind::MissingObject,
            Error::FontDecode(_) => ErrorKind::FontDecode,
            Error::ImageExtract(_) => ErrorKind::ImageExtract,
            Error::Render(_) => ErrorKind::Render,
            Error::TextExtract(_) => ErrorKind::TextExtract,
            Error::PageOutOfRange(_, _) => ErrorKind::PageOutOfRange,
            Error::InvalidPageRange(_) => ErrorKind::InvalidPageRange,
            Error::ResourceNotFound(_) => ErrorKind::ResourceNotFound,
            Error::Encoding(_) => ErrorKind::Encoding,
        }
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
    fn test_error_kind_classification() {
        assert_eq!(Error::Encrypted.kind(), ErrorKind::Encrypted);
        assert_eq!(Error::InvalidPassword.kind(), ErrorKind::InvalidPassword);
        assert_eq!(
            Error::Corrupted("bad xref".into()).kind(),
            ErrorKind::Corrupted
        );
        assert_eq!(
            Error::PageOutOfRange(10, 5).kind(),
            ErrorKind::PageOutOfRange
        );
        assert_eq!(Error::Other("boom".into()).kind(), ErrorKind::Other);
    }

    /// These values cross the C-ABI boundary and are duplicated by hand in
    /// `bindings/unpdf.h` (`UnpdfErrorKind`). Pinning every one of them here is what
    /// makes that duplication safe: renumbering shows up as a failure instead of as
    /// silently misclassified errors in a consumer. Adding a reason means adding a
    /// line with the next free number — never reusing or shifting one.
    #[test]
    fn test_error_kind_discriminants_are_stable() {
        assert_eq!(ErrorKind::Other as i32, 1);
        assert_eq!(ErrorKind::Io as i32, 2);
        assert_eq!(ErrorKind::UnknownFormat as i32, 3);
        assert_eq!(ErrorKind::UnsupportedVersion as i32, 4);
        assert_eq!(ErrorKind::PdfParse as i32, 5);
        assert_eq!(ErrorKind::Encrypted as i32, 6);
        assert_eq!(ErrorKind::InvalidPassword as i32, 7);
        assert_eq!(ErrorKind::Corrupted as i32, 8);
        assert_eq!(ErrorKind::MissingObject as i32, 9);
        assert_eq!(ErrorKind::FontDecode as i32, 10);
        assert_eq!(ErrorKind::ImageExtract as i32, 11);
        assert_eq!(ErrorKind::Render as i32, 12);
        assert_eq!(ErrorKind::TextExtract as i32, 13);
        assert_eq!(ErrorKind::PageOutOfRange as i32, 14);
        assert_eq!(ErrorKind::InvalidPageRange as i32, 15);
        assert_eq!(ErrorKind::ResourceNotFound as i32, 16);
        assert_eq!(ErrorKind::Encoding as i32, 17);
    }

    #[test]
    fn test_io_error_conversion() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "file not found");
        let err: Error = io_err.into();
        assert!(matches!(err, Error::Io(_)));
    }
}
