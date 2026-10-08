"""Regenerate the inline font fixtures (`inline-font-*.pdf`).

A page's `/Resources /Font` dictionary maps names to font dictionaries. ISO 32000-1
does not require those dictionaries to be indirect objects, so a font may be written
inline, as the value of its name. Each fixture writes its fonts that way:

    inline-font-type0.pdf      Type 0, predefined Unicode CMap (UniKS-UCS2-H), no /ToUnicode
    inline-font-simple.pdf     simple fonts: a standard 14 font under WinAnsiEncoding, and a
                               TrueType font with /Widths and a descriptor declaring weight 700
    inline-font-tounicode.pdf  Type 0, Identity-H, CIDs mapped by a /ToUnicode stream (a stream
                               is always an indirect object; the font around it is inline)
    inline-font-form.pdf       the inline font sits in a Form XObject's own /Resources
    inline-font-two-pages.pdf  two pages, each with an inline /F1 -- a different font each time

    python make_inline_font_fixtures.py        # writes next to this script

Standard library only; output is deterministic.
"""

from pathlib import Path

HERE = Path(__file__).resolve().parent

# "한글 문서" in UCS-2 (big-endian code units).
HANGUL_UCS2 = "<D55CAE000020BB38C11C>"

TYPE0_UCS2 = (
    b"<</Type/Font/Subtype/Type0/BaseFont/HYSMyeongJo-Medium/Encoding/UniKS-UCS2-H"
    b"/DescendantFonts[<</Type/Font/Subtype/CIDFontType0/BaseFont/HYSMyeongJo-Medium"
    b"/CIDSystemInfo<</Registry(Adobe)/Ordering(Korea1)/Supplement 1>>"
    b"/FontDescriptor<</Type/FontDescriptor/FontName/HYSMyeongJo-Medium/Flags 6"
    b"/FontBBox[0 -148 1001 880]/ItalicAngle 0/Ascent 880/Descent -148/CapHeight 880"
    b"/StemV 50>>/DW 1000/W[1[333]]>>]>>"
)


def widths(first: int, last: int, width: int) -> bytes:
    return b"[" + b" ".join(str(width).encode() for _ in range(first, last + 1)) + b"]"


def simple_truetype(base_font: bytes, width: int, weight: int) -> bytes:
    return (
        b"<</Type/Font/Subtype/TrueType/BaseFont/" + base_font
        + b"/FirstChar 32/LastChar 255/Widths" + widths(32, 255, width)
        + b"/Encoding/WinAnsiEncoding/FontDescriptor<</Type/FontDescriptor/FontName/"
        + base_font + b"/Flags 32/FontBBox[0 -200 1000 900]/ItalicAngle 0/Ascent 900"
        + b"/Descent -200/CapHeight 700/StemV 80/FontWeight " + str(weight).encode() + b">>>>"
    )


def stream(dict_entries: bytes, data: bytes) -> bytes:
    return (
        b"<<" + dict_entries + b"/Length " + str(len(data)).encode() + b">>\nstream\n"
        + data + b"\nendstream"
    )


def page(contents: int, resources: bytes) -> bytes:
    return (
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 400 200]/Resources" + resources
        + b"/Contents " + str(contents).encode() + b" 0 R>>"
    )


def pdf(objects: list) -> bytes:
    """Objects numbered from 1, in order; object 1 is the catalog."""
    out = bytearray(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n")
    offsets = []
    for number, body in enumerate(objects, start=1):
        offsets.append(len(out))
        out += b"%d 0 obj\n" % number + body + b"\nendobj\n"
    xref = len(out)
    size = len(objects) + 1
    out += b"xref\n0 %d\n0000000000 65535 f \n" % size
    for offset in offsets:
        out += b"%010d 00000 n \n" % offset
    out += b"trailer\n<</Size %d/Root 1 0 R>>\nstartxref\n%d\n%%%%EOF\n" % (size, xref)
    return bytes(out)


def catalog_and_pages(page_numbers: list) -> list:
    kids = b" ".join(b"%d 0 R" % n for n in page_numbers)
    return [
        b"<</Type/Catalog/Pages 2 0 R>>",
        b"<</Type/Pages/Kids[" + kids + b"]/Count " + str(len(page_numbers)).encode() + b">>",
    ]


def type0() -> bytes:
    content = b"BT /F1 14 Tf 20 150 Td " + HANGUL_UCS2.encode() + b" Tj ET"
    return pdf(catalog_and_pages([3]) + [
        page(4, b"<</Font<</F1 " + TYPE0_UCS2 + b">>>>"),
        stream(b"", content),
    ])


def simple() -> bytes:
    helvetica = b"<</Type/Font/Subtype/Type1/BaseFont/Helvetica/Encoding/WinAnsiEncoding>>"
    content = (
        b"BT /F1 12 Tf 20 150 Td (\\223Caf\\351\\224 costs \\200 5 \\227 paid) Tj ET\n"
        b"BT /F2 12 Tf 20 120 Td (Heavy words) Tj ET"
    )
    return pdf(catalog_and_pages([3]) + [
        page(4, b"<</Font<</F1 " + helvetica + b"/F2 " + simple_truetype(b"Plain", 600, 700)
             + b">>>>"),
        stream(b"", content),
    ])


def tounicode() -> bytes:
    cmap = (
        b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n"
        b"/CIDSystemInfo <</Registry (Adobe) /Ordering (UCS) /Supplement 0>> def\n"
        b"/CMapName /Adobe-Identity-UCS def /CMapType 2 def\n"
        b"1 begincodespacerange <0000> <FFFF> endcodespacerange\n"
        b"5 beginbfchar\n<0001> <D55C>\n<0002> <AE00>\n<0003> <0020>\n"
        b"<0004> <BB38>\n<0005> <C11C>\nendbfchar\n"
        b"endcmap CMapName currentdict /CMap defineresource pop end end"
    )
    font = (
        b"<</Type/Font/Subtype/Type0/BaseFont/Batang/Encoding/Identity-H/ToUnicode 5 0 R"
        b"/DescendantFonts[<</Type/Font/Subtype/CIDFontType2/BaseFont/Batang"
        b"/CIDSystemInfo<</Registry(Adobe)/Ordering(Identity)/Supplement 0>>"
        b"/FontDescriptor<</Type/FontDescriptor/FontName/Batang/Flags 6"
        b"/FontBBox[0 -150 1000 850]/ItalicAngle 0/Ascent 850/Descent -150/CapHeight 700"
        b"/StemV 80>>/DW 1000/W[3[250]]>>]>>"
    )
    content = b"BT /F1 14 Tf 20 150 Td <00010002000300040005> Tj ET"
    return pdf(catalog_and_pages([3]) + [
        page(4, b"<</Font<</F1 " + font + b">>>>"),
        stream(b"", content),
        stream(b"", cmap),
    ])


def form() -> bytes:
    form_content = b"BT /F1 14 Tf 20 150 Td " + HANGUL_UCS2.encode() + b" Tj ET"
    return pdf(catalog_and_pages([3]) + [
        page(4, b"<</XObject<</Fm1 5 0 R>>>>"),
        stream(b"", b"q /Fm1 Do Q"),
        stream(
            b"/Type/XObject/Subtype/Form/BBox[0 0 400 200]"
            b"/Resources<</Font<</F1 " + TYPE0_UCS2 + b">>>>",
            form_content,
        ),
    ])


def two_pages() -> bytes:
    first = b"BT /F1 12 Tf 20 150 Td (\\223quoted\\224 text) Tj ET"
    second = b"BT /F1 14 Tf 20 150 Td " + HANGUL_UCS2.encode() + b" Tj ET"
    return pdf(catalog_and_pages([3, 4]) + [
        page(5, b"<</Font<</F1 " + simple_truetype(b"Narrow", 500, 400) + b">>>>"),
        page(6, b"<</Font<</F1 " + TYPE0_UCS2 + b">>>>"),
        stream(b"", first),
        stream(b"", second),
    ])


FIXTURES = {
    "inline-font-type0.pdf": type0,
    "inline-font-simple.pdf": simple,
    "inline-font-tounicode.pdf": tounicode,
    "inline-font-form.pdf": form,
    "inline-font-two-pages.pdf": two_pages,
}


def main() -> None:
    for name, build in FIXTURES.items():
        (HERE / name).write_bytes(build())


if __name__ == "__main__":
    main()
