use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use unpdf::model::{Metadata, Page};
use unpdf::render::{RenderOptions, StreamingRenderer};

/// Output format selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutputFormat {
    Markdown,
    Text,
    Json,
}

/// Fan-out writer that appends MD/TXT/JSON files page-by-page.
///
/// JSON is written as `{"metadata":..., "pages":[ <p1>, <p2>, ... ]}`
/// with manual comma management.
pub struct MultiFormatWriter {
    md: Option<BufWriter<File>>,
    txt: Option<BufWriter<File>>,
    json: Option<BufWriter<File>>,
    render_opts: RenderOptions,
    json_first_page: bool,
}

impl MultiFormatWriter {
    pub fn new(
        out_dir: &Path,
        formats: &[OutputFormat],
        render_opts: RenderOptions,
    ) -> std::io::Result<Self> {
        let has = |f: OutputFormat| formats.iter().any(|x| *x == f);
        let md = if has(OutputFormat::Markdown) {
            Some(BufWriter::new(File::create(out_dir.join("extract.md"))?))
        } else {
            None
        };
        let txt = if has(OutputFormat::Text) {
            Some(BufWriter::new(File::create(out_dir.join("extract.txt"))?))
        } else {
            None
        };
        let json = if has(OutputFormat::Json) {
            Some(BufWriter::new(File::create(out_dir.join("content.json"))?))
        } else {
            None
        };
        Ok(Self {
            md,
            txt,
            json,
            render_opts,
            json_first_page: true,
        })
    }

    pub fn write_document_start(
        &mut self,
        metadata: &Metadata,
        page_count: u32,
    ) -> std::io::Result<()> {
        if let Some(w) = self.md.as_mut() {
            if self.render_opts.include_frontmatter {
                w.write_all(metadata.to_yaml_frontmatter().as_bytes())?;
            }
        }
        if let Some(w) = self.json.as_mut() {
            w.write_all(b"{\"metadata\":")?;
            serde_json::to_writer(&mut *w, metadata).map_err(io_err)?;
            w.write_all(b",\"page_count\":")?;
            w.write_all(page_count.to_string().as_bytes())?;
            w.write_all(b",\"pages\":[")?;
        }
        Ok(())
    }

    pub fn write_page(&mut self, page: &Page) -> std::io::Result<()> {
        if let Some(w) = self.md.as_mut() {
            let placeholder = unpdf::model::Document::new();
            let renderer = StreamingRenderer::new(&placeholder, self.render_opts.clone());
            for block in &page.elements {
                let chunk = renderer.render_block_public(block);
                if !chunk.is_empty() {
                    w.write_all(chunk.as_bytes())?;
                }
            }
        }
        if let Some(w) = self.txt.as_mut() {
            for block in &page.elements {
                let mut buf = String::new();
                block.append_plain_text(&mut buf);
                if !buf.is_empty() {
                    w.write_all(buf.as_bytes())?;
                    w.write_all(b"\n")?;
                }
            }
        }
        if let Some(w) = self.json.as_mut() {
            if !self.json_first_page {
                w.write_all(b",")?;
            }
            serde_json::to_writer(&mut *w, page).map_err(io_err)?;
            self.json_first_page = false;
        }
        Ok(())
    }

    pub fn finish(mut self) -> std::io::Result<()> {
        if let Some(w) = self.json.as_mut() {
            w.write_all(b"]}")?;
        }
        if let Some(mut w) = self.md.take() {
            w.flush()?;
        }
        if let Some(mut w) = self.txt.take() {
            w.flush()?;
        }
        if let Some(mut w) = self.json.take() {
            w.flush()?;
        }
        Ok(())
    }
}

fn io_err(e: serde_json::Error) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, e)
}
