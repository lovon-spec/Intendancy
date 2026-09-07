#!/usr/bin/env python3
"""Build the Agent Skills Registry logo: a two-line wordmark, "Agent" in near-black
and "Skills" in teal, set in Inter and outlined to paths so that no viewer needs
the font. The registry is unbranded infrastructure (docs/BRAND.md): the logo is
the name itself, in a typeface that belongs to no one.

Reproducible: the font is fetched from a pinned URL and checked against a pinned
SHA-256, static instances are cut from the variable font, the text is shaped with
HarfBuzz (kerning on), and the glyph outlines are written as SVG paths.

    python3 tools/logo/build-logo.py meta-evidence/agent-skills-registry-logo.svg

Requires: fontTools, uharfbuzz (pip install fonttools uharfbuzz). Inter is licensed
under the SIL Open Font License 1.1 (Copyright 2020 The Inter Project Authors);
outlined glyphs in a logo are a permitted use, and the font itself is not
redistributed by this repository.
"""
import hashlib, pathlib, sys, tempfile, urllib.request

FONT_URL = "https://github.com/google/fonts/raw/main/ofl/inter/Inter%5Bopsz%2Cwght%5D.ttf"
FONT_SHA256 = "29160a80ff49ddcab2c97711247e08b1fab27a484a329ce8b813d820dc559031"
WEIGHT, OPSZ = 800, 32
W, H, RADIUS = 760, 400, 24
DARK, TEAL, PLATE = "#1d2227", "#2b8a94", "#ffffff"
LINES = [("Agent", DARK, 70, 175), ("Skills", TEAL, 70, 335)]  # text, fill, x, baseline y
SIZE, TRACKING = 170, -3

def fetch_font(cache: pathlib.Path) -> pathlib.Path:
    cache.mkdir(parents=True, exist_ok=True)
    f = cache / "Inter-var.ttf"
    if not f.exists():
        urllib.request.urlretrieve(FONT_URL, f)
    digest = hashlib.sha256(f.read_bytes()).hexdigest()
    if digest != FONT_SHA256:
        sys.exit(f"font digest mismatch: {digest} != {FONT_SHA256}")
    return f

def main(out: pathlib.Path) -> None:
    import uharfbuzz as hb
    from fontTools.ttLib import TTFont
    from fontTools.varLib import instancer
    from fontTools.pens.svgPathPen import SVGPathPen
    from fontTools.pens.transformPen import TransformPen

    cache = pathlib.Path(tempfile.gettempdir()) / "agent-skills-registry-logo"
    var = fetch_font(cache)
    static = cache / f"Inter-{WEIGHT}.ttf"
    if not static.exists():
        instancer.instantiateVariableFont(TTFont(var), {"wght": WEIGHT, "opsz": OPSZ}).save(static)
    data = static.read_bytes()
    face = hb.Face(hb.Blob(data)); font = hb.Font(face); upem = face.upem; font.scale = (upem, upem)
    tt = TTFont(static); glyph_set = tt.getGlyphSet(); order = tt.getGlyphOrder()
    scale = SIZE / upem

    parts = [
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" role="img" aria-labelledby="title desc">',
        '<title id="title">Agent Skills Registry</title>',
        '<desc id="desc">The words Agent Skills stacked on a white plate; Agent in near-black, Skills in teal.</desc>',
        f'<rect width="{W}" height="{H}" rx="{RADIUS}" fill="{PLATE}"/>',
    ]
    for text, fill, x0, y0 in LINES:
        buf = hb.Buffer(); buf.add_str(text); buf.guess_segment_properties()
        hb.shape(font, buf, {"kern": True, "liga": True})
        x = 0.0; paths = []
        for info, pos in zip(buf.glyph_infos, buf.glyph_positions):
            pen = SVGPathPen(glyph_set)
            glyph_set[order[info.codepoint]].draw(TransformPen(pen, (scale, 0, 0, -scale, x + pos.x_offset * scale, -pos.y_offset * scale)))
            d = pen.getCommands()
            if d:
                paths.append(f'<path d="{d}"/>')
            x += pos.x_advance * scale + TRACKING
        parts.append(f'<g transform="translate({x0} {y0})" fill="{fill}">' + "".join(paths) + "</g>")
    parts.append("</svg>")
    out.write_text("\n".join(parts) + "\n", encoding="utf-8")
    print(out, out.stat().st_size, "bytes")

if __name__ == "__main__":
    main(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "meta-evidence/agent-skills-registry-logo.svg"))
