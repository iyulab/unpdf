"""
Tests for unpdf Python bindings.
"""

import os
import pytest

import unpdf


class TestVersion:
    """Tests for version function."""

    def test_version_returns_string(self):
        """Version should return a non-empty string."""
        ver = unpdf.version()
        assert isinstance(ver, str)
        assert len(ver) > 0


class TestIsPdf:
    """Tests for is_pdf function."""

    def test_non_existent_file(self):
        """Non-existent file should return False."""
        assert unpdf.is_pdf("non_existent_file.pdf") is False

    def test_non_pdf_file(self, tmp_path):
        """Non-PDF file should return False."""
        txt_file = tmp_path / "test.txt"
        txt_file.write_text("This is not a PDF")
        assert unpdf.is_pdf(str(txt_file)) is False


class TestGetPageCount:
    """Tests for get_page_count function."""

    def test_non_existent_file(self):
        """Non-existent file should return -1."""
        assert unpdf.get_page_count("non_existent_file.pdf") == -1


class TestToMarkdown:
    """Tests for to_markdown function."""

    def test_non_existent_file_raises(self):
        """Non-existent file should raise RuntimeError."""
        with pytest.raises(RuntimeError):
            unpdf.to_markdown("non_existent_file.pdf")


class TestToText:
    """Tests for to_text function."""

    def test_non_existent_file_raises(self):
        """Non-existent file should raise RuntimeError."""
        with pytest.raises(RuntimeError):
            unpdf.to_text("non_existent_file.pdf")


def _stream_object(dict_str: bytes, data: bytes) -> bytes:
    return dict_str + b"\nstream\n" + data + b"\nendstream"


def _assemble(objects: list[bytes]) -> bytes:
    """Assemble numbered objects into a minimal well-formed PDF."""
    pdf = bytearray(b"%PDF-1.4\n")
    offsets = []
    for idx, body in enumerate(objects):
        offsets.append(len(pdf))
        pdf += f"{idx + 1} 0 obj\n".encode()
        pdf += body
        pdf += b"\nendobj\n"
    xref_start = len(pdf)
    size = len(objects) + 1
    pdf += f"xref\n0 {size}\n0000000000 65535 f \n".encode()
    for offset in offsets:
        pdf += f"{offset:010} 00000 n \n".encode()
    pdf += (
        f"trailer\n<</Size {size}/Root 1 0 R>>\nstartxref\n{xref_start}\n%%EOF\n"
    ).encode()
    return bytes(pdf)


def _text_pdf() -> bytes:
    """One page with a single line of visible Helvetica text."""
    content = b"BT /F1 12 Tf 72 720 Td (Hello World) Tj ET\n"
    return _assemble([
        b"<</Type/Catalog/Pages 2 0 R>>",
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>",
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]"
        b"/Resources<</Font<</F1 5 0 R>>>>/Contents 4 0 R>>",
        _stream_object(b"<</Length %d>>" % len(content), content),
        b"<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>",
    ])


def _image_only_pdf() -> bytes:
    """One page drawn as a single full-page image, no text operators."""
    content = b"q 595 0 0 842 0 0 cm /Im0 Do Q\n"
    return _assemble([
        b"<</Type/Catalog/Pages 2 0 R>>",
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>",
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]"
        b"/Resources<</XObject<</Im0 5 0 R>>>>/Contents 4 0 R>>",
        _stream_object(b"<</Length %d>>" % len(content), content),
        _stream_object(
            b"<</Type/XObject/Subtype/Image/Width 1/Height 1/ColorSpace/DeviceGray"
            b"/BitsPerComponent 8/Length 1>>",
            b"\x80",
        ),
    ])


def _lost_page_pdf() -> bytes:
    """Declares two pages, but its second kid points at an object that is not there.

    The shape a damaged page tree takes: one page survives, one is lost, and the
    parse still succeeds — which is exactly why it has to be reported.
    """
    content = b"BT /F1 12 Tf 72 720 Td (Page one) Tj ET\n"
    return _assemble([
        b"<</Type/Catalog/Pages 2 0 R>>",
        b"<</Type/Pages/Kids[3 0 R 99 0 R]/Count 2>>",
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]"
        b"/Resources<</Font<</F1 5 0 R>>>>/Contents 4 0 R>>",
        _stream_object(b"<</Length %d>>" % len(content), content),
        b"<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>",
    ])


def _suppressed_text_run_pdf() -> bytes:
    """One page whose text uses an Identity-H composite font with no
    ``ToUnicode`` map and no embedded cmap — the decoder has no way to turn its
    CIDs into characters, so the run is discarded and counted as suppressed.

    Mirrors the Rust fixture in ``tests/suppression_reporting_test.rs``.
    """
    content = b"BT /F1 12 Tf 72 720 Td (BC) Tj ET\n"
    return _assemble([
        b"<</Type/Catalog/Pages 2 0 R>>",
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>",
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 612 792]"
        b"/Resources<</Font<</F1 5 0 R>>>>/Contents 4 0 R>>",
        _stream_object(b"<</Length %d>>" % len(content), content),
        b"<</Type/Font/Subtype/Type0/BaseFont/NoMap/Encoding/Identity-H"
        b"/DescendantFonts[6 0 R]>>",
        b"<</Type/Font/Subtype/CIDFontType2/BaseFont/NoMap"
        b"/CIDSystemInfo<</Registry(Adobe)/Ordering(Identity)/Supplement 0>>>>",
    ])


def _jpeg_pdf(width: int, height: int) -> bytes:
    """One page with a single ``DCTDecode``-tagged image XObject of the given size.

    The bytes are not a real decodable JPEG — nothing decodes them — but the
    ``Filter`` entry is what the parser uses to classify a resource as a renderable
    image format, unlike the raw/undecoded pixel buffer ``_image_only_pdf`` produces.
    """
    content = b"q 595 0 0 842 0 0 cm /Im0 Do Q\n"
    return _assemble([
        b"<</Type/Catalog/Pages 2 0 R>>",
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>",
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]"
        b"/Resources<</XObject<</Im0 5 0 R>>>>/Contents 4 0 R>>",
        _stream_object(b"<</Length %d>>" % len(content), content),
        _stream_object(
            b"<</Type/XObject/Subtype/Image/Width %d/Height %d/ColorSpace/DeviceRGB"
            b"/BitsPerComponent 8/Filter/DCTDecode/Length 4>>" % (width, height),
            b"\xff\xd8\xff\xd9",
        ),
    ])


class TestParseOptions:
    """``options`` — the C ABI surface for ``ParseOptions``, previously unreachable
    from this binding (docket #125: no ``ParseOptions`` field at all reached C# or
    Python, both routed through the same option-less C ABI entry points)."""

    def test_no_options_matches_previous_behavior(self):
        info = unpdf.get_info(_jpeg_pdf(100, 100))
        assert info["resource_count"] == 0

    def test_extract_resources_populates_resource_inventory(self):
        info = unpdf.get_info(
            _jpeg_pdf(100, 100), options={"extract_resources": True}
        )
        assert info["resource_count"] == 1

    def test_raw_undecoded_images_never_surfaced(self):
        # The undecoded pixel buffer `_image_only_pdf` produces is a format most
        # GetResourceData callers cannot use, and must never appear regardless of
        # `min_image_dimension` — the shared-filter fix landed in the same cycle
        # this options surface was added, and this is its regression test.
        info = unpdf.get_info(
            _image_only_pdf(),
            options={"extract_resources": True, "min_image_dimension": 0},
        )
        assert info["resource_count"] == 0

    def test_min_image_dimension_drops_small_images_by_default(self):
        info = unpdf.get_info(_jpeg_pdf(10, 10), options={"extract_resources": True})
        assert info["resource_count"] == 0

    def test_min_image_dimension_zero_keeps_small_images(self):
        info = unpdf.get_info(
            _jpeg_pdf(10, 10),
            options={"extract_resources": True, "min_image_dimension": 0},
        )
        assert info["resource_count"] == 1

    def test_malformed_options_json_raises_invalid_argument(self):
        # `options` must be JSON-serializable; a non-serializable value raises in
        # Python before any native call, which is the correct place for it to fail.
        with pytest.raises(TypeError):
            unpdf.get_info(_jpeg_pdf(10, 10), options={"extract_resources": object()})

    def test_options_reaches_file_path_entry_point_too(self, tmp_path):
        path = tmp_path / "jpeg.pdf"
        path.write_bytes(_jpeg_pdf(100, 100))
        info = unpdf.get_info(str(path), options={"extract_resources": True})
        assert info["resource_count"] == 1


class TestResourceAccessors:
    """``get_resource_ids`` / ``get_resource_info`` / ``get_resource_data`` — the
    Python binding declared these native functions in ``_native.py`` but never
    exposed them; ``resource_count`` was reachable but nothing to retrieve the
    resources themselves was (docket #125 triage finding)."""

    def test_no_extract_resources_returns_empty_list(self):
        assert unpdf.get_resource_ids(_jpeg_pdf(100, 100)) == []

    def test_get_resource_ids_lists_extracted_resources(self):
        ids = unpdf.get_resource_ids(
            _jpeg_pdf(100, 100), options={"extract_resources": True}
        )
        assert len(ids) == 1
        assert ids[0].startswith("page1_Im0")

    def test_get_resource_info_reports_metadata(self):
        options = {"extract_resources": True}
        data = _jpeg_pdf(100, 100)
        [resource_id] = unpdf.get_resource_ids(data, options=options)

        info = unpdf.get_resource_info(data, resource_id, options=options)
        assert info["width"] == 100
        assert info["height"] == 100
        assert info["type"] == "image"

    def test_get_resource_data_returns_bytes(self):
        options = {"extract_resources": True}
        data = _jpeg_pdf(100, 100)
        [resource_id] = unpdf.get_resource_ids(data, options=options)

        resource_bytes = unpdf.get_resource_data(data, resource_id, options=options)
        assert resource_bytes == b"\xff\xd8\xff\xd9"

    def test_get_resource_info_unknown_id_raises(self):
        with pytest.raises(RuntimeError) as exc_info:
            unpdf.get_resource_info(
                _jpeg_pdf(100, 100), "nonexistent", options={"extract_resources": True}
            )
        assert exc_info.value.kind == unpdf.ErrorKind.RESOURCE_NOT_FOUND

    def test_get_resource_data_unknown_id_raises(self):
        with pytest.raises(RuntimeError) as exc_info:
            unpdf.get_resource_data(
                _jpeg_pdf(100, 100), "nonexistent", options={"extract_resources": True}
            )
        assert exc_info.value.kind == unpdf.ErrorKind.RESOURCE_NOT_FOUND


class TestInputForms:
    """A PDF may be given as a path, a path-like object, or its own bytes.

    All three must reach the same result: the native library has both a file and a
    bytes entry point, so requiring callers to write PDFs to disk first — or to pass
    ``str(Path(...))`` — would be an artificial limit of this binding alone.
    """

    def test_str_path_pathlike_and_bytes_agree(self, tmp_path):
        pdf_file = tmp_path / "text.pdf"
        data = _text_pdf()
        pdf_file.write_bytes(data)

        expected = unpdf.to_markdown(str(pdf_file))
        assert unpdf.to_markdown(pdf_file) == expected  # pathlib.Path
        assert unpdf.to_markdown(data) == expected  # bytes
        assert unpdf.to_markdown(bytearray(data)) == expected  # bytearray

    def test_bytes_reach_every_entry_point(self):
        data = _text_pdf()
        assert unpdf.get_page_count(data) == 1
        assert unpdf.is_pdf(data) is True
        assert unpdf.get_info(data)["section_count"] == 1
        assert unpdf.get_extraction_quality(data)["char_count"] > 0
        assert unpdf.get_page_stats(data, 1)["page"] == 1
        assert unpdf.to_text(data)
        assert unpdf.to_json(data)

    def test_empty_bytes_is_classified_not_a_crash(self):
        with pytest.raises(unpdf.UnpdfError) as excinfo:
            unpdf.to_markdown(b"")
        assert excinfo.value.kind == unpdf.ErrorKind.INVALID_ARGUMENT

    def test_garbage_bytes_are_classified(self):
        with pytest.raises(unpdf.UnpdfError) as excinfo:
            unpdf.to_markdown(b"not a pdf")
        assert excinfo.value.kind == unpdf.ErrorKind.UNKNOWN_FORMAT

    def test_soft_failure_paths_still_soft_for_bytes(self):
        # These two report failure by return value rather than by raising; bytes input
        # must not turn that into an exception.
        assert unpdf.is_pdf(b"not a pdf") is False
        assert unpdf.get_page_count(b"not a pdf") == -1
        assert unpdf.is_pdf(b"") is False
        assert unpdf.get_page_count(b"") == -1
        assert unpdf.is_pdf("does-not-exist.pdf") is False
        assert unpdf.get_page_count("does-not-exist.pdf") == -1

    @pytest.mark.parametrize("bad", [None, 123, 4.5, ["x.pdf"]])
    def test_wrong_type_raises_rather_than_reporting_a_bad_pdf(self, bad):
        # A wrong-typed argument is a caller bug, not an unparsable PDF. Folding it into
        # the False/-1 return would hide the mistake at the call site.
        with pytest.raises(TypeError):
            unpdf.is_pdf(bad)
        with pytest.raises(TypeError):
            unpdf.get_page_count(bad)
        with pytest.raises(TypeError):
            unpdf.to_markdown(bad)


class TestGetExtractionQuality:
    """Tests for get_extraction_quality function."""

    def test_image_only_pdf_reports_scan(self, tmp_path):
        """Image-only PDF should be flagged as a scan (no text layer)."""
        pdf_file = tmp_path / "scan.pdf"
        pdf_file.write_bytes(_image_only_pdf())
        quality = unpdf.get_extraction_quality(str(pdf_file))
        assert quality["is_scan_pdf"] is True
        assert quality["char_count"] == 0

    def test_text_pdf_reports_text(self, tmp_path):
        """Text PDF should not be flagged as a scan."""
        pdf_file = tmp_path / "text.pdf"
        pdf_file.write_bytes(_text_pdf())
        quality = unpdf.get_extraction_quality(str(pdf_file))
        assert quality["is_scan_pdf"] is False
        assert quality["char_count"] > 0

    def test_intact_pdf_reports_complete(self, tmp_path):
        """An intact document must not claim damage."""
        pdf_file = tmp_path / "text.pdf"
        pdf_file.write_bytes(_text_pdf())
        quality = unpdf.get_extraction_quality(str(pdf_file))
        assert quality["pages_incomplete"] is False
        assert quality["declared_page_count"] == 1
        assert quality["unresolved_page_nodes"] == 0
        assert quality["skipped_object_count"] == 0

    def test_damaged_page_tree_reports_incomplete(self, tmp_path):
        """Silently dropped pages must be observable.

        Parsing succeeds and reports one page, so without these fields the caller
        cannot tell this apart from a genuine one-page document.
        """
        pdf_file = tmp_path / "damaged.pdf"
        pdf_file.write_bytes(_lost_page_pdf())
        quality = unpdf.get_extraction_quality(str(pdf_file))
        assert quality["pages_incomplete"] is True
        assert quality["declared_page_count"] == 2
        assert quality["unresolved_page_nodes"] >= 1

    def test_non_existent_file_raises(self):
        """Non-existent file should raise RuntimeError."""
        with pytest.raises(RuntimeError):
            unpdf.get_extraction_quality("non_existent_file.pdf")


class TestGetPageStats:
    """Tests for get_page_stats function."""

    def test_image_only_page(self, tmp_path):
        """Image-only page: no text ops, at least one image op."""
        pdf_file = tmp_path / "scan.pdf"
        pdf_file.write_bytes(_image_only_pdf())
        stats = unpdf.get_page_stats(str(pdf_file), 1)
        assert stats["text_op_count"] == 0
        assert stats["image_op_count"] >= 1
        assert stats["ocr_text_suppressed"] is False

    def test_text_page(self, tmp_path):
        """Text page: text ops present, no image ops."""
        pdf_file = tmp_path / "text.pdf"
        pdf_file.write_bytes(_text_pdf())
        stats = unpdf.get_page_stats(str(pdf_file), 1)
        assert stats["text_op_count"] >= 1
        assert stats["image_op_count"] == 0

    def test_out_of_range_page_raises(self, tmp_path):
        """Out-of-range page should raise RuntimeError."""
        pdf_file = tmp_path / "text.pdf"
        pdf_file.write_bytes(_text_pdf())
        with pytest.raises(RuntimeError):
            unpdf.get_page_stats(str(pdf_file), 99)

    def test_unresolvable_font_reports_suppressed_text_runs(self, tmp_path):
        """The per-page count is what the document-level total is built from, so
        a consumer discriminating causes across pages in a mixed-quality document
        needs it here too, not just from ``get_extraction_quality``.
        """
        pdf_file = tmp_path / "suppressed.pdf"
        pdf_file.write_bytes(_suppressed_text_run_pdf())
        stats = unpdf.get_page_stats(str(pdf_file), 1)
        quality = unpdf.get_extraction_quality(str(pdf_file))
        assert stats["suppressed_text_runs"] > 0
        assert stats["suppressed_text_runs"] == quality["suppressed_text_runs"]


class TestToJson:
    """Tests for to_json function."""

    def test_non_existent_file_raises(self):
        """Non-existent file should raise RuntimeError."""
        with pytest.raises(RuntimeError):
            unpdf.to_json("non_existent_file.pdf")


class TestGetInfo:
    """Tests for get_info function."""

    def test_non_existent_file_raises(self):
        """Non-existent file should raise RuntimeError."""
        with pytest.raises(RuntimeError):
            unpdf.get_info("non_existent_file.pdf")


class TestErrorKind:
    """Failure classification: ``UnpdfError.kind``."""

    def test_missing_file_is_classified_as_io(self):
        """A missing file is an I/O failure, distinguishable from a damaged PDF."""
        with pytest.raises(unpdf.UnpdfError) as excinfo:
            unpdf.to_text("definitely-not-a-real-file-9f3a.pdf")
        assert excinfo.value.kind == unpdf.ErrorKind.IO

    def test_non_pdf_input_is_classified(self, tmp_path):
        """Any failure must carry a reason — a message with kind NONE is useless."""
        junk = tmp_path / "not.pdf"
        junk.write_text("this is plainly not a PDF at all")
        with pytest.raises(unpdf.UnpdfError) as excinfo:
            unpdf.to_text(str(junk))
        assert excinfo.value.kind != unpdf.ErrorKind.NONE

    def test_out_of_range_page_is_classified(self, tmp_path):
        """Page range failures are distinguishable from parse failures."""
        pdf_file = tmp_path / "text.pdf"
        pdf_file.write_bytes(_text_pdf())
        with pytest.raises(unpdf.UnpdfError) as excinfo:
            unpdf.get_page_stats(str(pdf_file), 99)
        assert excinfo.value.kind == unpdf.ErrorKind.PAGE_OUT_OF_RANGE

    def test_unpdf_error_is_a_runtime_error(self):
        """Callers written against the pre-classification API keep working."""
        assert issubclass(unpdf.UnpdfError, RuntimeError)

    def test_error_kind_values_match_the_native_abi(self):
        """These numbers are duplicated from unpdf.h; pin them so drift fails loudly."""
        assert {k.name: int(k) for k in unpdf.ErrorKind} == {
            "NONE": 0,
            "OTHER": 1,
            "IO": 2,
            "UNKNOWN_FORMAT": 3,
            "UNSUPPORTED_VERSION": 4,
            "PDF_PARSE": 5,
            "ENCRYPTED": 6,
            "INVALID_PASSWORD": 7,
            "CORRUPTED": 8,
            "MISSING_OBJECT": 9,
            "FONT_DECODE": 10,
            "IMAGE_EXTRACT": 11,
            "RENDER": 12,
            "TEXT_EXTRACT": 13,
            "PAGE_OUT_OF_RANGE": 14,
            "INVALID_PAGE_RANGE": 15,
            "RESOURCE_NOT_FOUND": 16,
            "ENCODING": 17,
            "INVALID_ARGUMENT": 100,
            "PANIC": 101,
            "INVALID_OUTPUT": 102,
        }

    def test_unknown_kind_is_preserved_as_int(self):
        """A future native build may report a reason this package predates."""
        err = unpdf.UnpdfError("from the future", 9999)
        assert err.kind == 9999


class TestMarkdownFlags:
    """A core render option is only real to a Python caller once the flag that
    selects it is importable and actually reaches the renderer. An ignored flag
    does not fail: the call succeeds and simply does not do what was asked."""

    def test_flags_are_exported_from_the_package(self):
        """The ``flags`` argument is public, so its constants must be too."""
        assert unpdf.UNPDF_FLAG_FRONTMATTER == 1
        assert unpdf.UNPDF_FLAG_ESCAPE_SPECIAL == 2
        assert unpdf.UNPDF_FLAG_PAGE_MARKERS == 8
        assert unpdf.UNPDF_FLAG_REFINE == 16

    def test_refine_does_not_error_when_enabled(self, tmp_path):
        path = tmp_path / "text.pdf"
        path.write_bytes(_text_pdf())
        markdown = unpdf.to_markdown(str(path), unpdf.UNPDF_FLAG_REFINE)
        assert len(markdown) > 0

    def test_page_markers_off_by_default(self, tmp_path):
        path = tmp_path / "text.pdf"
        path.write_bytes(_text_pdf())
        assert "<!-- page " not in unpdf.to_markdown(str(path))

    def test_page_markers_marks_page_boundaries(self, tmp_path):
        path = tmp_path / "text.pdf"
        path.write_bytes(_text_pdf())
        markdown = unpdf.to_markdown(str(path), unpdf.UNPDF_FLAG_PAGE_MARKERS)
        assert "<!-- page 1 -->" in markdown

    def test_page_markers_does_not_imply_frontmatter(self, tmp_path):
        path = tmp_path / "text.pdf"
        path.write_bytes(_text_pdf())
        markdown = unpdf.to_markdown(str(path), unpdf.UNPDF_FLAG_PAGE_MARKERS)
        assert not markdown.startswith("---")

    def test_flags_combine(self, tmp_path):
        path = tmp_path / "text.pdf"
        path.write_bytes(_text_pdf())
        markdown = unpdf.to_markdown(
            str(path),
            unpdf.UNPDF_FLAG_FRONTMATTER | unpdf.UNPDF_FLAG_PAGE_MARKERS,
        )
        assert markdown.startswith("---")
        assert "<!-- page 1 -->" in markdown
