//! CLI smoke tests — spawn the binary and verify output file layout.

use std::path::Path;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_unpdf")
}

fn fixture() -> std::path::PathBuf {
    // CARGO_MANIFEST_DIR is the cli/ directory; fixture lives at repo root/tests/fixtures/
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/arxiv-2502.21142.pdf")
}

#[test]
fn convert_default_outputs_md_only() {
    let fixture = fixture();
    if !fixture.exists() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    let status = Command::new(bin())
        .args([
            "convert",
            fixture.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--quiet",
        ])
        .status()
        .unwrap();
    assert!(status.success());
    assert!(out.join("extract.md").exists(), "extract.md missing");
    assert!(
        !out.join("extract.txt").exists(),
        "txt should not exist by default"
    );
    assert!(
        !out.join("content.json").exists(),
        "json should not exist by default"
    );
}

#[test]
fn convert_all_flag_produces_three_files() {
    let fixture = fixture();
    if !fixture.exists() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    let status = Command::new(bin())
        .args([
            "convert",
            fixture.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--all",
            "--quiet",
        ])
        .status()
        .unwrap();
    assert!(status.success());
    assert!(out.join("extract.md").exists());
    assert!(out.join("extract.txt").exists());
    assert!(out.join("content.json").exists());
}

#[test]
fn convert_formats_flag_selects_subset() {
    let fixture = fixture();
    if !fixture.exists() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    let status = Command::new(bin())
        .args([
            "convert",
            fixture.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--formats",
            "md,json",
            "--quiet",
        ])
        .status()
        .unwrap();
    assert!(status.success());
    assert!(out.join("extract.md").exists());
    assert!(out.join("content.json").exists());
    assert!(!out.join("extract.txt").exists());
}
