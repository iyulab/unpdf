"""Regenerate the embedded-CMap fixtures (`embedded-cmap-*.pdf`).

A Type 0 font's `/Encoding` may be a CMap stream written into the file instead of the
name of a predefined CMap (ISO 32000-1 section 9.7.5.3). Every fixture here shows
"한글 문서" (or a variation) with a non-embedded Adobe-Korea1 CIDFont and no /ToUnicode:

    embedded-cmap-usecmap-unicode.pdf  a stream that only builds on UniKS-UCS2-H (/UseCMap)
    embedded-cmap-usecmap-legacy.pdf   builds on KSC-EUC-H and redirects one code to another CID
    embedded-cmap-ranges.pdf           fully defined: mixed 1/2-byte code space, cidrange, cidchar
    embedded-cmap-chain.pdf            a stream whose /UseCMap is another stream
    embedded-cmap-vertical.pdf         fully defined, /WMode 1
    embedded-cmap-loop.pdf             two streams that name each other as /UseCMap
    embedded-cmap-garbage.pdf          an encoding stream that is not a CMap

    python make_embedded_cmap_fixtures.py        # writes next to this script

Standard library only; output is deterministic. Adobe-Korea1 CIDs used: 한 3296, 글 1238,
space 1, 문 1945, 서 2226.
"""

from pathlib import Path

HERE = Path(__file__).resolve().parent


def font(encoding: bytes) -> bytes:
    """A Type 0 font whose /Encoding is `encoding` (a name or an indirect reference)."""
    return (
        b"<</Type/Font/Subtype/Type0/BaseFont/HYSMyeongJo-Medium/Encoding " + encoding
        + b"/DescendantFonts[<</Type/Font/Subtype/CIDFontType0/BaseFont/HYSMyeongJo-Medium"
        b"/CIDSystemInfo<</Registry(Adobe)/Ordering(Korea1)/Supplement 1>>"
        b"/FontDescriptor<</Type/FontDescriptor/FontName/HYSMyeongJo-Medium/Flags 6"
        b"/FontBBox[0 -148 1001 880]/ItalicAngle 0/Ascent 880/Descent -148/CapHeight 880"
        b"/StemV 50>>/DW 1000/W[1[333]]>>]>>"
    )


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


def document(content: bytes, *cmaps: bytes) -> bytes:
    """One page showing `content` in /F1; the CMap streams are objects 5, 6, ..."""
    return pdf([
        b"<</Type/Catalog/Pages 2 0 R>>",
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>",
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 400 200]/Resources<</Font<</F1 4 0 R>>>>"
        b"/Contents 7 0 R>>",
        font(b"5 0 R"),
        cmaps[0],
        cmaps[1] if len(cmaps) > 1 else b"null",
        stream(b"", b"BT /F1 14 Tf 20 150 Td " + content + b" Tj ET"),
    ])


def cmap_dict(extra: bytes = b"") -> bytes:
    return (
        b"/Type/CMap/CMapName/Embedded-Test"
        b"/CIDSystemInfo<</Registry(Adobe)/Ordering(Korea1)/Supplement 1>>" + extra
    )


HEAD = b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n"
TAIL = b"\nendcmap CMapName currentdict /CMap defineresource pop end end"

# "한글 문서" in UCS-2, in EUC-KR, and as the codes the fully defined CMaps below use.
UCS2 = b"<D55CAE000020BB38C11C>"
EUC_KR = b"<C7D1B1DB20B9AEBCAD>"
FULL_BODY = (
    b"2 begincodespacerange <00> <7F> <8000> <FFFF> endcodespacerange\n"
    b"1 begincidrange <20> <7E> 1 endcidrange\n"
    b"4 begincidchar <8001> 3296 <8002> 1238 <8003> 1945 <8004> 2226 endcidchar"
)
FULL_CODES = b"<80018002208003800441>"  # 한 글 space 문 서 A


def usecmap_unicode() -> bytes:
    return document(UCS2, stream(
        cmap_dict(b"/UseCMap/UniKS-UCS2-H"),
        HEAD + b"/UniKS-UCS2-H usecmap" + TAIL,
    ))


def usecmap_legacy() -> bytes:
    # 글 (B1DB) is redirected to the CID of 한, so the line reads "한한 문서".
    return document(EUC_KR, stream(
        cmap_dict(b"/UseCMap/KSC-EUC-H"),
        HEAD + b"/KSC-EUC-H usecmap\n1 begincidchar <B1DB> 3296 endcidchar" + TAIL,
    ))


def ranges() -> bytes:
    return document(FULL_CODES, stream(cmap_dict(), HEAD + FULL_BODY + TAIL))


def chain() -> bytes:
    # Object 5 builds on object 6, which defines the code space and most codes.
    return document(
        b"<800180022080038004418005>",  # ... and <8005>, which only the first stream defines
        stream(
            cmap_dict(b"/UseCMap 6 0 R"),
            HEAD + b"1 begincidchar <8005> 3296 endcidchar" + TAIL,
        ),
        stream(cmap_dict(), HEAD + FULL_BODY + TAIL),
    )


def vertical() -> bytes:
    return document(FULL_CODES, stream(
        cmap_dict(b"/WMode 1"), HEAD + b"/WMode 1 def\n" + FULL_BODY + TAIL))


def loop() -> bytes:
    return document(
        UCS2,
        stream(cmap_dict(b"/UseCMap 6 0 R"), HEAD + FULL_BODY + TAIL),
        stream(cmap_dict(b"/UseCMap 5 0 R"), HEAD + FULL_BODY + TAIL),
    )


def garbage() -> bytes:
    junk = bytes((i * 37 + 11) % 256 for i in range(200))
    return document(UCS2, stream(b"/Type/CMap", junk))


FIXTURES = {
    "embedded-cmap-usecmap-unicode.pdf": usecmap_unicode,
    "embedded-cmap-usecmap-legacy.pdf": usecmap_legacy,
    "embedded-cmap-ranges.pdf": ranges,
    "embedded-cmap-chain.pdf": chain,
    "embedded-cmap-vertical.pdf": vertical,
    "embedded-cmap-loop.pdf": loop,
    "embedded-cmap-garbage.pdf": garbage,
}


def main() -> None:
    for name, build in FIXTURES.items():
        (HERE / name).write_bytes(build())


if __name__ == "__main__":
    main()
