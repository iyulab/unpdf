//! CLI smoke tests — spawn the binary and verify output file layout.

use std::path::Path;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_unpdf")
}

/// A single-page PDF built here rather than read from disk, so every test in
/// this file runs everywhere. Three of them used to read
/// `tests/fixtures/arxiv-2502.21142.pdf` behind an `if !exists { return; }`
/// guard — a file never committed to the repo, so those three reported green
/// without spawning the binary even once.
fn minimal_pdf() -> Vec<u8> {
    let stream = b"BT /F1 12 Tf 20 100 Td (Hello unpdf) Tj ET";
    let bodies: Vec<Vec<u8>> = vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]/Contents 4 0 R\
           /Resources<</Font<</F1 5 0 R>>>>>>"
            .to_vec(),
        {
            let mut v = format!("<</Length {}>>stream\n", stream.len()).into_bytes();
            v.extend_from_slice(stream);
            v.extend_from_slice(b"\nendstream");
            v
        },
        b"<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>".to_vec(),
    ];

    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in bodies.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"endobj\n");
    }

    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n", bodies.len() + 1).as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for offset in &offsets {
        out.extend_from_slice(format!("{:010} 00000 n \n", offset).as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer<</Root 1 0 R/Size {}>>\nstartxref\n{}\n%%EOF\n",
            bodies.len() + 1,
            xref
        )
        .as_bytes(),
    );
    out
}

/// Binds and immediately drops a listener, so the port is known to be free and
/// a connection to it is refused rather than left hanging.
fn closed_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

#[test]
fn convert_default_outputs_md_only() {
    let tmp = tempfile::tempdir().unwrap();
    let fixture = tmp.path().join("input.pdf");
    std::fs::write(&fixture, minimal_pdf()).unwrap();
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
    let tmp = tempfile::tempdir().unwrap();
    let fixture = tmp.path().join("input.pdf");
    std::fs::write(&fixture, minimal_pdf()).unwrap();
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
    let tmp = tempfile::tempdir().unwrap();
    let fixture = tmp.path().join("input.pdf");
    std::fs::write(&fixture, minimal_pdf()).unwrap();
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

/// `convert` streams pages, which never assembles the whole document the AI
/// passes need, so configuring AI switches it to a buffered parse. That switch
/// must not change anything else about the output: same files, same bytes.
///
/// The endpoint here is deliberately closed — every AI call fails and falls
/// back, which is what isolates the path change from the model's own output.
#[test]
fn convert_with_ai_configured_produces_the_same_output_as_streaming() {
    let tmp = tempfile::tempdir().unwrap();
    let pdf = tmp.path().join("input.pdf");
    std::fs::write(&pdf, minimal_pdf()).unwrap();

    let run = |out: &Path, extra: &[&str]| {
        let mut cmd = Command::new(bin());
        cmd.args([
            "convert",
            pdf.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--all",
            "--quiet",
        ]);
        cmd.args(extra);
        let status = cmd.status().unwrap();
        assert!(status.success(), "convert failed with {extra:?}");
    };

    let streamed = tmp.path().join("streamed");
    run(&streamed, &[]);

    let base_url = format!("http://127.0.0.1:{}", closed_port());
    let buffered = tmp.path().join("buffered");
    run(
        &buffered,
        &[
            "--ai-base-url",
            &base_url,
            "--ai-api-key",
            "unused",
            "--ai-model",
            "unused",
        ],
    );

    for name in ["extract.md", "extract.txt", "content.json"] {
        let a =
            std::fs::read(streamed.join(name)).unwrap_or_else(|e| panic!("streaming {name}: {e}"));
        let b =
            std::fs::read(buffered.join(name)).unwrap_or_else(|e| panic!("buffered {name}: {e}"));
        assert_eq!(
            a, b,
            "{name} differs between the streaming and buffered paths"
        );
    }
}

/// Supplying only part of the AI configuration is rejected up front, on every
/// command that accepts the flags — a half-configured endpoint is a typo, and
/// silently skipping the pass would look like the model found nothing to say.
#[test]
fn partial_ai_configuration_is_rejected_on_every_command() {
    let tmp = tempfile::tempdir().unwrap();
    let pdf = tmp.path().join("input.pdf");
    std::fs::write(&pdf, minimal_pdf()).unwrap();

    for command in ["convert", "markdown", "text", "json"] {
        let output = Command::new(bin())
            .args([
                command,
                pdf.to_str().unwrap(),
                "--ai-model",
                "only-this-one",
            ])
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "{command} accepted a half-supplied AI configuration"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("--ai-base-url") && stderr.contains("--ai-api-key"),
            "{command} did not name the missing flags: {stderr}"
        );
    }
}
