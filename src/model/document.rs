//! Document-level types.

use super::{Page, Resource};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A parsed PDF document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    /// Document metadata (title, author, etc.)
    pub metadata: Metadata,

    /// Pages in the document
    pub pages: Vec<Page>,

    /// Embedded resources (images, fonts, etc.)
    pub resources: HashMap<String, Resource>,

    /// Document outline (bookmarks)
    pub outline: Option<Outline>,
}

impl Document {
    /// Create a new empty document.
    pub fn new() -> Self {
        Self {
            metadata: Metadata::default(),
            pages: Vec::new(),
            resources: HashMap::new(),
            outline: None,
        }
    }

    /// Get the number of pages in the document.
    pub fn page_count(&self) -> u32 {
        self.pages.len() as u32
    }

    /// Get a page by number (1-indexed).
    pub fn get_page(&self, page_num: u32) -> Option<&Page> {
        if page_num == 0 {
            return None;
        }
        self.pages.get((page_num - 1) as usize)
    }

    /// Add a page to the document.
    pub fn add_page(&mut self, page: Page) {
        self.pages.push(page);
    }

    /// Add a resource to the document.
    pub fn add_resource(&mut self, id: String, resource: Resource) {
        self.resources.insert(id, resource);
    }

    /// Get a resource by ID.
    pub fn get_resource(&self, id: &str) -> Option<&Resource> {
        self.resources.get(id)
    }

    /// Check if the document has any pages.
    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }

    /// Get plain text content of the entire document.
    pub fn plain_text(&self) -> String {
        self.pages
            .iter()
            .map(|page| page.plain_text())
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

/// Document metadata.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Metadata {
    /// Document title
    pub title: Option<String>,

    /// Document author
    pub author: Option<String>,

    /// Document subject
    pub subject: Option<String>,

    /// Keywords
    pub keywords: Option<String>,

    /// Creator application
    pub creator: Option<String>,

    /// PDF producer
    pub producer: Option<String>,

    /// Creation date
    pub created: Option<DateTime<Utc>>,

    /// Last modification date
    pub modified: Option<DateTime<Utc>>,

    /// PDF version (e.g., "1.7")
    pub pdf_version: String,

    /// Total number of pages
    pub page_count: u32,

    /// Whether the document is encrypted
    pub encrypted: bool,

    /// Whether the document is tagged (accessible)
    pub tagged: bool,
}

impl Metadata {
    /// Create new metadata with PDF version.
    pub fn with_version(version: impl Into<String>) -> Self {
        Self {
            pdf_version: version.into(),
            ..Default::default()
        }
    }

    /// Convert metadata to YAML frontmatter format.
    pub fn to_yaml_frontmatter(&self) -> String {
        let mut lines = vec!["---".to_string()];

        if let Some(ref title) = self.title {
            lines.push(format!("title: \"{}\"", escape_yaml(title)));
        }
        if let Some(ref author) = self.author {
            lines.push(format!("author: \"{}\"", escape_yaml(author)));
        }
        if let Some(ref subject) = self.subject {
            lines.push(format!("subject: \"{}\"", escape_yaml(subject)));
        }
        if let Some(ref keywords) = self.keywords {
            lines.push(format!("keywords: \"{}\"", escape_yaml(keywords)));
        }
        if let Some(ref creator) = self.creator {
            lines.push(format!("creator: \"{}\"", escape_yaml(creator)));
        }
        if let Some(ref producer) = self.producer {
            lines.push(format!("producer: \"{}\"", escape_yaml(producer)));
        }
        if let Some(ref created) = self.created {
            lines.push(format!("created: {}", created.to_rfc3339()));
        }
        if let Some(ref modified) = self.modified {
            lines.push(format!("modified: {}", modified.to_rfc3339()));
        }

        lines.push(format!("pdf_version: \"{}\"", self.pdf_version));
        lines.push(format!("pages: {}", self.page_count));

        lines.push("---".to_string());
        lines.push(String::new());

        lines.join("\n")
    }
}

/// Escape special characters for YAML strings.
fn escape_yaml(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

/// Document outline (bookmarks/table of contents).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Outline {
    /// Top-level outline items
    pub items: Vec<OutlineItem>,
}

impl Outline {
    /// Create a new empty outline.
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// Add an item to the outline.
    pub fn add_item(&mut self, item: OutlineItem) {
        self.items.push(item);
    }

    /// Check if the outline is empty.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Get the total number of items (including nested).
    pub fn total_items(&self) -> usize {
        fn count_items(items: &[OutlineItem]) -> usize {
            items
                .iter()
                .map(|item| 1 + count_items(&item.children))
                .sum()
        }
        count_items(&self.items)
    }
}

/// A single outline item (bookmark).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutlineItem {
    /// Item title
    pub title: String,

    /// Target page number (1-indexed)
    pub page: Option<u32>,

    /// Nesting level (0 = top level)
    pub level: u8,

    /// Child items
    pub children: Vec<OutlineItem>,
}

impl OutlineItem {
    /// Create a new outline item.
    pub fn new(title: impl Into<String>, page: Option<u32>, level: u8) -> Self {
        Self {
            title: title.into(),
            page,
            level,
            children: Vec::new(),
        }
    }

    /// Add a child item.
    pub fn add_child(&mut self, child: OutlineItem) {
        self.children.push(child);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_document_new() {
        let doc = Document::new();
        assert!(doc.is_empty());
        assert_eq!(doc.page_count(), 0);
    }

    #[test]
    fn test_metadata_frontmatter() {
        let mut metadata = Metadata::with_version("1.7");
        metadata.title = Some("Test Document".to_string());
        metadata.author = Some("John Doe".to_string());
        metadata.page_count = 10;

        let yaml = metadata.to_yaml_frontmatter();
        assert!(yaml.contains("title: \"Test Document\""));
        assert!(yaml.contains("author: \"John Doe\""));
        assert!(yaml.contains("pdf_version: \"1.7\""));
        assert!(yaml.contains("pages: 10"));
    }

    #[test]
    fn test_outline() {
        let mut outline = Outline::new();
        let mut chapter1 = OutlineItem::new("Chapter 1", Some(1), 0);
        chapter1.add_child(OutlineItem::new("Section 1.1", Some(2), 1));
        chapter1.add_child(OutlineItem::new("Section 1.2", Some(5), 1));
        outline.add_item(chapter1);

        assert_eq!(outline.total_items(), 3);
    }
}
