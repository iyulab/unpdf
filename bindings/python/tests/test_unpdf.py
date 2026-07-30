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
