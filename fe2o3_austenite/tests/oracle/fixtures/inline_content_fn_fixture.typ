// The inline mid-prose content-fn call regression root (reader-completeness item 3). `content_bindings.rs`
// exercises the splice, the argument substitution, the unknown-call refusal and the cycle guard at unit
// level; this is the oracle root proving an inline content-fn CALL used within a running paragraph --
// `see #term("website") for details` -- renders through the reader byte-faithfully to typst 0.15.1, with the
// prose before AND after the call kept. Before the fix, an own-line reference expanded but a mid-prose one
// was refused (a `#name(args)` call, its arguments dropped) or leaked its raw `#name` (a bare reference), so
// the words around it survived but the call itself never set its value. The fix splices each inline call's
// argument-substituted body into the surrounding prose at the position it stood, the inline twin of the
// own-line splice.
//
// Deliberately self-contained -- a local `#let doc`, a local `#let term` / `#let brand`, plain-text bodies
// wrapped only in `#emph[ ... ]` (an emphasis call both engines set identically in Libertinus) -- so it needs
// no symbol-modifier patch to compile under typst 0.15.1, unlike the two `oxeweb` roots.
//
// Non-vacuous: reverting the inline-expansion change makes `#term("...")` a refused call (its argument text
// dropped, the italic word gone) and `#brand` a raw `#brand` leak, so the set text and the pinned PDF hash
// both move. Reverting the argument substitution collapses every `#term(...)` call to the same parameter
// fallback, likewise moving the hash.
#let doc(body, heading-font: "Libertinus Serif") = body
#show: doc.with(heading-font: "Libertinus Serif")

#let term(w) = [#emph[#w]]
#let brand = [Oxegen]

= The Inline Field Guide, by #brand

Consult the #term("website") for the current schedule, and see #term("appendix") for the derivations, since
#brand keeps both of them current. The words before and after each inline call must survive intact, and each
call must set its own argument -- so #term("website") and #term("appendix") must read as two different italic
words, not one repeated fallback.

This second paragraph repeats the inline pattern: read the #term("glossary") first, then consult #brand, then
the rest of this sentence, so a regression that dropped an inline call or its surrounding prose would move both
the set text and the pinned PDF hash. The heading references #brand in its own title, so inline expansion is
proved in a heading as well as in running prose; it stands at level one, so it carries no section number under
either engine.
