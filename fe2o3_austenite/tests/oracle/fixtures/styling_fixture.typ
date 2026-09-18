// The oracle harness's own styling fixture -- the corpus's first root that actually carries lowerable
// `#set` declarations (see `lang::set::lower_root_declarations` and `lang::set::lower_set`), giving the
// A1 lowering work a root the oracle can see. Deliberately self-contained: no `#import` of any shared
// template, so it depends on nothing this crate does not own and needs no symbol-modifier patch (see
// `mod.rs`'s `prepare_patched_template_mirror`) to compile under typst 0.15.1.
//
// `doc`'s `heading-font` names "Libertinus Serif" -- both engines' own default face -- on purpose: the
// point of this fixture today is proving the lowering PATH runs end to end (the value reaches the
// theme, the reader does not refuse the construct), not deliberately mismatching Austenite's and
// Typst's output. A later A1 lane naming a real second face here is expected to move this root's
// baseline (see `record_and_diff`'s `ORACLE_ACCEPT` flow) -- that is this fixture's whole purpose.
#let doc(body, heading-font: "Libertinus Serif") = body

#set text(size: 13pt)
#set par(leading: 16pt, first-line-indent: 18pt)
#show: doc.with(heading-font: "Libertinus Serif")

= A Styling Fixture

This paragraph is written long enough to wrap onto more than one line, so the lowered leading and
first-line indent are both visible in the rendered page rather than sitting on a single short line
where neither setting would show up in a raster comparison. The body text size is likewise set above
the engines' own default, so a regression in the size lowering shows as a visible reflow, not just a
number that quietly stops being read.

#figure(
  rect(width: 4cm, height: 3cm, fill: gray),
  caption: [A figure, so the corpus has a landmark page for the oracle's raster sample.],
) <fig-styling-fixture>

== A Second Heading

A second short section, so the document carries more than one heading for the anchor-page comparison
to order-match against.
