"""Two tiny test fonts: glyph 'A' is a filled square (100..900 x 0..800 of 1000 units),
'space' has no outline. One with TrueType outlines, one bare CFF (FontFile3 /Type1C)."""
import sys
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.pens.t2CharStringPen import T2CharStringPen

out = sys.argv[1]
order = [".notdef", "space", "A"]
cmap = {32: "space", 65: "A"}
metrics = {".notdef": (500, 0), "space": (250, 0), "A": (1000, 100)}

def square(pen):
    pen.moveTo((100, 0)); pen.lineTo((900, 0)); pen.lineTo((900, 800)); pen.lineTo((100, 800)); pen.closePath()

# TrueType
fb = FontBuilder(1000, isTTF=True)
fb.setupGlyphOrder(order); fb.setupCharacterMap(cmap)
glyphs = {}
for name in order:
    pen = TTGlyphPen(None)
    if name == "A":
        square(pen)
    glyphs[name] = pen.glyph()
fb.setupGlyf(glyphs); fb.setupHorizontalMetrics(metrics); fb.setupHorizontalHeader(ascent=800, descent=-200)
fb.setupNameTable({"familyName": "UnpdfSquare", "styleName": "Regular"}); fb.setupOS2(); fb.setupPost()
fb.save(f"{out}/square.ttf")

# CFF (OpenType), then pull out the bare CFF table
fb = FontBuilder(1000, isTTF=False)
fb.setupGlyphOrder(order); fb.setupCharacterMap(cmap)
cs = {}
for name in order:
    pen = T2CharStringPen(metrics[name][0], None)
    if name == "A":
        square(pen)
    cs[name] = pen.getCharString()
fb.setupCFF("UnpdfSquareCFF", {"FullName": "UnpdfSquareCFF"}, cs, {})
fb.setupHorizontalMetrics(metrics); fb.setupHorizontalHeader(ascent=800, descent=-200)
fb.setupNameTable({"familyName": "UnpdfSquareCFF", "styleName": "Regular"}); fb.setupOS2(); fb.setupPost()
font = fb.font
open(f"{out}/square.cff", "wb").write(font.getTableData("CFF "))
