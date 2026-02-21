//! Benchmarks for unpdf parsing performance.
//!
//! Run with: cargo bench
//!
//! These benchmarks test parsing performance with synthetic PDF data.

use criterion::{black_box, criterion_group, criterion_main, Criterion};

/// Creates a minimal synthetic PDF with the given number of pages.
fn create_test_pdf(page_count: usize) -> Vec<u8> {
    let mut content = String::new();

    // PDF header
    content.push_str("%PDF-1.4\n");

    // Object 1: Catalog
    content.push_str("1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

    // Object 2: Pages
    let kids: Vec<String> = (0..page_count).map(|i| format!("{} 0 R", i + 3)).collect();
    content.push_str(&format!(
        "2 0 obj\n<< /Type /Pages /Kids [{}] /Count {} >>\nendobj\n",
        kids.join(" "),
        page_count
    ));

    // Page objects and content
    let mut next_obj = 3;
    for i in 0..page_count {
        let page_obj = next_obj;
        let content_obj = next_obj + 1;
        next_obj += 2;

        // Page object
        content.push_str(&format!(
            "{} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents {} 0 R >>\nendobj\n",
            page_obj, content_obj
        ));

        // Content stream
        let text = format!(
            "BT /F1 12 Tf 100 700 Td (Page {} - Benchmark test content for unpdf performance measurement.) Tj ET",
            i + 1
        );
        content.push_str(&format!(
            "{} 0 obj\n<< /Length {} >>\nstream\n{}\nendstream\nendobj\n",
            content_obj,
            text.len(),
            text
        ));
    }

    // Cross-reference table (simplified)
    let xref_offset = content.len();
    content.push_str(&format!("xref\n0 {}\n", next_obj));
    content.push_str("0000000000 65535 f \n");
    // Simplified: just write placeholder offsets
    for _ in 1..next_obj {
        content.push_str("0000000000 00000 n \n");
    }

    // Trailer
    content.push_str(&format!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
        next_obj, xref_offset
    ));

    content.into_bytes()
}

/// Benchmark PDF format detection.
fn bench_format_detection(c: &mut Criterion) {
    let pdf_data = create_test_pdf(1);
    let non_pdf_data = b"Not a PDF file at all, just random text content";

    c.bench_function("detect_valid_pdf", |b| {
        b.iter(|| unpdf::detect_format_from_bytes(black_box(&pdf_data)).unwrap());
    });

    c.bench_function("detect_non_pdf", |b| {
        b.iter(|| unpdf::detect_format_from_bytes(black_box(non_pdf_data)).is_err());
    });
}

/// Benchmark PDF parsing at various sizes.
fn bench_pdf_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("pdf_parsing");

    for page_count in [1, 5, 10].iter() {
        let data = create_test_pdf(*page_count);

        group.bench_function(format!("{}_pages", page_count), |b| {
            b.iter(|| {
                // Use lenient mode since synthetic PDFs may not be fully valid
                let options = unpdf::ParseOptions::new().lenient();
                let _ = unpdf::parse_bytes_with_options(black_box(&data), options);
            });
        });
    }

    group.finish();
}

/// Benchmark builder pattern overhead.
fn bench_builder_creation(c: &mut Criterion) {
    c.bench_function("builder_creation", |b| {
        b.iter(|| {
            let _builder = unpdf::Unpdf::new()
                .lenient()
                .with_frontmatter()
                .with_cleanup(unpdf::CleanupPreset::Standard);
        });
    });
}

criterion_group!(
    benches,
    bench_format_detection,
    bench_pdf_parsing,
    bench_builder_creation,
);
criterion_main!(benches);
