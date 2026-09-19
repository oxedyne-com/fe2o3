// The oracle harness's `#if media`-guarded include fixture -- the regression root for the bug found by
// adversarial QA of the elearnity books: the assembler reached a chapter's `#if media == "ebook" [ ... ]
// else [ ... ]` guard, followed BOTH branches, and printed the `#if ...[`, `] else [` and `]` marker
// lines as prose. Five books' Sources chapters (CheapThinking, UpsideDown, TheMarbleMachine, Oxpecker,
// TheGoldenSwan) doubled their back matter this way. The assembler now evaluates the guard and follows
// only the taken branch, dropping the other and its marker lines.
//
// `media` is bound in this root's own `#let` (a real book binds it in `config.typ` and the chapter reads
// `#import "config.typ": media`; the guard evaluator resolves the scalar from the config first, then from
// the guard's own file, so both answer). With `media == "ebook"` the ebook branch is taken: only
// `ebook_sources.typ` renders, `print_sources.typ` is dropped, and no `#if`/`else`/`]` line appears. A
// regression that followed both branches would set the print heading and its paragraph too, and the raw
// marker lines as prose -- moving the pinned PDF hash and the heading pages the oracle order-matches
// against Typst, which follows exactly the same one branch. Self-contained: no shared-template import,
// no `config.typ` (the doc idiom), so it needs no symbol-modifier patch to compile under typst 0.15.1.
#set page(width: 595.276pt, height: 841.89pt, margin: 56.9pt)
#set text(size: 11pt, font: "Libertinus Serif")
#set par(justify: true, leading: 0.65em, spacing: 1.2em, first-line-indent: 0pt)

#let media = "ebook"

= Sources and Notes

This chapter draws its source list from the edition being built. The `media` guard below selects the
full ebook source set and drops the condensed print set, so exactly one of the two branches reaches the
page in either engine.

#if media == "ebook" [
  #include "ebook_sources.typ"
] else [
  #include "print_sources.typ"
]
