use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use unpdf::model::{Metadata, Page};
use unpdf::render::{CleanupPipeline, PageMarkerStyle, RenderOptions, StreamingRenderer};

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
    md_path: Option<PathBuf>,
    txt: Option<BufWriter<File>>,
    json: Option<BufWriter<File>>,
    render_opts: RenderOptions,
    json_first_page: bool,
    /// 이미지 출력 디렉토리. None 이면 이미지를 디스크에 쓰지 않음.
    /// 디렉토리는 `MultiFormatWriter::new` 이전에 생성되어 있어야 하며,
    /// 첫 이미지가 실제로 쓰일 때까지는 `images_created` 로 지연 확인.
    images_dir: Option<PathBuf>,
    images_created: bool,
    image_count: u32,
    /// Tracks whether any content has been written to the MD file.
    /// Used to determine correct page marker spacing.
    md_written: bool,
}

impl MultiFormatWriter {
    pub fn new(
        out_dir: &Path,
        formats: &[OutputFormat],
        render_opts: RenderOptions,
        images_dir: Option<PathBuf>,
    ) -> std::io::Result<Self> {
        let has = |f: OutputFormat| formats.iter().any(|x| *x == f);
        let md_path = has(OutputFormat::Markdown).then(|| out_dir.join("extract.md"));
        let md = if let Some(ref p) = md_path {
            Some(BufWriter::new(File::create(p)?))
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
            md_path,
            txt,
            json,
            render_opts,
            json_first_page: true,
            images_dir,
            images_created: false,
            image_count: 0,
            md_written: false,
        })
    }

    /// 페이지별 이미지를 디스크로 flush. 첫 이미지가 있을 때 디렉토리 생성.
    fn flush_page_images(&mut self, page: &Page) -> std::io::Result<()> {
        let Some(dir) = self.images_dir.clone() else {
            return Ok(());
        };
        if page.images.is_empty() {
            return Ok(());
        }
        if !self.images_created {
            std::fs::create_dir_all(&dir)?;
            self.images_created = true;
        }
        for (id, resource) in &page.images {
            let path = dir.join(id);
            std::fs::write(&path, &resource.data)?;
            self.image_count += 1;
        }
        Ok(())
    }

    /// 디스크로 flush 된 이미지 개수 (finish 후 호출 시 최종값).
    pub fn image_count(&self) -> u32 {
        self.image_count
    }

    pub fn write_document_start(
        &mut self,
        metadata: &Metadata,
        page_count: u32,
    ) -> std::io::Result<()> {
        if let Some(w) = self.md.as_mut() {
            if self.render_opts.include_frontmatter {
                w.write_all(metadata.to_yaml_frontmatter().as_bytes())?;
                self.md_written = true;
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
        // 이미지 먼저 flush — MD 의 `![](images/X.jpg)` 참조가 가리키는 파일이
        // 존재하도록 순서 보장.
        self.flush_page_images(page)?;

        if let Some(w) = self.md.as_mut() {
            if self.render_opts.page_markers == PageMarkerStyle::Comment {
                let marker = if self.md_written {
                    format!("\n<!-- page {} -->\n\n", page.number)
                } else {
                    format!("<!-- page {} -->\n\n", page.number)
                };
                w.write_all(marker.as_bytes())?;
                self.md_written = true;
            }
            let placeholder = unpdf::model::Document::new();
            let renderer = StreamingRenderer::new(&placeholder, self.render_opts.clone());
            for block in &page.elements {
                let chunk = renderer.render_block_public(block);
                if !chunk.is_empty() {
                    w.write_all(chunk.as_bytes())?;
                    self.md_written = true;
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
            drop(w);
            // Streaming renderer bypasses the CleanupPipeline. Apply
            // configured cleanup now as a read-modify-write pass on the
            // completed MD file. Keeps per-page streaming memory profile
            // while still delivering standard/aggressive cleanup semantics.
            if let (Some(path), Some(ref cleanup_opts)) =
                (self.md_path.as_ref(), &self.render_opts.cleanup)
            {
                let raw = std::fs::read_to_string(path)?;
                let cleaned = CleanupPipeline::new(cleanup_opts.clone()).process(&raw);
                std::fs::write(path, cleaned)?;
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use unpdf::model::{Page, Paragraph};
    use unpdf::render::PageMarkerStyle;

    #[test]
    fn test_streaming_writer_inserts_page_marker() {
        let tmp = std::env::temp_dir().join("unpdf_writer_marker_test");
        std::fs::create_dir_all(&tmp).unwrap();

        let doc = unpdf::model::Document::new();
        let render_opts = RenderOptions::new()
            .with_page_markers(PageMarkerStyle::Comment)
            .with_cleanup(unpdf::render::CleanupOptions::from_preset(
                unpdf::CleanupPreset::Minimal,
            ));
        let formats = vec![OutputFormat::Markdown];
        let mut mfw = MultiFormatWriter::new(&tmp, &formats, render_opts, None).unwrap();

        mfw.write_document_start(&doc.metadata, 2).unwrap();

        let mut page1 = Page::letter(1);
        page1.add_paragraph(Paragraph::with_text("Page one text"));
        mfw.write_page(&page1).unwrap();

        let mut page2 = Page::letter(2);
        page2.add_paragraph(Paragraph::with_text("Page two text"));
        mfw.write_page(&page2).unwrap();

        mfw.finish().unwrap();

        let content = std::fs::read_to_string(tmp.join("extract.md")).unwrap();
        assert!(
            content.contains("<!-- page 1 -->"),
            "page 1 marker missing:\n{}",
            content
        );
        assert!(
            content.contains("<!-- page 2 -->"),
            "page 2 marker missing:\n{}",
            content
        );

        let p1_pos = content.find("<!-- page 1 -->").unwrap();
        let p2_pos = content.find("<!-- page 2 -->").unwrap();
        assert!(p1_pos < p2_pos, "page 1 marker must precede page 2 marker");

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn test_streaming_writer_no_marker_by_default() {
        let tmp = std::env::temp_dir().join("unpdf_writer_no_marker_test");
        std::fs::create_dir_all(&tmp).unwrap();

        let doc = unpdf::model::Document::new();
        let render_opts = RenderOptions::new();
        let formats = vec![OutputFormat::Markdown];
        let mut mfw = MultiFormatWriter::new(&tmp, &formats, render_opts, None).unwrap();

        mfw.write_document_start(&doc.metadata, 1).unwrap();
        let mut page = Page::letter(1);
        page.add_paragraph(Paragraph::with_text("Content"));
        mfw.write_page(&page).unwrap();
        mfw.finish().unwrap();

        let content = std::fs::read_to_string(tmp.join("extract.md")).unwrap();
        assert!(
            !content.contains("<!-- page "),
            "unexpected marker:\n{}",
            content
        );

        std::fs::remove_dir_all(&tmp).ok();
    }
}
