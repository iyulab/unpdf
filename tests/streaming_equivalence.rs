//! 스트리밍 파싱 경로(병렬)가 순차 경로와 구조적으로 동등한 결과를 내는지 검증.
//!
//! **주의**: byte-단위 엄격한 동치성은 현재 보장되지 않는다.
//! 레이아웃/테이블 감지 단계에서 HashMap 반복 순서에 의한 pre-existing
//! non-determinism 이 있어, 같은 입력으로도 실행마다 수 바이트 수준의
//! 출력 변동이 발생한다. 이 비결정성은 병렬 경로가 도입되기 전부터 존재했고
//! 별도로 추적한다.
//!
//! 여기서는 병렬 경로가 순차 경로 대비 **거시적 동등성** (페이지 수,
//! 페이지 번호 순서, 페이지별 텍스트 길이의 근사 일치) 을 유지하는지를
//! 검증한다 — 본 브랜치에서 추가된 rayon 병렬화가 출력의 재정렬이나
//! 페이지 유실을 일으킴이 없음을 보장한다.
//!
//! **코퍼스는 여기서 합성한다.** 종전에는 `tests/fixtures/*.pdf` 를 읽고
//! 디렉터리가 없으면 빈 `Vec` 을 돌려줬는데, 그 디렉터리는 한 번도 커밋된
//! 적이 없어 두 테스트가 **공집합 위를 돌며 단언 0 회로 초록**이었다.
//! 병렬화가 페이지를 뒤섞어도 아무도 알 수 없었다. 이제 다중 페이지 PDF 를
//! 직접 조립하므로 어디서나 실제로 돌고, 픽스처 부재는 **skip 이 아니라
//! 실패**다(`assert!(!corpus.is_empty())`).

use std::path::PathBuf;
use unpdf::{parse_file_with_options, ParseOptions};

/// 다중 페이지 PDF 를 조립한다. 페이지마다 큰 글꼴 제목 · 본문 · 우측 정렬
/// 연도를 두어 heading 분류와 XY-Cut 의 수평 분할 경로까지 지나가게 한다.
fn multi_page_pdf(page_count: usize) -> Vec<u8> {
    assert!(page_count > 0);
    // 1 = Catalog, 2 = Pages, 3+2i = Page i, 4+2i = Contents i, 3+2N = Font
    let font_obj = 3 + 2 * page_count;

    let kids: Vec<String> = (0..page_count)
        .map(|i| format!("{} 0 R", 3 + 2 * i))
        .collect();

    let mut bodies: Vec<Vec<u8>> = vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        format!(
            "<</Type/Pages/Kids[{}]/Count {}>>",
            kids.join(" "),
            page_count
        )
        .into_bytes(),
    ];

    for i in 0..page_count {
        bodies.push(
            format!(
                "<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]/Contents {} 0 R\
                 /Resources<</Font<</F1 {} 0 R>>>>>>",
                4 + 2 * i,
                font_obj
            )
            .into_bytes(),
        );

        let stream = format!(
            "BT /F1 18 Tf 20 170 Td (Section {i}) Tj ET\n\
             BT /F1 10 Tf 20 150 Td (Body text for page {i} with several words) Tj ET\n\
             BT /F1 10 Tf 140 150 Td (2026) Tj ET"
        );
        let mut obj = format!("<</Length {}>>stream\n", stream.len()).into_bytes();
        obj.extend_from_slice(stream.as_bytes());
        obj.extend_from_slice(b"\nendstream");
        bodies.push(obj);
    }

    bodies.push(b"<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>".to_vec());
    debug_assert_eq!(bodies.len(), font_obj);

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

/// 합성 코퍼스. `TempDir` 를 함께 돌려주며, 그것이 드롭되면 파일이 사라지므로
/// 호출자는 반드시 살려둔다. 실제 문서 코퍼스를 들이려면 여기 경로를 더하되,
/// **부재 시 건너뛰는 분기를 두지 않는다** — 그것이 이 파일이 원래 가졌던 결함이다.
fn corpus() -> (tempfile::TempDir, Vec<(PathBuf, usize)>) {
    let dir = tempfile::tempdir().unwrap();
    let mut paths = Vec::new();

    // 1 페이지는 병렬 분할이 일어나지 않아 순서 회귀를 못 잡는다 — 여러 장이 필요하다.
    for pages in [3usize, 7] {
        let path = dir.path().join(format!("synthetic-{pages}p.pdf"));
        std::fs::write(&path, multi_page_pdf(pages)).unwrap();
        paths.push((path, pages));
    }

    assert!(
        !paths.is_empty(),
        "corpus must never be empty — an empty corpus makes every assertion below vacuous"
    );
    (dir, paths)
}

/// 페이지 텍스트 길이가 ± 허용 오차 내에서 일치하는지. 본질 텍스트는 같으나
/// pre-existing non-determinism 으로 인한 소수 문자의 차이만 허용한다.
fn lengths_within_tolerance(seq_len: usize, par_len: usize) -> bool {
    let diff = seq_len.abs_diff(par_len);
    let base = seq_len.max(par_len).max(1);
    // 페이지 전체 대비 1% 이하의 길이 차이만 허용
    diff * 100 <= base
}

#[test]
fn parallel_preserves_page_count_and_order() {
    let (_dir, corpus) = corpus();
    for (pdf, expected_pages) in corpus {
        let seq = parse_file_with_options(&pdf, ParseOptions::new().with_parallel(false))
            .expect("seq parse");
        let par = parse_file_with_options(&pdf, ParseOptions::new().with_parallel(true))
            .expect("par parse");

        // 「두 경로가 같다」는 둘 다 0 페이지여도 참이다 — 절대 기대치를 먼저 잠근다.
        assert_eq!(
            seq.page_count() as usize,
            expected_pages,
            "fixture {} should yield {} pages",
            pdf.display(),
            expected_pages
        );

        assert_eq!(
            seq.page_count(),
            par.page_count(),
            "page count mismatch for {}",
            pdf.display()
        );

        // 페이지 번호는 엄격 ASC 순서로 동일해야 한다.
        let seq_nums: Vec<u32> = seq.pages.iter().map(|p| p.number).collect();
        let par_nums: Vec<u32> = par.pages.iter().map(|p| p.number).collect();
        assert_eq!(
            seq_nums,
            par_nums,
            "page ordering mismatch for {}",
            pdf.display()
        );

        // ASC 보장
        assert!(
            par_nums.windows(2).all(|w| w[0] < w[1]),
            "parallel output not in ASC page_num order: {:?}",
            par_nums
        );
    }
}

#[test]
fn parallel_preserves_page_text_lengths_within_tolerance() {
    let (_dir, corpus) = corpus();
    for (pdf, expected_pages) in corpus {
        let seq = parse_file_with_options(&pdf, ParseOptions::new().with_parallel(false)).unwrap();
        let par = parse_file_with_options(&pdf, ParseOptions::new().with_parallel(true)).unwrap();
        assert_eq!(seq.pages.len(), expected_pages, "{}", pdf.display());

        for (sp, pp) in seq.pages.iter().zip(par.pages.iter()) {
            // 빈 텍스트끼리도 「허용 오차 내」라 통과한다 — 내용이 있었음을 먼저 잠근다.
            assert!(
                !sp.plain_text().trim().is_empty(),
                "page {} of {} extracted no text",
                sp.number,
                pdf.display()
            );
            let s_len = sp.plain_text().len();
            let p_len = pp.plain_text().len();
            assert!(
                lengths_within_tolerance(s_len, p_len),
                "page {} text length diverges: seq={} par={} (fixture: {})",
                sp.number,
                s_len,
                p_len,
                pdf.display()
            );
        }
    }
}
