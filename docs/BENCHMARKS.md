<!-- Generated from benchmark results; edit the generator, not this file. -->

# Benchmarks

Measured on **unpdf 0.36.0** (the published package) on 2026-10-09. Scores come from each benchmark's own evaluator; nothing here is re-implemented.

## opendataloader-bench

[opendataloader-bench](https://github.com/opendataloader-project/opendataloader-bench): 200 PDFs with Markdown ground truth (Apache-2.0). NID = reading-order text similarity, TEDS = table structure, MHS = heading structure. Each is averaged over the documents it applies to (last column): TEDS only over documents whose ground truth has a table, MHS only over those with headings. A document's overall score is the mean of the metrics that apply to it, and Overall is that score averaged over all documents -- so it is not the mean of the three rows below it.

| Metric | Score | Documents scored |
|---|---|---|
| Overall | **0.863** | 200 |
| NID | 0.910 | 200 |
| TEDS | 0.755 | 42 |
| MHS | 0.767 | 107 |

### Compared with other engines (rank 9 of 20)

Other engines' scores are the runs the benchmark repository itself publishes, cited as-is with their version and date, except the rows marked "measured here". Several of them run vision models on a GPU; this library reads the PDF text layer on a CPU.

Rows marked "measured here" were converted on a CPU with each engine's Python API, as called here (docling `DocumentConverter().convert(path).document.export_to_markdown()`, liteparse `LiteParse(output_format="markdown", ocr_enabled=False).parse(path).text`, pdf-inspector `pdf_inspector.process_pdf(path).markdown`, pymupdf4llm `pymupdf4llm.to_markdown(path)`, markitdown `MarkItDown().convert(path).text_content`; default options except OCR, which is off, since this page compares reading the text layer; a document that fails to convert is scored as empty output) and scored with the same evaluator and dataset revision as this page.

| Engine | Version | Overall | NID | TEDS | MHS | Measured |
|---|---|---|---|---|---|---|
| opendataloader-hybrid | 2.2.1 | 0.907 | 0.934 | 0.928 | 0.821 | 2026-04-06 |
| docling | 2.135.0 | 0.891 | 0.905 | 0.925 | 0.830 | 2026-10-08 (measured here) |
| nutrient | 1.0.1 | 0.885 | 0.925 | 0.708 | 0.819 | 2026-04-30 |
| liteparse | 2.15.1 | 0.883 | 0.912 | 0.818 | 0.821 | 2026-10-09 (measured here) |
| docling | 2.84.0 | 0.882 | 0.898 | 0.887 | 0.824 | 2026-04-06 |
| opendataloader-hybrid-hydrogen | 2.2.1 | 0.877 | 0.926 | 0.796 | 0.769 | 2026-04-08 |
| pdf-inspector | 1.25.2 | 0.870 | 0.911 | 0.800 | 0.777 | 2026-10-09 (measured here) |
| pymupdf4llm * | 1.28.2 | 0.869 | 0.907 | 0.790 | 0.783 | 2026-10-08 (measured here) |
| **unpdf** | 0.36.0 | 0.863 | 0.910 | 0.755 | 0.767 | 2026-10-09 |
| marker | 1.10.1 | 0.861 | 0.890 | 0.808 | 0.796 | 2026-01-06 |
| opendataloader-hybrid-helium | 0.2.0-SNAPSHOT | 0.845 | 0.879 | 0.807 | 0.755 | 2026-04-17 |
| unstructured-hires | 0.17.2 | 0.841 | 0.904 | 0.588 | 0.749 | 2026-04-06 |
| edgeparse | 0.3.0 | 0.837 | 0.894 | 0.717 | 0.706 | 2026-04-06 |
| mineru | 2.7.0 | 0.831 | 0.857 | 0.873 | 0.743 | 2026-01-06 |
| opendataloader | 2.2.1 | 0.831 | 0.902 | 0.489 | 0.739 | 2026-04-06 |
| pymupdf4llm | 0.2.0 | 0.732 | 0.885 | 0.401 | 0.412 | 2025-11-27 |
| unstructured | 0.17.2 | 0.686 | 0.882 | 0.000 | 0.388 | 2026-04-06 |
| markitdown | 0.1.5 | 0.589 | 0.844 | 0.273 | 0.000 | 2026-04-06 |
| markitdown | 0.1.8 | 0.589 | 0.844 | 0.273 | 0.000 | 2026-10-08 (measured here) |
| liteparse | 1.2.1 | 0.576 | 0.866 | 0.000 | 0.000 | 2026-04-06 |

\* Runs a learned layout model on the CPU (pymupdf4llm 1.28 with pymupdf_layout, an ONNX model), not the text layer alone.

Reproduce:

```bash
# Python 3.13 or newer (what the evaluator declares)
git clone https://github.com/opendataloader-project/opendataloader-bench && cd opendataloader-bench && git checkout 7af1d8f4d0c09f51ea1a5c6ba5f66e993286d109
pip install unpdf-markdown==0.36.0 apted rapidfuzz beautifulsoup4 lxml
python - <<'EOF'
import pathlib, unpdf
out = pathlib.Path('prediction/unpdf/markdown')
out.mkdir(parents=True, exist_ok=True)
for pdf in sorted(pathlib.Path('pdfs').glob('*.pdf')):
    (out / f'{pdf.stem}.md').write_text(unpdf.to_markdown(str(pdf)), encoding='utf-8')
EOF
python src/evaluator.py --prediction-root prediction --engine unpdf
# scores: prediction/unpdf/evaluation.json, under metrics.score
```

## olmOCR-Bench

[olmOCR-Bench](https://huggingface.co/datasets/allenai/olmOCR-bench) (ODC-BY): 1,403 PDF pages checked by unit tests (text present/absent, reading order, tables). Scored with the official [`olmocr.bench`](https://github.com/allenai/olmocr) scorer.

Average of the 8 per-file scores below, as the official scorer reports it: **34.7%**. Read it by file: `arxiv_math` and `old_scans_math` require LaTeX math output, and `old_scans` requires OCR of scanned pages -- both outside what a text-layer extractor does. `baseline` is the benchmark's sanity check that each page's output is not blank, not endlessly repeating, and in the expected character sets.

Some pages outside `old_scans` are scanned images too, with no text layer to read, and a text-layer extractor writes nothing for them. The last column scores only the pages that have one (PyMuPDF finds more than 50 characters of text on the page), with the same tests; the official rate is the one before it.

| Test file | Pass rate | Tests | Pages with a text layer |
|---|---|---|---|
| arxiv_math | 0.6% | 17/2927 | 0.6% (17/2927; 0 on image-only pages) |
| headers_footers | 45.5% | 346/760 | 42.4% (302/712; 41 on image-only pages) |
| long_tiny_text | 26.2% | 116/442 | 44.3% (116/262; 180 on image-only pages) |
| multi_column | 61.0% | 539/884 | 64.1% (539/841; 43 on image-only pages) |
| old_scans | 13.3% | 70/526 | -- |
| old_scans_math | 0.0% | 0/458 | 0.0% (0/204; 254 on image-only pages) |
| table_tests | 43.7% | 447/1022 | 47.1% (446/946; 74 on image-only pages) |
| baseline (the benchmark's sanity check) | 87.3% | 1217/1394 | -- |

Dataset revision `54a96a6fb6a2bd3b297e59869491db4d3625b711`, scorer revision `f7cfe4c22098b154c76b6ec950d1c0a464eecf8d`.

Reproduce:

```bash
# Python 3.11 or newer, on Linux or macOS: the scorer looks outputs up by forward-slash paths,
# so on Windows it finds none and every test fails
git clone https://github.com/allenai/olmocr && git -C olmocr checkout f7cfe4c22098b154c76b6ec950d1c0a464eecf8d
pip install -e "./olmocr[bench]" unpdf-markdown==0.36.0 huggingface_hub
playwright install chromium
python - <<'EOF'
import pathlib, unpdf
from huggingface_hub import snapshot_download
snapshot_download('allenai/olmOCR-bench', repo_type='dataset',
                  revision='54a96a6fb6a2bd3b297e59869491db4d3625b711', local_dir='olmOCR-bench')
data = pathlib.Path('olmOCR-bench/bench_data')
for pdf in sorted((data / 'pdfs').rglob('*.pdf')):
    out = data / 'unpdf' / pdf.parent.name / f'{pdf.stem}_pg1_repeat1.md'
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(unpdf.to_markdown(str(pdf)), encoding='utf-8')
EOF
python -m olmocr.bench.benchmark --dir olmOCR-bench/bench_data --candidate unpdf --bootstrap_samples 200
```

<!-- benchmark-data {"benchmarks": ["odl", "olmocr"], "odl": {"mhs": 0.767, "nid": 0.91, "overall": 0.863, "teds": 0.755}, "olmocr": {"categories": {"arxiv_math": 0.6, "baseline": 87.3, "headers_footers": 45.5, "long_tiny_text": 26.2, "multi_column": 61.0, "old_scans": 13.3, "old_scans_math": 0.0, "table_tests": 43.7}, "mean": 34.7}, "rank": [9, 20], "version": "0.36.0"} -->
