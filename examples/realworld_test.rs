use std::fs;
use std::path::Path;

fn main() {
    let dir = Path::new("test-files/realworld");
    let mut results = Vec::new();

    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().map_or(false, |e| e == "pdf") {
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

            match unpdf::parse_file(&path) {
                Ok(doc) => {
                    let page_count = doc.page_count();
                    let opts = unpdf::render::RenderOptions::default();
                    let md = unpdf::render::to_markdown(&doc, &opts).unwrap_or_default();
                    let text_len = md.len();
                    let has_outline = doc.outline.is_some();
                    let resource_count = doc.resources.len();

                    results.push((
                        name,
                        size,
                        "OK".to_string(),
                        page_count,
                        text_len,
                        has_outline,
                        resource_count,
                    ));
                }
                Err(e) => {
                    results.push((name, size, format!("ERR: {}", e), 0, 0, false, 0));
                }
            }
        }
    }

    results.sort_by(|a, b| a.0.cmp(&b.0));

    println!(
        "{:<35} {:>8} {:>6} {:>8} {:>6} {:>5} {}",
        "File", "Size", "Pages", "TextLen", "Outln", "Imgs", "Status"
    );
    println!("{}", "-".repeat(95));
    for (name, size, status, pages, text_len, outline, imgs) in &results {
        let size_str = if *size > 1_000_000 {
            format!("{:.1}MB", *size as f64 / 1_000_000.0)
        } else {
            format!("{}KB", size / 1024)
        };
        println!(
            "{:<35} {:>8} {:>6} {:>8} {:>6} {:>5} {}",
            name,
            size_str,
            pages,
            text_len,
            if *outline { "Y" } else { "N" },
            imgs,
            status
        );
    }

    let ok_count = results.iter().filter(|r| r.2 == "OK").count();
    let err_count = results.len() - ok_count;
    let zero_text = results
        .iter()
        .filter(|r| r.2 == "OK" && r.4 == 0)
        .count();
    println!(
        "\n--- Summary ---\nTotal: {}, OK: {}, Errors: {}, Zero-text: {}",
        results.len(),
        ok_count,
        err_count,
        zero_text
    );
}
