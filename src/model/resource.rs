//! Resource types for embedded content (images, fonts, etc.)

use serde::{Deserialize, Serialize};

/// An embedded resource in the document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resource {
    /// Raw binary data
    #[serde(skip_serializing)]
    pub data: Vec<u8>,

    /// MIME type (e.g., "image/jpeg")
    pub mime_type: String,

    /// Resource type
    pub resource_type: ResourceType,

    /// Original filename if known
    pub filename: Option<String>,

    /// Width in pixels (for images)
    pub width: Option<u32>,

    /// Height in pixels (for images)
    pub height: Option<u32>,

    /// Color space (e.g., "RGB", "CMYK", "Gray")
    pub color_space: Option<String>,

    /// Bits per component (e.g., 8)
    pub bits_per_component: Option<u8>,
}

impl Resource {
    /// Create a new resource.
    pub fn new(data: Vec<u8>, mime_type: impl Into<String>, resource_type: ResourceType) -> Self {
        Self {
            data,
            mime_type: mime_type.into(),
            resource_type,
            filename: None,
            width: None,
            height: None,
            color_space: None,
            bits_per_component: None,
        }
    }

    /// Create an image resource.
    pub fn image(data: Vec<u8>, mime_type: impl Into<String>) -> Self {
        Self::new(data, mime_type, ResourceType::Image)
    }

    /// Create a JPEG image resource.
    pub fn jpeg(data: Vec<u8>) -> Self {
        Self::image(data, "image/jpeg")
    }

    /// Create a PNG image resource.
    pub fn png(data: Vec<u8>) -> Self {
        Self::image(data, "image/png")
    }

    /// Set image dimensions.
    pub fn with_dimensions(mut self, width: u32, height: u32) -> Self {
        self.width = Some(width);
        self.height = Some(height);
        self
    }

    /// Set color space.
    pub fn with_color_space(mut self, color_space: impl Into<String>) -> Self {
        self.color_space = Some(color_space.into());
        self
    }

    /// Set bits per component.
    pub fn with_bits_per_component(mut self, bits: u8) -> Self {
        self.bits_per_component = Some(bits);
        self
    }

    /// Set filename.
    pub fn with_filename(mut self, filename: impl Into<String>) -> Self {
        self.filename = Some(filename.into());
        self
    }

    /// Get the size of the resource data in bytes.
    pub fn size(&self) -> usize {
        self.data.len()
    }

    /// Check if this is an image resource.
    pub fn is_image(&self) -> bool {
        matches!(self.resource_type, ResourceType::Image)
    }

    /// Check if this is a font resource.
    pub fn is_font(&self) -> bool {
        matches!(self.resource_type, ResourceType::Font)
    }

    /// Get a suggested filename based on resource type and ID.
    pub fn suggested_filename(&self, id: &str) -> String {
        if let Some(ref filename) = self.filename {
            return filename.clone();
        }

        let extension = self.extension();
        format!("{}.{}", id, extension)
    }

    /// Get the file extension based on MIME type.
    pub fn extension(&self) -> &str {
        match self.mime_type.as_str() {
            "image/jpeg" => "jpg",
            "image/png" => "png",
            "image/gif" => "gif",
            "image/tiff" => "tiff",
            "image/bmp" => "bmp",
            "image/webp" => "webp",
            "image/jp2" | "image/jpeg2000" => "jp2",
            "application/pdf" => "pdf",
            "font/ttf" | "font/truetype" => "ttf",
            "font/otf" | "font/opentype" => "otf",
            "font/woff" => "woff",
            "font/woff2" => "woff2",
            // For raw image data without recognized format, use .raw
            _ if self.is_image() => "raw",
            _ => "bin",
        }
    }

    /// Detect MIME type from data magic bytes.
    pub fn detect_mime_type(data: &[u8]) -> Option<&'static str> {
        if data.len() < 8 {
            return None;
        }

        // JPEG: FF D8 FF
        if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
            return Some("image/jpeg");
        }

        // PNG: 89 50 4E 47 0D 0A 1A 0A
        if data.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
            return Some("image/png");
        }

        // GIF: GIF87a or GIF89a
        if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
            return Some("image/gif");
        }

        // TIFF: 49 49 2A 00 (little-endian) or 4D 4D 00 2A (big-endian)
        if data.starts_with(&[0x49, 0x49, 0x2A, 0x00])
            || data.starts_with(&[0x4D, 0x4D, 0x00, 0x2A])
        {
            return Some("image/tiff");
        }

        // BMP: BM
        if data.starts_with(b"BM") {
            return Some("image/bmp");
        }

        // WEBP: RIFF....WEBP
        if data.len() >= 12 && data.starts_with(b"RIFF") && &data[8..12] == b"WEBP" {
            return Some("image/webp");
        }

        // JPEG 2000: 00 00 00 0C 6A 50 20 20
        if data.starts_with(&[0x00, 0x00, 0x00, 0x0C, 0x6A, 0x50, 0x20, 0x20]) {
            return Some("image/jp2");
        }

        None
    }
}

/// Type of embedded resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResourceType {
    /// Image (JPEG, PNG, etc.)
    Image,
    /// Font
    Font,
    /// Embedded file attachment
    Attachment,
    /// Other/unknown
    Other,
}

impl std::fmt::Display for ResourceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResourceType::Image => write!(f, "image"),
            ResourceType::Font => write!(f, "font"),
            ResourceType::Attachment => write!(f, "attachment"),
            ResourceType::Other => write!(f, "other"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resource_new() {
        let res = Resource::jpeg(vec![0xFF, 0xD8, 0xFF]);
        assert!(res.is_image());
        assert_eq!(res.mime_type, "image/jpeg");
        assert_eq!(res.extension(), "jpg");
    }

    #[test]
    fn test_detect_mime_type() {
        // JPEG
        let jpeg_data = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46];
        assert_eq!(Resource::detect_mime_type(&jpeg_data), Some("image/jpeg"));

        // PNG
        let png_data = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        assert_eq!(Resource::detect_mime_type(&png_data), Some("image/png"));

        // Unknown
        let unknown = vec![0x00, 0x00, 0x00, 0x00];
        assert_eq!(Resource::detect_mime_type(&unknown), None);
    }

    #[test]
    fn test_suggested_filename() {
        let res = Resource::jpeg(vec![]).with_filename("photo.jpg");
        assert_eq!(res.suggested_filename("img1"), "photo.jpg");

        let res2 = Resource::png(vec![]);
        assert_eq!(res2.suggested_filename("img2"), "img2.png");
    }
}
