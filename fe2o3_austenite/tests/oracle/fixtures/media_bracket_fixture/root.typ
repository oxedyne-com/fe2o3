// The bracket-aware `#if` guard-extent regression root (G1). `media_fixture/root.typ` already gates the
// assembler evaluating a guard and following only its taken branch; this one gates a narrower failure in
// how far a taken branch's own EXTENT reaches. A real Sources chapter's guarded `#include` sits beside
// other prose that can itself open a nested content bracket -- an `#emph[...]` aside spanning more than
// one physical line is enough -- so the branch's body can carry a lone `]` line (that aside's own closer)
// well before the guard's own closing `]` is reached. The line-marker extent this fixture regresses
// against treated ANY bare `]` line as the guard's end: it closed on the aside's closer, took the guard's
// own `]`/`else`/`]` as leaked prose thereafter, and followed BOTH branches from that point on. The
// bracket-depth extent tracks the guard's own opening `[` at depth one and only its matching depth-one `]`
// closes it, so the aside's closer at depth two falls through as ordinary body text instead -- exactly
// what Typst's own reader does with it. The guarded `#include` itself stays bare, the real corpus shape.
#set page(width: 595.276pt, height: 841.89pt, margin: 56.9pt)
#set text(size: 11pt, font: "Libertinus Serif")
#set par(justify: true, leading: 0.65em, spacing: 1.2em, first-line-indent: 0pt)

#let media = "ebook"

= Sources and Notes

This chapter's source list carries an emphasised aside before the guard's own closer, so the taken
branch's body has a nested content bracket of its own -- and the `]` that closes it -- before the branch
itself ends.

#if media == "ebook" [
  #include "ebook_sources.typ"

  This sentence carries an aside worth noting: #emph[
  the aside spans more than one physical source line before its own closing bracket arrives
  ]
  and the paragraph continues here, well before the guard's own closer.
] else [
  #include "print_sources.typ"

  This sentence carries the same shape in the else branch: #emph[
  its own aside also spans more than one physical source line before closing
  ]
  so neither branch's inner closer can be mistaken for the guard's own.
]
