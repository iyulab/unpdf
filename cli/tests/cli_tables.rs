//! `unpdf tables` writes each table the document holds as CSV: to standard output, nothing
//! else on it, or one file per table named for its page and place.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_unpdf")
}

/// Wraps 1-indexed object bodies in a header, a cross-reference table and a trailer.
fn assemble(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for offset in &offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<</Size {}/Root 1 0 R>>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

/// One page with a ruled 2x2 table: `Name | Age` over `Alice, B. | 30`.
fn table_pdf() -> Vec<u8> {
    let content = b"2 w \
        70 660 m 70 700 l S \
        170 660 m 170 700 l S \
        270 660 m 270 700 l S \
        70 700 m 270 700 l S \
        70 680 m 270 680 l S \
        70 660 m 270 660 l S \
        BT /F1 12 Tf 80 690 Td (Name) Tj ET \
        BT /F1 12 Tf 180 690 Td (Age) Tj ET \
        BT /F1 12 Tf 80 670 Td (Alice, B.) Tj ET \
        BT /F1 12 Tf 180 670 Td (30) Tj ET\n";
    let mut stream = format!("<</Length {}>>\nstream\n", content.len()).into_bytes();
    stream.extend_from_slice(content);
    stream.extend_from_slice(b"\nendstream");
    assemble(&[
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]\
          /Resources<</Font<</F1 5 0 R>>>>/Contents 4 0 R>>"
            .to_vec(),
        stream,
        b"<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>".to_vec(),
    ])
}

fn write_pdf(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("table.pdf");
    std::fs::write(&path, table_pdf()).unwrap();
    path
}

#[test]
fn tables_go_to_standard_output_as_csv_and_nothing_else() {
    let dir = tempfile::tempdir().unwrap();
    let pdf = write_pdf(dir.path());
    let out = Command::new(bin())
        .args(["tables"])
        .arg(&pdf)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "Name,Age\r\n\"Alice, B.\",30\r\n"
    );
}

#[test]
fn each_table_is_a_file_named_for_its_page_and_place() {
    let dir = tempfile::tempdir().unwrap();
    let pdf = write_pdf(dir.path());
    let out_dir = dir.path().join("tables");
    let out = Command::new(bin())
        .args(["tables", "--tsv", "-o"])
        .arg(&out_dir)
        .arg(&pdf)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let written = std::fs::read_to_string(out_dir.join("p1-t1.tsv")).unwrap();
    assert_eq!(written, "Name\tAge\r\nAlice, B.\t30\r\n");
}
