// The oracle harness's own markup-builtins fixture -- the corpus root that exercises the block-position
// (own-line) Typst markup builtins the reader now sets rather than skipping: `#pagebreak()`, `#lorem(n)`
// and `#v(<abs len>)`. See `lang::parse`'s `builtin_opener`/`dispatch_capture` for the recognition path and
// `doc::Block::PageBreak` for the pagination side.
//
// Deliberately self-contained: no `#import` of any shared template, so it depends on nothing this crate
// does not own and needs no symbol-modifier patch (see `mod.rs`'s `prepare_patched_template_mirror`) to
// compile under typst 0.15.1. It carries two headings, so the anchor-page comparison has landmarks to
// order-match, and it spans two pages, so the forced `#pagebreak()` is proved to turn the page rather than
// merely be parsed: revert the builtin recognition and the reader skips all three, collapsing the document
// to a single page whose text loses the two `#lorem` paragraphs -- which moves this root's pinned PDF hash,
// so the fixture is non-vacuous.
//
// `#lorem` is checked at counts (45, 30) well inside the plain-Latin opening of the embedded corpus, where
// every word carries only commas and full stops, so the placeholder text sets byte-identically to the
// oracle. `#v(24pt)` is an absolute length, the only kind the reader sets; both engines add exactly 24pt.

= A Markup Builtins Fixture

An opening paragraph written long enough to wrap onto more than one line, so the first page carries real
body content above the forced break and the raster sample lands on set prose rather than a single short
line adrift near the top margin.

#lorem(45)

#v(24pt)

A short paragraph set after an explicit vertical space, so the `#v(24pt)` sits between two real blocks of
prose where its leading is visible in a raster comparison rather than collapsing at a page edge.

#pagebreak()

= After The Forced Break

This heading and the placeholder paragraph beneath it must land on the second page, proving the forced
page break actually turned the page rather than being parsed and dropped.

#lorem(30)
