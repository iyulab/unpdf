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
