// The oracle harness's reverse-claim-index fixture -- the corpus's root for the D2 reverse claim-reference
// index. It exercises three things at once: `#claim-label(<code>)` sets a compressed code in the outside
// margin (the A1 marginalia primitive, as `marginalia_fixture.typ` does); `#claim-refs(<code>..)` registers
// a metadata-only reference, setting nothing visible; and a line-leading `#context { ... collect-claim-refs()
// ... }` appendix builds the reverse index -- for each claim code, in byte order, the pages it was referenced
// on. Self-contained -- no `#import` of any shared template -- so it compiles under typst 0.15.1 with no
// symbol-modifier patch, and austenite reads `#claim-label`/`#claim-refs` as builtins and recognises the
// `collect-claim-refs(` signature in the `#context` block without evaluating the block (it is hard-stratified
// and runs no `query`), building the same reverse index from the references it gathered walking the body.
//
// The page geometry is austenite's own `PageGeometry::a4()` (A4, uniform 56.9 pt margins) at the default
// 11 pt Libertinus Serif, so a referenced code lands on the same leaf in both engines. The body references a
// small set of codes spread across two pages, so the reverse index is non-vacuous: at least one code is
// referenced on more than one page, and the codes sort into byte order. The `claim-refs`/`claim-label`/
// `collect-claim-refs` definitions reproduce the book's `claims.typ` metadata-and-query machinery so Typst
// renders the same index austenite builds from the ledger.

#set page(width: 595.276pt, height: 841.89pt, margin: 56.9pt)
#set text(size: 11pt, font: "Libertinus Serif")
#set par(justify: true)

#let claim-ref-tag = "claim-ref"

// Register one or more claim codes at the current location. No visible output.
#let claim-refs(..codes) = {
  let code-strs = codes.pos().map(c => if type(c) == label { str(c) } else { str(c) })
  [#metadata((tag: claim-ref-tag, codes: code-strs))<claim-ref>]
}

// Query helper -- returns the metadata elements registered by claim-refs/claim-label.
#let collect-claim-refs() = {
  query(<claim-ref>)
}

// Place a compressed claim code in the outside margin AND register its metadata for the reverse index.
#let claim-label(..codes) = {
  let strs = codes.pos().map(c => if type(c) == label { str(c) } else { str(c) })
  let display = strs.join(" ")
  [#metadata((tag: claim-ref-tag, codes: strs))<claim-ref>]
  box(width: 0pt, clip: false, context {
    let pos = here().position()
    let page-w = 595.276pt
    let outside-m = 56.9pt
    let content = text(size: 6.5pt, fill: luma(90), hyphenate: false, display)
    let m = measure(content)
    let sized = box(width: m.width, content)
    if calc.odd(counter(page).get().first()) {
      place(dx: page-w - outside-m - pos.x, sized)
    } else {
      place(dx: outside-m - m.width - pos.x, sized)
    }
  })
}

= Reverse Claim Index

The opening paragraph invokes a foundational claim#claim-refs(<A1>) whose code is registered as a metadata-
only reference, setting nothing in the body column and nothing in the margin, while the reverse index records
the page it fell on. A second proposition#claim-refs(<B2>) is registered on the same leaf, so two distinct
codes share this first page and the index must list each against it.

A claim reference cannot resolve its own page inline, because the page a mark lands on is a forward layout
fact the paragraph has not yet fixed. The stratified answer is to weave a zero-width anchor at the point of
the reference, record where it lands, and read the page back from the converged ledger when the reverse index
is set as back matter. Nothing about the anchor occupies horizontal space, so the words flow as if it were
absent, and the same code#claim-refs(<A2>) referenced here joins the index under its own entry.

The compression the visible label applies collapses a run of three or more consecutive codes into a range,
and a single code sets as written. A visible label#claim-label(<C3>) both draws its code in the outside margin
and registers the same metadata for the reverse index, so a label and a bare reference contribute alike to the
page list, the label leaving ink in the margin and the reference leaving none.

Reproduction is the instrument. Every downstream document that leans on a reverse index is an opportunity to
harden the machinery, to break it against a real page and mend it upstream rather than papering over the gap
in the document that surfaced it. The reverse index is one such exercise, long missing from the reproduction
because a hard-stratified engine could not run the query the appendix is written with, and had to recognise
its shape instead and build the same index from the anchors it recorded walking the body.

The apparatus of a scholarly page is mostly its cross references, and mostly invisible until it is missing. A
claim referenced in the body and gathered into an appendix is a small thing on any one line and a substantial
thing across a book, the thread that ties a proposition to every place it is invoked. Reproducing it end to
end, from the reference mark to the sorted page list, is the difference between a document that looks right
and one that is right.

Enough prose now stands above this point to carry the column past the foot of the first page and onto the
second, so that a reference set below falls on a different leaf and the reverse index lists a code against two
distinct pages rather than one. The overlay draws each entry's pages from the same converged ledger the running
head reads, so neither can reopen the fixed point the composition settled on.

This second-page paragraph invokes the foundational claim#claim-refs(<A1>) once more, on a later leaf than its
first mention, so the index lists that code against two pages and proves the deduplicated, sorted page list is
built from real resolved folios rather than a single occurrence. A further reference#claim-refs(<B1>) registers
a fourth code, so the sorted index carries A1, A2, B1, B2 and C3 in byte order, each followed by the pages it
was referenced on.

== Index of Body References

Foundational claims are cited through invisible metadata. The reverse index records the pages where each claim
is invoked.

#context {
 let refs = collect-claim-refs()
 if refs.len() == 0 [
 _No body references registered._
 ] else {
 let by-code = (:)
 for r in refs {
 for code in r.value.codes {
 if code in by-code {
 by-code.insert(code, by-code.at(code) + (r.location(),))
 } else {
 by-code.insert(code, (r.location(),))
 }
 }
 }
 let codes = by-code.keys().sorted()
 for code in codes {
 let locs = by-code.at(code)
 let pages = locs.map(loc => str(counter(page).at(loc).first()))
 [#strong(code): #pages.join(", ").]
 linebreak()
 }
 }
}

== Sources

A section set after the reverse index, so its placement pins the listing above. In Typst the
`#context { ... collect-claim-refs() ... }` block sets in flow at its heading, so this section opens directly
beneath the last claim entry. A listing appended as back matter instead -- after every later section -- would
strand these entries below this heading rather than above it, leaving the "Index of Body References" heading
and its intro paragraph orphaned pages ahead of a headless run of codes. The section is here so the reverse
index's in-flow placement is a fact the oracle can see, not an assertion the comment alone carries.
