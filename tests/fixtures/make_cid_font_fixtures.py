"""Regenerate the non-embedded CJK font fixtures (`cid-*.pdf`).

ReportLab's built-in CJK fonts are composite (Type0) fonts that name a predefined
Unicode CMap as their `/Encoding`, carry no `/ToUnicode` and embed no font program --
the reader is expected to supply the glyphs. ReportLab also writes the descendant
CIDFont inline in the `/DescendantFonts` array rather than as a separate object.

    python make_cid_font_fixtures.py        # writes next to this script

Requires `reportlab`. Output is deterministic (`invariant=1`).
"""

from pathlib import Path

from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.cidfonts import UnicodeCIDFont
from reportlab.pdfgen import canvas

HERE = Path(__file__).resolve().parent

# (output name, font, vertical, lines)
FIXTURES = [
    ("cid-korea1-ucs2-h.pdf", "HYSMyeongJo-Medium", False,
     ["테스트 문서 - PDF", "프로젝트 코드명", "예산: 2,300,000원"]),
    ("cid-japan1-ucs2-h.pdf", "HeiseiMin-W3", False,
     ["日本語のテスト文書", "こんにちは世界"]),
    ("cid-japan1-ucs2-v.pdf", "HeiseiMin-W3", True,
     ["縦書きのテスト"]),
    ("cid-gb1-ucs2-h.pdf", "STSong-Light", False,
     ["中文测试文档", "你好世界"]),
    ("cid-cns1-ucs2-h.pdf", "MSung-Light", False,
     ["中文測試文件", "你好世界"]),
]


def main() -> None:
    for name, face, vertical, lines in FIXTURES:
        font = UnicodeCIDFont(face, isVertical=vertical)
        pdfmetrics.registerFont(font)
        page = canvas.Canvas(str(HERE / name), pagesize=(300, 200), invariant=1)
        page.setFont(font.fontName, 14)
        y = 170
        for line in lines:
            page.drawString(20, y, line)
            y -= 24
        page.save()


if __name__ == "__main__":
    main()
