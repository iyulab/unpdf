//! 스트리밍 파싱 경로(병렬)가 순차 경로와 구조적으로 동등한 결과를 내는지 검증.
//!
//! **주의**: byte-단위 엄격한 동치성은 현재 보장되지 않는다.
//! 레이아웃/테이블 감지 단계에서 HashMap 반복 순서에 의한 pre-existing
//! non-determinism 이 있어, 같은 입력으로도 실행마다 수 바이트 수준의
//! 출력 변동이 발생한다. 이 비결정성은 이 브랜치 이전부터 존재했고
//! 별도 이슈로 추적한다: `claudedocs/issues/ISSUE-unpdf-20260414-layout-nondeterminism.md`.
//!
//! 여기서는 병렬 경로가 순차 경로 대비 **거시적 동등성** (페이지 수,
//! 페이지 번호 순서, 페이지별 텍스트 길이의 근사 일치) 을 유지하는지를
//! 검증한다 — 본 브랜치에서 추가된 rayon 병렬화가 출력의 재정렬이나
//! 페이지 유실을 일으키지 않음을 보장한다.

use std::path::{Path, PathBuf};
use unpdf::{parse_file_with_options, ParseOptions};

fn fixtures() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    if !dir.exists() {
        return vec![];
    }
    std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| {
            let p = e.ok()?.path();
            if p.extension()?.to_str()? == "pdf" {
                Some(p)
            } else {
                None
            }
        })
        .collect()
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
    for pdf in fixtures() {
        let seq = parse_file_with_options(&pdf, ParseOptions::new().with_parallel(false))
            .expect("seq parse");
        let par = parse_file_with_options(&pdf, ParseOptions::new().with_parallel(true))
            .expect("par parse");

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
    for pdf in fixtures() {
        let seq = parse_file_with_options(&pdf, ParseOptions::new().with_parallel(false)).unwrap();
        let par = parse_file_with_options(&pdf, ParseOptions::new().with_parallel(true)).unwrap();

        for (sp, pp) in seq.pages.iter().zip(par.pages.iter()) {
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
