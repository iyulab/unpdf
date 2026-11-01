use std::ops::Range;

use super::backend::{DecodedRun, DecodedText, GlyphAdvance, TextCluster};
use super::sanitize::{is_non_text_control, sanitize_extracted_text};

const MAX_CLUSTERS: usize = 65_536;

#[derive(Debug)]
pub(crate) struct SourceText {
    pub source: Range<usize>,
    pub text: Range<usize>,
}

pub(crate) fn append(
    output: &mut String,
    source: Range<usize>,
    text: &str,
    mapping: &mut Option<Vec<SourceText>>,
) {
    let start = output.len();
    output.push_str(text);
    record(mapping, source, start..output.len());
}

pub(crate) fn record(
    mapping: &mut Option<Vec<SourceText>>,
    source: Range<usize>,
    text: Range<usize>,
) {
    if mapping.as_ref().is_some_and(|m| m.len() >= MAX_CLUSTERS) {
        *mapping = None;
    }
    if let Some(mapping) = mapping {
        mapping.push(SourceText { source, text });
    }
}

pub(crate) fn finish(
    mut decoded: DecodedText,
    mapping: Option<Vec<SourceText>>,
    measured: Option<Vec<(Range<usize>, GlyphAdvance)>>,
    source_len: usize,
) -> DecodedRun {
    let glyphs = measured
        .as_ref()
        .map(|m| m.iter().map(|(_, g)| *g).collect());
    let mapped = mapping
        .as_deref()
        .zip(measured.as_deref())
        .and_then(|(mapping, measured)| clusters(&decoded.text, mapping, measured, source_len));
    let clusters = if let Some((text, clusters)) = mapped {
        decoded.text = text;
        Some(clusters)
    } else {
        decoded.text = sanitize_extracted_text(decoded.text);
        None
    };
    DecodedRun {
        decoded,
        glyphs,
        clusters,
    }
}

fn clusters(
    text: &str,
    mapping: &[SourceText],
    measured: &[(Range<usize>, GlyphAdvance)],
    source_len: usize,
) -> Option<(String, Vec<TextCluster>)> {
    let mut source_at = 0;
    let mut glyph_at = 0;
    let mut text_at = 0;
    let mut output = String::with_capacity(text.len());
    let mut clusters = Vec::with_capacity(mapping.len());
    for part in mapping {
        if part.source.start != source_at || part.text.start != text_at {
            return None;
        }
        let first = glyph_at;
        while source_at < part.source.end {
            let (range, _) = measured.get(glyph_at)?;
            if range.start != source_at || range.end > part.source.end || range.is_empty() {
                return None;
            }
            source_at = range.end;
            glyph_at += 1;
        }
        let start = output.len();
        output.extend(
            text.get(part.text.clone())?
                .chars()
                .filter(|&c| !is_non_text_control(c)),
        );
        text_at = part.text.end;
        clusters.push(TextCluster {
            text: start..output.len(),
            glyphs: first..glyph_at,
        });
    }
    (source_at == source_len && glyph_at == measured.len() && text_at == text.len())
        .then_some((output, clusters))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glyph() -> GlyphAdvance {
        GlyphAdvance {
            width: 500.0,
            vertical: None,
            is_word_space: false,
        }
    }

    #[test]
    fn sanitization_keeps_empty_cluster_advances_and_decode_diagnostics() {
        let decoded = DecodedText {
            text: "A\0B".into(),
            undecoded_codes: 2,
            ..Default::default()
        };
        let mapping = (0..3)
            .map(|i| SourceText {
                source: i..i + 1,
                text: i..i + 1,
            })
            .collect();
        let measured = (0..3).map(|i| (i..i + 1, glyph())).collect();
        let run = finish(decoded, Some(mapping), Some(measured), 3);
        assert_eq!(run.decoded.text, "AB");
        assert_eq!(run.decoded.undecoded_codes, 2);
        let clusters = run.clusters.unwrap();
        assert_eq!(clusters[1].text, 1..1);
        assert_eq!(clusters[1].glyphs, 1..2);
        assert_eq!(clusters[2].text, 1..2);
        assert_eq!(clusters[2].glyphs, 2..3);
    }

    #[test]
    fn mismatched_source_endpoints_or_utf8_boundaries_keep_the_whole_decoded_text() {
        for part in [
            SourceText {
                source: 0..1,
                text: 0..2,
            },
            SourceText {
                source: 1..2,
                text: 0..4,
            },
        ] {
            let run = finish(
                DecodedText::text("😀".into()),
                Some(vec![part]),
                Some(vec![(0..2, glyph())]),
                2,
            );
            assert_eq!(run.decoded.text, "😀");
            assert_eq!(run.glyphs.unwrap(), [glyph()]);
            assert!(run.clusters.is_none());
        }
    }

    #[test]
    fn the_cluster_cap_preserves_text_at_and_beyond_its_boundary() {
        for count in [MAX_CLUSTERS, MAX_CLUSTERS + 1] {
            let mut text = String::new();
            let mut mapping = Some(Vec::new());
            for i in 0..count {
                append(&mut text, i..i + 1, "a", &mut mapping);
            }
            assert_eq!(text.len(), count);
            if count == MAX_CLUSTERS {
                let measured = (0..count).map(|i| (i..i + 1, glyph())).collect();
                let run = finish(DecodedText::text(text), mapping, Some(measured), count);
                assert_eq!(run.clusters.unwrap().len(), count);
            } else {
                assert!(mapping.is_none());
            }
        }
    }
}
