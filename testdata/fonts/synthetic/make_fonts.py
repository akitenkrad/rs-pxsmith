"""Generate the synthetic TTF fixtures for `pxsmith font import` (MYTASK-3552).

Run from this directory:  uv run --with fonttools python make_fonts.py

Every glyph is drawn as unit squares on a pixel grid (100 font units per pixel,
1 em = 8 px = 800 units), so centre-point rasterisation at --size 8 must give
back exactly the bitmaps below. The one exception is `O`, whose outline is
shifted off the grid on purpose so that the off-grid advisory has something to
count. Self-made, CC0.
"""

from fontTools.fontBuilder import FontBuilder
from fontTools.feaLib.builder import addOpenTypeFeaturesFromString
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib.tables._k_e_r_n import KernTable_format_0, table__k_e_r_n

PX = 100
ASCENT_PX, DESCENT_PX = 6, 2

# rows top to bottom, 8 rows (ascent 6 + descent 2); '#' is ink
GLYPHS = {
    "space": (4, ["...."] * 8),
    "A": (6, ["..#...", ".#.#..", "#...#.", "#####.", "#...#.", "#...#.", "......", "......"]),
    "V": (6, ["#...#.", "#...#.", "#...#.", ".#.#..", ".#.#..", "..#...", "......", "......"]),
    "uni30FC": (8, ["........", "........", "........", "#######.", "........", "........", "........", "........"]),
    "uni30FC.vert": (8, ["...#....", "...#....", "...#....", "...#....", "...#....", "...#....", "...#....", "........"]),
    "uniFF08": (8, [".....#..", "....#...", "....#...", "....#...", "....#...", "....#...", ".....#..", "........"]),
    "uniFF08.vert": (8, ["........", "........", "........", "........", "........", ".######.", "#......#", "........"]),
    "O": (6, [".###..", "#...#.", "#...#.", "#...#.", "#...#.", ".###..", "......", "......"]),
}
CMAP = {0x20: "space", 0x41: "A", 0x56: "V", 0x30FC: "uni30FC", 0xFF08: "uniFF08", 0x4F: "O"}
OFF_GRID = {"O": 30}  # units added to every x coordinate


def draw(name):
    width, rows = GLYPHS[name]
    shift = OFF_GRID.get(name, 0)
    pen = TTGlyphPen(None)
    for r, row in enumerate(rows):
        for c, cell in enumerate(row):
            if cell != "#":
                continue
            top = (ASCENT_PX - r) * PX
            x0, x1 = c * PX + shift, (c + 1) * PX + shift
            y0, y1 = top - PX, top
            pen.moveTo((x0, y0)); pen.lineTo((x0, y1)); pen.lineTo((x1, y1)); pen.lineTo((x1, y0)); pen.closePath()
    return pen.glyph(), width * PX


def build(path, family, gpos_kern, legacy_kern):
    order = [".notdef"] + list(GLYPHS)
    fb = FontBuilder(8 * PX, isTTF=True)
    fb.setupGlyphOrder(order)
    fb.setupCharacterMap(CMAP)
    glyf, metrics = {}, {}
    empty = TTGlyphPen(None).glyph()
    glyf[".notdef"], metrics[".notdef"] = empty, (4 * PX, 0)
    for name in GLYPHS:
        g, adv = draw(name)
        glyf[name], metrics[name] = g, (adv, 0)
    fb.setupGlyf(glyf)
    fb.setupHorizontalMetrics({n: (adv, 0) for n, (adv, _) in metrics.items()})
    fb.setupHorizontalHeader(ascent=ASCENT_PX * PX, descent=-DESCENT_PX * PX)
    fb.setupNameTable({"familyName": family, "styleName": "Regular",
                       "copyright": "pxsmith synthetic test font (CC0)",
                       "licenseDescription": "CC0 1.0 Universal"})
    fb.setupOS2(sTypoAscender=ASCENT_PX * PX, sTypoDescender=-DESCENT_PX * PX,
                usWinAscent=ASCENT_PX * PX, usWinDescent=DESCENT_PX * PX)
    fb.setupPost()
    fea = "languagesystem DFLT dflt;\nfeature vert { sub uni30FC by uni30FC.vert; sub uniFF08 by uniFF08.vert; } vert;\n"
    if gpos_kern:
        fea += "feature kern { pos A V -100; } kern;\n"
    addOpenTypeFeaturesFromString(fb.font, fea)
    if legacy_kern:
        kern = table__k_e_r_n(); kern.version = 0
        sub = KernTable_format_0(); sub.version = 0; sub.coverage = 1; sub.format = 0
        sub.kernTable = {("A", "V"): -100}
        kern.kernTables = [sub]
        fb.font["kern"] = kern
    fb.save(path)


build("pxsynth-gpos.ttf", "PxSynthGpos", gpos_kern=True, legacy_kern=False)
build("pxsynth-kern.ttf", "PxSynthKern", gpos_kern=False, legacy_kern=True)
print("wrote pxsynth-gpos.ttf, pxsynth-kern.ttf")
