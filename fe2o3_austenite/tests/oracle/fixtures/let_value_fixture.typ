// The scalar `#let` value-binding regression root (reader-completeness item 2). `content_bindings.rs`
// already exercises a `#let name = [ ... ]` content binding and a `#let f(p) = block/box(...)` furniture
// function unit-level; this is the oracle root proving the third shape -- a bare `#let name = <literal>`
// scalar (a string, an integer or a length) -- renders through the reader, not just past it. Before the
// fix, a reference to any of these three names rendered its raw `#name` text (the generic-call fallback
// has no group to fold in, so it falls through to plain text with the `#` kept); the fix substitutes the
// literal's own display text instead, in both a heading title and running prose. Deliberately self-contained
// (a local `#let doc`, plain-text body), so it needs no symbol-modifier patch to compile under typst 0.15.1.
#let doc(body, heading-font: "Libertinus Serif") = body
#show: doc.with(heading-font: "Libertinus Serif")

#let title = "Cheap Thinking Field Guide"
#let version = "1.2"
#let edition = 3

= #title

This paragraph precedes the reference-bearing ones, written long enough to wrap onto more than one line so
the page carries real body text the raster sample can land on rather than a single short line where a
regression might hide.

This is version #version, edition #edition, of the guide. A regression that stopped substituting either
scalar would print the raw source token instead of its value, moving both the running text and the pinned
PDF hash.

== #title, Edition #edition

A second heading also references both a string and a numeric scalar, so the fixture exercises substitution
in a heading title beyond the document's own first one, and carries a second heading for the oracle's
order-matched anchor-page comparison to line up against.

The edition number repeats here as plain prose: edition #edition, version #version.
