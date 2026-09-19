// The "evidence" half of the cross-directory-include fixture: content reached only through a chapter's
// own `../` include, never included directly by the root. If this file is not followed at all, its
// figure and paragraph are simply absent from both the block stream and the anchor count -- the silent
// drop this fixture gates.

#figure(
  rect(width: 100%, height: 240pt, fill: luma(230), stroke: 1pt + luma(150)),
  caption: [A figure supplied entirely by a chapter's own cross-directory include.],
)

This paragraph is supplied entirely by the evidence file: it exists nowhere in the root or the chapter
that includes it, so it renders only if the chapter's own `#include "../evidence/evidence_one.typ"` line
is actually resolved and lowered rather than left to print as a literal line of body text.
