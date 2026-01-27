//! Document model types for PDF content representation.
//!
//! This module defines the intermediate representation (IR) that bridges
//! PDF parsing and content rendering. The model is format-agnostic and
//! can represent content from any PDF document.

mod document;
mod page;
mod paragraph;
mod resource;
mod table;

pub use document::{Document, Metadata, Outline, OutlineItem};
pub use page::{Block, Page};
pub use paragraph::{
    Alignment, InlineContent, ListInfo, ListStyle, NumberStyle, Paragraph, ParagraphStyle, TextRun,
    TextStyle,
};
pub use resource::{Resource, ResourceType};
pub use table::{Table, TableCell, TableRow};
