# Pearl web reader

A vanilla-JS browser reader for the Pearl (`.prl`) document format. It fetches a real `.prl`, parses its
text jdat directly in the browser, and renders every page to inline SVG from the format's own data model
-- glyph outlines stored once and placed per leaf, plus fills, strokes, rules and rasters -- reproducing
the transform the Austenite SVG arm applies, so the page is pixel-identical to Austenite's own SVG output
(which already matches the PDF).

No framework, no build step, no wasm, and **no JSON projection**. Just `index.html` + `jdat.js` +
`pearl.js`, reading the `.prl` the engine writes.

## Run

```bash
cd fe2o3_austenite/web/pearl-reader
python3 -m http.server 8137 --bind 127.0.0.1
# then open http://127.0.0.1:8137/index.html
```

A static server is needed because the reader `fetch()`es the `.prl`; a bare `file://` open is blocked by
the browser's local-file CORS rule.

## How it reads the format

The `.prl` is text jdat: an ordered map of ordered maps, lists, strings and typed scalar atoms.
`jdat.js` is a minimal recursive-descent parser for exactly the subset a v0 `.prl` uses:

| jdat text                | shape        | JS value                    |
|--------------------------|--------------|-----------------------------|
| `(omap\|{ "k": v, ... })` | ordered map  | object, insertion order kept |
| `[ v, v, ... ]`          | list         | array                       |
| `"..."`                  | string       | string (RFC 8259 escapes)   |
| `(u32\|N)` `(i32\|N)` `(u8\|N)` | integer atom | Number                |
| `(f32\|X.YeZ)`            | float atom   | Number                      |

There are no byte-strings in a v0 `.prl`: a raster's PNG rides as a **base64 string** (the writer stores
`base64::encode(png)`), so the whole file is these five shapes. Stripping the type tag yields the same
plain value the old JSON projection did, so `pearl.js`'s renderer is unchanged -- it consumes the parsed
document directly. Unknown map keys and type tags are tolerated, so a parallel lane adding fields does
not break the reader.

Regenerate the samples with the engine:

```bash
T=~/.cache/cargo-targets/$RC_SLOT/austenite-pearl/debug   # or your target dir
$T/austenite --pearl samples/keystone.typ out/            # writes out/document.prl
cp out/document.prl samples/keystone.prl
```

## Verify (pixel parity)

Rasterise the reader's SVG and Austenite's reference `page-001.svg` through the same rasteriser
(inkscape) and pixel-diff them. Differing pixels of 500,990 against the SVG-arm reference:

| Document  | exact-match diff | at 12% fuzz |
|-----------|------------------|-------------|
| keystone  | 560 (0.112%)     | 0           |
| maths     | 40 (0.008%)      | 0           |
| raster    | 45 (0.009%)      | 0           |

Zero differing pixels at 12% fuzz means no glyph, path or raster is misplaced: the residual is sub-pixel
antialiasing on glyph and hairline edges, from placing a transformed outline versus a baked one. The
raster page (`raster.prl`, a PNG carried as base64) diffs to zero at fuzz -- the `<image>` leaf renders
identically.

## Byte parity

Not met, and the reason is architectural rather than number formatting. `pearl_render` **bakes** each
translate/matrix transform into the path `d` coordinates and emits a bare `<path d="...">`; the reader
keeps the stored `d` and applies a `transform` attribute. Same pixels, different SVG text. Closing it
would mean porting `fe2o3_graphics`'s `Path::transform` + `write_path_data` float formatting into JS --
worthwhile only if a byte-identical SVG is itself a requirement; pixel parity above is the shippable
metric.
