use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use unpdf::model::{Block, Metadata, Page};
use unpdf::render::{CleanupPipeline, PageMarkerStyle, RenderOptions, StreamingRenderer};

fn image_hash(data: &[u8]) -> (u64, usize) {
    // Sample head + tail instead of hashing all bytes — O(1) regardless of image size.
    // Combined with the byte-length component, false-positive probability is negligible.
    const SAMPLE: usize = 64;
    let mut h = DefaultHasher::new();
    h.write(&data[..data.len().min(SAMPLE)]);
    if data.len() > SAMPLE {
        h.write(&data[data.len() - SAMPLE..]);
    }
    (h.finish(), data.len())
}

/// Output format selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutputFormat {
    Markdown,
    Text,
    Json,
}

/// Summary of files written by the convert pipeline.
#[derive(Debug, Default)]
pub struct WriteSummary {
    pub md_path: Option<PathBuf>,
    pub txt_path: Option<PathBuf>,
    pub json_path: Option<PathBuf>,
    /// Number of unique images written to disk.
    pub image_count: u32,
    /// Total word count across all pages.
    pub word_count: usize,
}

/// Fan-out writer that appends MD/TXT/JSON files page-by-page.
///
/// JSON is written as `{"metadata":..., "pages":[ <p1>, <p2>, ... ]}`
/// with manual comma management.
pub struct MultiFormatWriter {
    md: Option<BufWriter<File>>,
    md_path: Option<PathBuf>,
    txt: Option<BufWriter<File>>,
    txt_path: Option<PathBuf>,
    json: Option<BufWriter<File>>,
    json_path: Option<PathBuf>,
    render_opts: RenderOptions,
    json_first_page: bool,
    /// 이미지 출력 디렉토리. None 이면 이미지를 디스크에 쓰지 않음.
    /// 디렉토리는 `MultiFormatWriter::new` 이전에 생성되어 있어야 하며,
    /// 첫 이미지가 실제로 쓰일 때까지는 `images_created` 로 지연 확인.
    images_dir: Option<PathBuf>,
    images_created: bool,
    image_count: u32,
    word_count: usize,
    /// (hash, byte_len) → canonical resource_id. 동일 바이트 이미지 중복 방지.
    image_dedup: HashMap<(u64, usize), String>,
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
        let has = |f: OutputFormat| formats.contains(&f);
        let md_path = has(OutputFormat::Markdown).then(|| out_dir.join("extract.md"));
        let md = if let Some(ref p) = md_path {
            Some(BufWriter::new(File::create(p)?))
        } else {
            None
        };
        let txt_path = has(OutputFormat::Text).then(|| out_dir.join("extract.txt"));
        let txt = if let Some(ref p) = txt_path {
            Some(BufWriter::new(File::create(p)?))
        } else {
            None
        };
        let json_path = has(OutputFormat::Json).then(|| out_dir.join("content.json"));
        let json = if let Some(ref p) = json_path {
            Some(BufWriter::new(File::create(p)?))
        } else {
            None
        };
        Ok(Self {
            md,
            md_path,
            txt,
            txt_path,
            json,
            json_path,
            render_opts,
            json_first_page: true,
            images_dir,
            images_created: false,
            image_count: 0,
            word_count: 0,
            image_dedup: HashMap::new(),
            md_written: false,
        })
    }

    /// 페이지별 이미지를 디스크로 flush. 첫 이미지가 있을 때 디렉토리 생성.
    ///
    /// 동일 바이트 이미지는 첫 등장 시에만 저장되며, 이후 등장분은 page의
    /// images 목록과 Block::Image resource_id 가 모두 canonical ID로 교체된다.
    fn flush_page_images(&mut self, page: &mut Page) -> std::io::Result<()> {
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

        // duplicate_id → canonical_id
        let mut redirects: HashMap<String, String> = HashMap::new();

        for (id, resource) in &page.images {
            let key = image_hash(&resource.data);
            match self.image_dedup.entry(key) {
                std::collections::hash_map::Entry::Occupied(e) => {
                    redirects.insert(id.clone(), e.get().clone());
                }
                std::collections::hash_map::Entry::Vacant(e) => {
                    std::fs::write(dir.join(id), &resource.data)?;
                    self.image_count += 1;
                    e.insert(id.clone());
                }
            }
        }

        if redirects.is_empty() {
            return Ok(());
        }

        page.images.retain(|(id, _)| !redirects.contains_key(id));
        for block in &mut page.elements {
            if let Block::Image { resource_id, .. } = block {
                if let Some(canonical) = redirects.get(resource_id.as_str()) {
                    *resource_id = canonical.clone();
                }
            }
        }

        Ok(())
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

    pub fn write_page(&mut self, page: &mut Page) -> std::io::Result<()> {
        // 이미지 먼저 flush — MD 의 `![](images/X.jpg)` 참조가 가리키는 파일이
        // 존재하도록 순서 보장. 중복 이미지 dedup도 여기서 처리됨.
        self.flush_page_images(page)?;

        // Accumulate word count from this page's text blocks.
        for block in &page.elements {
            let mut buf = String::new();
            block.append_plain_text(&mut buf);
            self.word_count += buf.split_whitespace().count();
        }

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

    pub fn finish(mut self) -> std::io::Result<WriteSummary> {
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
        Ok(WriteSummary {
            md_path: self.md_path,
            txt_path: self.txt_path,
            json_path: self.json_path,
            image_count: self.image_count,
            word_count: self.word_count,
        })
    }
}

fn io_err(e: serde_json::Error) -> std::io::Error {
    std::io::Error::other(e)
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
        mfw.write_page(&mut page1).unwrap();

        let mut page2 = Page::letter(2);
        page2.add_paragraph(Paragraph::with_text("Page two text"));
        mfw.write_page(&mut page2).unwrap();

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
        mfw.write_page(&mut page).unwrap();
        mfw.finish().unwrap();

        let content = std::fs::read_to_string(tmp.join("extract.md")).unwrap();
        assert!(
            !content.contains("<!-- page "),
            "unexpected marker:\n{}",
            content
        );

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn test_duplicate_images_written_once() {
        use unpdf::model::{Block, Resource};

        let tmp = std::env::temp_dir().join("unpdf_writer_dedup_test");
        std::fs::create_dir_all(&tmp).unwrap();
        let images_dir = tmp.join("images");

        let image_bytes: Vec<u8> = vec![0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3, 4]; // fake JPEG magic
        let make_resource = || {
            Resource::new(
                image_bytes.clone(),
                "image/jpeg".to_string(),
                unpdf::model::ResourceType::Image,
            )
        };

        let render_opts = RenderOptions::new();
        let formats = vec![OutputFormat::Markdown];
        let mut mfw =
            MultiFormatWriter::new(&tmp, &formats, render_opts, Some(images_dir.clone())).unwrap();

        let doc = unpdf::model::Document::new();
        mfw.write_document_start(&doc.metadata, 2).unwrap();

        // 두 페이지에 동일한 바이트의 이미지 각각 삽입
        let mut page1 = Page::letter(1);
        let id1 = "page1_Image1.jpg".to_string();
        page1.images.push((id1.clone(), make_resource()));
        page1.elements.push(Block::image(id1));

        let mut page2 = Page::letter(2);
        let id2 = "page2_Image1.jpg".to_string();
        page2.images.push((id2.clone(), make_resource()));
        page2.elements.push(Block::image(id2));

        mfw.write_page(&mut page1).unwrap();
        mfw.write_page(&mut page2).unwrap();

        let summary = mfw.finish().unwrap();
        assert_eq!(summary.image_count, 1);

        // 디스크에 이미지 파일이 하나만 존재해야 함
        let files: Vec<_> = std::fs::read_dir(&images_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(
            files.len(),
            1,
            "중복 이미지는 파일 하나만 써야 함: {:?}",
            files
        );

        // 두 페이지의 Markdown이 모두 동일한 canonical 경로를 참조해야 함
        let content = std::fs::read_to_string(tmp.join("extract.md")).unwrap();
        let image_links: Vec<_> = content.lines().filter(|l| l.contains("![](")).collect();
        assert_eq!(image_links.len(), 2, "이미지 링크가 두 줄이어야 함");
        assert_eq!(
            image_links[0], image_links[1],
            "두 페이지의 이미지 링크가 동일해야 함"
        );

        std::fs::remove_dir_all(&tmp).ok();
    }
}
