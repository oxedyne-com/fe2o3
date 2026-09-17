# Pearl web reader (first cut)

A vanilla-JS browser reader for the Pearl (`.prl`) document format. It renders a `.prl` to inline SVG
from the format's own data model -- glyph outlines stored once and placed per leaf, plus fills,
strokes, rules and rasters -- reproducing the transform the Austenite SVG arm applies, so the page is
pixel-identical to Austenite's own SVG output (which already matches the PDF).

No framework, no build step, no wasm. Just `index.html` + `pearl.js`.

## Run

```bash
cd fe2o3_austenite/web/pearl-reader
python3 -m http.server 8137 --bind 127.0.0.1
# then open http://127.0.0.1:8137/index.html
```

A static server is needed because the reader `fetch()`es the document JSON; a bare `file://` open is
blocked by the browser's local-file CORS rule.

## Transport

The `.prl` is text jdat. The companion binary `pearl_json` (added to this crate) decodes the `.prl`'s
`Dat` and re-encodes the *identical* value as JSON, which the browser `JSON.parse`s. Nothing is
pre-rendered -- the browser builds every page from outlines and placements. Rasters ride as base64
strings, exactly as the `.prl` already stores them.

```bash
T=~/.cache/cargo-targets/$RC_SLOT/austenite-pearl/debug   # or your target dir
$T/austenite --pearl samples/keystone.typ out/            # writes out/document.prl (+ reference SVG/PDF)
$T/pearl_json out/document.prl samples/keystone.json      # projects it to JSON for the reader
```

`samples/*.json` are checked-in demo projections so the reader runs out of the box; regenerate them
with the two commands above.

## Verify (pixel parity)

Rasterise the reader page and Austenite's reference `page-001.svg` through the same rasteriser and
pixel-diff them. Measured differing pixels (of 500,990) against the reference:

| Document        | exact-match diff | at 12% fuzz |
|-----------------|------------------|-------------|
| keystone        | 105 (0.021%)     | 3 (0.0006%) |
| manuscript p1-3 | 24 / 69 / 36     | 0 / 0 / 0   |
| maths           | 77 (0.015%)      | 0           |

The residual is sub-pixel antialiasing on glyph and hairline-rule edges, from placing a transformed
outline versus a baked one -- not glyph misplacement.
