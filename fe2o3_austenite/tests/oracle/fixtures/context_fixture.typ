// The oracle harness's `#context{ ... }` brace-form fixture -- the regression root for the bug found by
// adversarial QA of the Lucronics book: a line-leading `#context { ... }` code-block call (the brace twin
// of `#context[ ... ]`) was set VERBATIM as body text, leaking ~300 lines of source as prose in ch29.8.
// The reader now recognises and refuses the brace form the same way it refuses the bracket form, so the
// block never survives into the rendered output. Deliberately self-contained: no `#import` of any shared
// template, so it depends on nothing this crate does not own and needs no symbol-modifier patch (see
// `mod.rs`'s `prepare_patched_template_mirror`) to compile under typst 0.15.1.
//
// The `#context` block below binds locals and iterates without emitting any content, so Typst evaluates
// it to nothing and Austenite refuses-and-drops it: the two engines' pages agree. A regression that
// re-leaked the block would set its source lines as extra paragraphs, moving the pinned PDF hash and the
// heading pages that the oracle order-matches against Typst. The real ch29.8 block DOES emit a reverse
// index; there, evaluating it (generating that back-matter index) is a separate scheduled feature, and
// this fix only stops the leak -- the section renders absent-but-reported, not garbled.
#let doc(body, heading-font: "Libertinus Serif") = body
#show: doc.with(heading-font: "Libertinus Serif")

= A Context Leak Fixture

This paragraph precedes the context block, and is written long enough to wrap onto more than one line so
the page carries real body text the raster sample can land on rather than a single short line where a
regression might hide.

#context {
 let claim-refs = ("c1", "c2", "c3", "c4")
 let by-code = (:)
 for r in claim-refs {
  if r in by-code {
   by-code.insert(r, by-code.at(r) + 1)
  } else {
   by-code.insert(r, 1)
  }
 }
 let codes = by-code.keys().sorted()
 let total = codes.len()
}

This paragraph follows the context block. Both engines set exactly these two paragraphs and nothing
between them: Austenite refuses and drops the block, and Typst evaluates it to no content, so the body is
identical either way.

== A Second Heading

A second short section, so the fixture carries more than one heading for the oracle's order-matched
anchor-page comparison to line up against.
