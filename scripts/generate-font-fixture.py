#!/usr/bin/env python3
"""Generate original deterministic test glyphs; requires fontTools 4.60.2.
No installed/system font is read or copied. Production/tests consume the saved TTF.
"""
from pathlib import Path
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen

font = FontBuilder(1000, isTTF=True)
font.setupGlyphOrder(['.notdef', 'space', 'X'])
font.setupCharacterMap({32: 'space', 88: 'X'})
glyphs = {}
for name in ['.notdef', 'space', 'X']:
    pen = TTGlyphPen(None)
    if name == 'X':
        # Our X deliberately consists of two bars with a large empty middle.
        for left, right in [(0, 200), (800, 1000)]:
            pen.moveTo((left, 0)); pen.lineTo((right, 0))
            pen.lineTo((right, 1000)); pen.lineTo((left, 1000)); pen.closePath()
    glyphs[name] = pen.glyph()
font.setupGlyf(glyphs)
font.setupHorizontalMetrics({name: (1000, 0) for name in glyphs})
font.setupHorizontalHeader(ascent=1000, descent=0)
font.setupNameTable({'familyName': 'OVLFixtureBars', 'styleName': 'Regular',
    'uniqueFontIdentifier': 'OVLFixtureBars-1', 'fullName': 'OVLFixtureBars Regular',
    'psName': 'OVLFixtureBars-Regular', 'version': 'Version 1.0',
    'licenseDescription': 'Original OpenVideoLibrary test fixture, distributed under the repository license.'})
font.setupOS2(sTypoAscender=1000, sTypoDescender=0, usWinAscent=1000, usWinDescent=0, fsType=0)
font.setupPost()
font.font['head'].created = font.font['head'].modified = 2082844800
font.save(Path(__file__).resolve().parents[1] / 'crates/server/tests/fixtures/OVLFixtureBars.ttf')
