// The oracle harness's cross-directory-include fixture -- the regression root for the bug found by
// adversarial QA of the Lucronics book: a CHAPTER's own `#include "../x.typ"` (not the book root's) was
// neither resolved nor reported, and the raw `#include "..."` line printed as literal body text while the
// included file's content -- figures and prose both -- vanished with no line in the `[austenite] skipped:`
// report. `chap_dynstrat_captonic_dynamics.typ:51` includes `../evidence/lucronics_evidence.typ` this
// way; this fixture reproduces the same shape (a chapter one directory down from the root, itself
// including a file from a SIBLING directory via `../`) without depending on the real book tree.
//
// Directory layout, mirroring Lucronics':
//   include_fixture/root.typ                     (this file)
//   include_fixture/chapters/chapter_one.typ      (the "book root" includes this)
//   include_fixture/evidence/evidence_one.typ     (the CHAPTER includes this, via `../evidence/...`)
//
// A build that stops following the chapter's own include regresses this root two ways at once: the
// evidence paragraph and figure disappear from the page/anchor counts below, AND the literal
// `#include "../evidence/evidence_one.typ"` line would reappear as a paragraph of body text, which also
// moves the pinned PDF hash. Either failure mode reds this fixture.

#set page(width: 595.276pt, height: 841.89pt, margin: 56.9pt)
#set text(size: 11pt, font: "Libertinus Serif")
#set par(justify: true, leading: 0.65em, spacing: 1.2em, first-line-indent: 0pt)

#include "chapters/chapter_one.typ"
