// The comment-aware skip-scanner regression root (G3). `context_fixture.typ` already gates the reader
// refusing a `#context { ... }` block outright; this one gates a narrower failure in how far that block's
// skip reaches when a comment inside it mentions a brace. Before the fix, `//` and `/* ... */` were not
// recognised inside the multi-line bracket/brace skip scanner ([`SkipState`]), so a `}` written inside a
// comment -- `let c = 1 // }` -- popped the block's own brace early: the skip closed after the comment
// line, and the block's real tail (its remaining statements and its own closing `}`) leaked into the body
// as raw source. Deliberately self-contained, like `context_fixture.typ`, so it needs no symbol-modifier
// patch to compile under typst 0.15.1.
//
// The block below binds locals and emits nothing, so Typst evaluates it to no content and Austenite
// refuses-and-drops it whole: the two engines' pages agree. A regression that re-leaked the block's tail
// past the commented brace would set its remaining source lines as extra paragraphs, moving the pinned
// PDF hash and the heading pages the oracle order-matches against Typst.
#let doc(body, heading-font: "Libertinus Serif") = body
#show: doc.with(heading-font: "Libertinus Serif")

= A Commented Context Leak Fixture

This paragraph precedes the context block, and is written long enough to wrap onto more than one line so
the page carries real body text the raster sample can land on rather than a single short line where a
regression might hide.

#context {
 let c = 1 // a line comment mentioning a brace: c = 1 }
 /* a block comment mentioning a brace: } */
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
between them: Austenite refuses and drops the block whole, comments included, and Typst evaluates it to no
content, so the body is identical either way.

== A Second Heading

A second short section, so the fixture carries more than one heading for the oracle's order-matched
anchor-page comparison to line up against.
