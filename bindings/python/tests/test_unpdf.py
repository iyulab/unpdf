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
