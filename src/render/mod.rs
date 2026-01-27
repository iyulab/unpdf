//! Rendering module for converting documents to various output formats.

mod cleanup;
mod json;
mod markdown;
mod options;
mod result;
pub mod streaming;
mod text;
pub mod visitor;

pub use cleanup::{CleanupOptions, CleanupPipeline, CleanupPreset};
pub use json::{to_json, JsonFormat};
pub use markdown::{to_markdown, to_markdown_with_stats, MarkdownRenderer};
pub use options::{HeadingConfig, PageSelection, RenderOptions, TableFallback};
pub use result::{ExtractionStats, RenderResult};
pub use streaming::{collect_content, RenderEvent, StreamingRenderer};
pub use text::to_text;
pub use visitor::{CompositeVisitor, DefaultVisitor, DocumentVisitor, VisitorAction};
