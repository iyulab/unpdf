"""Regenerate the vertical-writing fixtures (`vertical-*.pdf`).

A composite font under a vertical CMap (`-V` name, `Identity-V`, or `/WMode 1`) advances
DOWN the page: each glyph moves the text position by the CIDFont's `/W2` vertical
displacement `w1y` (or the `/DW2` default `[vy w1y]`, `[880 -1000]` when absent), and its
origin sits at the position vector `(vx, vy)` from the text position (ISO 32000-1
section 9.7.4.3). Every fixture shows the hiragana letters (U+3042, U+3044, U+3046; Adobe-Japan1
CIDs 843, 845, 847) as three single-glyph `Tj` strings in ONE text object, 20 pt, starting
at (300, 180), under the non-embedded font HeiseiMin-W3:

    vertical-default.pdf       UniJIS-UCS2-V, no /W2, no /DW2          (advance -1000 each)
    vertical-w2.pdf            /W2 [843 [-500 500 880] 847 [-1500 500 880]]; 845 takes /DW2
    vertical-w2-range.pdf      /W2 [843 847 -400 500 880]              (range form)
    vertical-dw2.pdf           /DW2 [800 -600], no /W2
    vertical-origin.pdf        /W2 [843 847 -1000 250 700]             (position vector off the default)
    vertical-tj.pdf            [<3042> -500] TJ [1000 <3044>] TJ <3046> Tj  (adjustments move vertically)
    vertical-spacing.pdf       5 Tc, 7 Tw, 50 Tz around the same three strings
    vertical-malformed-w2.pdf  a /W2 full of garbage (names, short triples, huge ranges)
    vertical-identity.pdf      Identity-V with /ToUnicode, /W2 for CIDs 1 and 2
    vertical-embedded.pdf      an embedded CMap stream with /WMode 1, no /W2

    python make_vertical_fixtures.py        # writes next to this script

Standard library only; output is deterministic.
"""

from pathlib import Path

HERE = Path(__file__).resolve().parent

STRINGS = b"<3042> Tj <3044> Tj <3046> Tj"


def stream(dict_entries: bytes, data: bytes) -> bytes:
    return (
        b"<<" + dict_entries + b"/Length " + str(len(data)).encode() + b">>\nstream\n"
        + data + b"\nendstream"
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


def document(content: bytes, cid_extra: bytes = b"", encoding: bytes = b"/UniJIS-UCS2-V",
             ordering: bytes = b"Japan1", font_extra: bytes = b"", extra_objects=()) -> bytes:
    """One page; /F1 is a Type 0 font over an indirect CIDFont (object 5).

    `extra_objects` are numbered from 7 (object 6 is the page content)."""
    return pdf([
        b"<</Type/Catalog/Pages 2 0 R>>",
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>",
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 400 200]/Resources<</Font<</F1 4 0 R>>>>"
        b"/Contents 6 0 R>>",
        b"<</Type/Font/Subtype/Type0/BaseFont/HeiseiMin-W3/Encoding " + encoding
        + font_extra + b"/DescendantFonts[5 0 R]>>",
        b"<</Type/Font/Subtype/CIDFontType0/BaseFont/HeiseiMin-W3"
        b"/CIDSystemInfo<</Registry(Adobe)/Ordering(" + ordering + b")/Supplement 2>>"
        b"/FontDescriptor<</Type/FontDescriptor/FontName/HeiseiMin-W3/Flags 6"
        b"/FontBBox[-123 -257 1001 910]/ItalicAngle 0/Ascent 880/Descent -120"
        b"/CapHeight 700/StemV 50>>/DW 1000" + cid_extra + b">>",
        stream(b"", b"BT /F1 20 Tf 300 180 Td " + content + b" ET"),
        *extra_objects,
    ])


def default() -> bytes:
    return document(STRINGS)


def w2() -> bytes:
    return document(STRINGS, b"/W2[843[-500 500 880]847[-1500 500 880]]")


def w2_range() -> bytes:
    return document(STRINGS, b"/W2[843 847 -400 500 880]")


def origin() -> bytes:
    return document(STRINGS, b"/W2[843 847 -1000 250 700]")


def dw2() -> bytes:
    return document(STRINGS, b"/DW2[800 -600]")


def tj() -> bytes:
    return document(b"[<3042> -500] TJ [1000 <3044>] TJ <3046> Tj")


def spacing() -> bytes:
    return document(b"5 Tc 7 Tw 50 Tz " + STRINGS)


def malformed_w2() -> bytes:
    return document(
        STRINGS,
        b"/DW2[/x]/W2[4294967295[-1 1 1 -2 2 2]843[-500]/Name 844 845 5 0 4000000000 -1 1 1 9 [1 2] 846 [1]]",
    )


def identity() -> bytes:
    to_unicode = (
        b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n"
        b"1 begincodespacerange <0000> <FFFF> endcodespacerange\n"
        b"2 beginbfchar <0001> <3042> <0002> <3044> endbfchar\n"
        b"endcmap CMapName currentdict /CMap defineresource pop end end"
    )
    return document(
        b"<0001> Tj <0002> Tj",
        b"/W2[1[-700 250 700]]",
        encoding=b"/Identity-V",
        ordering=b"Identity",
        font_extra=b"/ToUnicode 7 0 R",
        extra_objects=[stream(b"", to_unicode)],
    )


def embedded() -> bytes:
    cmap = (
        b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n/WMode 1 def\n"
        b"1 begincodespacerange <0000> <FFFF> endcodespacerange\n"
        b"1 begincidrange <3040> <309F> 841 endcidrange\n"
        b"endcmap CMapName currentdict /CMap defineresource pop end end"
    )
    return document(
        STRINGS,
        encoding=b"7 0 R",
        extra_objects=[stream(
            b"/Type/CMap/CMapName/Embedded-V/WMode 1"
            b"/CIDSystemInfo<</Registry(Adobe)/Ordering(Japan1)/Supplement 2>>",
            cmap,
        )],
    )


FIXTURES = {
    "vertical-default.pdf": default,
    "vertical-w2.pdf": w2,
    "vertical-w2-range.pdf": w2_range,
    "vertical-dw2.pdf": dw2,
    "vertical-origin.pdf": origin,
    "vertical-tj.pdf": tj,
    "vertical-spacing.pdf": spacing,
    "vertical-malformed-w2.pdf": malformed_w2,
    "vertical-identity.pdf": identity,
    "vertical-embedded.pdf": embedded,
}


def main() -> None:
    for name, build in FIXTURES.items():
        (HERE / name).write_bytes(build())


if __name__ == "__main__":
    main()
