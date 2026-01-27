//! PDF format detection and validation.

use crate::error::{Error, Result};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

/// PDF format information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfFormat {
    /// PDF version (e.g., "1.7", "2.0")
    pub version: String,
    /// Whether the file appears to be linearized (fast web view)
    pub linearized: bool,
}

impl std::fmt::Display for PdfFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PDF {}", self.version)
    }
}

/// PDF magic bytes: %PDF-
const PDF_MAGIC: &[u8] = b"%PDF-";
const PDF_MAGIC_LEN: usize = 5;
const VERSION_LEN: usize = 3; // e.g., "1.7"

/// Detect PDF format from a file path.
///
/// # Arguments
/// * `path` - Path to the PDF file
///
/// # Returns
/// * `Ok(PdfFormat)` if the file is a valid PDF
/// * `Err(Error::UnknownFormat)` if the file is not a PDF
///
/// # Example
/// ```no_run
/// use unpdf::detect::detect_format_from_path;
///
/// let format = detect_format_from_path("document.pdf").unwrap();
/// println!("PDF version: {}", format.version);
/// ```
pub fn detect_format_from_path<P: AsRef<Path>>(path: P) -> Result<PdfFormat> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut header = [0u8; 16];
    reader.read_exact(&mut header)?;
    detect_format_from_bytes(&header)
}

/// Detect PDF format from bytes.
///
/// # Arguments
/// * `data` - Byte slice containing at least the first 16 bytes of the file
///
/// # Returns
/// * `Ok(PdfFormat)` if the data starts with valid PDF header
/// * `Err(Error::UnknownFormat)` if the data is not a PDF
pub fn detect_format_from_bytes(data: &[u8]) -> Result<PdfFormat> {
    if data.len() < PDF_MAGIC_LEN + VERSION_LEN {
        return Err(Error::UnknownFormat);
    }

    // Check for PDF magic bytes
    if !data.starts_with(PDF_MAGIC) {
        return Err(Error::UnknownFormat);
    }

    // Extract version string (e.g., "1.7" from "%PDF-1.7")
    let version_bytes = &data[PDF_MAGIC_LEN..PDF_MAGIC_LEN + VERSION_LEN];
    let version = String::from_utf8_lossy(version_bytes).to_string();

    // Validate version format (should be like "1.0" to "2.0")
    if !is_valid_version(&version) {
        return Err(Error::UnsupportedVersion(version));
    }

    Ok(PdfFormat {
        version,
        linearized: false, // TODO: Detect linearization from file structure
    })
}

/// Check if a version string is valid.
fn is_valid_version(version: &str) -> bool {
    if version.len() != 3 {
        return false;
    }

    let chars: Vec<char> = version.chars().collect();
    chars[0].is_ascii_digit() && chars[1] == '.' && chars[2].is_ascii_digit()
}

/// Check if a file is a valid PDF.
///
/// # Arguments
/// * `path` - Path to the file
///
/// # Returns
/// * `true` if the file is a valid PDF
/// * `false` otherwise
pub fn is_pdf<P: AsRef<Path>>(path: P) -> bool {
    detect_format_from_path(path).is_ok()
}

/// Check if bytes represent a valid PDF.
///
/// # Arguments
/// * `data` - Byte slice to check
///
/// # Returns
/// * `true` if the data is a valid PDF header
/// * `false` otherwise
pub fn is_pdf_bytes(data: &[u8]) -> bool {
    detect_format_from_bytes(data).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_valid_pdf() {
        let data = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3";
        let format = detect_format_from_bytes(data).unwrap();
        assert_eq!(format.version, "1.7");
    }

    #[test]
    fn test_detect_pdf_2_0() {
        let data = b"%PDF-2.0\n%\xe2\xe3\xcf\xd3";
        let format = detect_format_from_bytes(data).unwrap();
        assert_eq!(format.version, "2.0");
    }

    #[test]
    fn test_detect_invalid_format() {
        let data = b"<!DOCTYPE html>";
        let result = detect_format_from_bytes(data);
        assert!(matches!(result, Err(Error::UnknownFormat)));
    }

    #[test]
    fn test_detect_too_short() {
        let data = b"%PDF";
        let result = detect_format_from_bytes(data);
        assert!(matches!(result, Err(Error::UnknownFormat)));
    }

    #[test]
    fn test_is_pdf_bytes() {
        assert!(is_pdf_bytes(b"%PDF-1.4\n"));
        assert!(!is_pdf_bytes(b"Not a PDF"));
    }

    #[test]
    fn test_version_validation() {
        assert!(is_valid_version("1.0"));
        assert!(is_valid_version("1.7"));
        assert!(is_valid_version("2.0"));
        assert!(!is_valid_version("10.0"));
        assert!(!is_valid_version("abc"));
    }
}
