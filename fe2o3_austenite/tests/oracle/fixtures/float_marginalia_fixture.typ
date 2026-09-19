// The oracle harness's float+marginalia fixture -- the regression root for the anchor-region-membership
// bug: a margin note (or any other nested anchor -- a ref, an index entry) recorded INSIDE a float's own
// body must inherit that float's page-region membership, not default to `Region::Body`, or a later float
// insertion on the same page shifts the note away from its own line while the float's own ink (correctly
// region-tagged) stays put. `float_fixture.typ` exercises float placement alone and `marginalia_fixture.typ`
// exercises the margin note alone; neither puts one inside the other, which is why this class of bug slipped
// a green oracle. Modelled closely on `float_fixture.typ` (same self-contained `#let aside-box`, same A4
// geometry) plus `marginalia_fixture.typ`'s `#let claim-label`, so it compiles under typst 0.15.1 with no
// symbol-modifier patch, the same idiom as both.
//
// Two top-placed `#aside-box` floats stack in the top band; the FIRST one's body carries a
// `#claim-label(<A1>)`, so the margin-note anchor is recorded not as a direct child of the float's node list
// but nested inside it, through the same `place_line`/`place_vbox`/`place_leaf` helpers ordinary body text
// goes through. When the second aside is inserted, `try_insert_float` shifts every `Region::Body` anchor on
// the page down by the second float's band -- correct for a body anchor, wrong for this one, which sits
// inside the first float's own already-placed material and must stay put. Before the anchor-region-membership
// fix this note landed beside the SECOND aside instead of its own line, at the same physical y the second
// aside's own text sits at (both anchors dragged down as `Region::Body`, since the first aside's material had
// already been laid and stamped `Region::Top` by the time the shift ran).
//
// NO LEVEL-1 HEADING, for the same reason `float_fixture.typ` carries none: a lone-file `= Heading` lowers as
// austenite's ~136pt chapter-opener grid (tracked as A18), which is a heading-rendering confound, not
// something this root gates.
//
// The oracle compares page count and anchor order against Typst as every root does; `expected.json` pins no
// per-anchor y for this root (only page count, anchor count and the whole-PDF sha), so the real gate on the
// nested anchor's position is that pinned PDF sha -- a regression that moves the note's drawn y changes the
// hashed content stream. `float-fixture`'s per-float y, by contrast, IS checked live against a numeric
// baseline, but that baseline is Typst's own in-float `<fp>` probes read via `mod.rs` (see the `float-fixture`
// branch there), not `expected.json`, since Typst's plain `query` reports an anchor's IN-FLOW position, not
// the position either fixture is checking (a floated or nested anchor's position after layout has moved it).

#set page(width: 595.276pt, height: 841.89pt, margin: 56.9pt)
#set text(size: 11pt, font: "Libertinus Serif")
#set par(justify: true, leading: 0.65em, spacing: 1.2em, first-line-indent: 0pt)

#let aside-box(float: true, body) = {
  let inner = box(
    width: 100%,
    inset: (x: 10pt, y: 10pt),
    fill: luma(240),
    stroke: (left: 2pt + luma(80)),
    [#body],
  )
  if float { figure(placement: auto, inner, caption: none) } else { inner }
}
#let claim-refs(..codes) = { }
#let claim-label(..codes) = {
  let strs = codes.pos().map(c => if type(c) == label { str(c) } else { str(c) })
  let display = strs.join(" ")
  box(width: 0pt, clip: false, context {
    let pos = here().position()
    let content = text(size: 6.5pt, fill: luma(90), hyphenate: false, display)
    let m = measure(content)
    place(dx: 595.276pt - 56.9pt - pos.x, box(width: m.width, content))
  })
}

An opening paragraph of plain prose, long enough to be a paragraph, so the first aside is met high on the
page and its midpoint sits in the upper half, drawing it to the top of the page above the prose.

#aside-box[A first aside carrying a claim code#claim-label(<A1>) inside its body, so a nested margin-note
anchor is recorded inside the float's own material rather than as a direct child of the float list.]

A second paragraph of plain prose keeps the cursor high enough that the next aside also goes to the top,
stacking beneath the first one in the top band and shifting the body down once more.

#aside-box[A second aside, met high as well, so it stacks in the top band under the first one.]

A closing paragraph of plain prose ends the document beneath the two stacked top asides.
