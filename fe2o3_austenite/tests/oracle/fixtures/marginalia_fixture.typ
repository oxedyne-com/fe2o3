// The oracle harness's marginalia fixture -- the corpus's first root exercising the A1 MARGINALIA
// primitive. A `#claim-label(<code>)` places a compressed claim code in the outside margin (6.5 pt grey,
// recto/verso aware) while setting nothing in the body text column; a `#claim-refs(<code>)` is metadata
// only and sets nothing at all. Self-contained -- no `#import` of any shared template -- so it compiles
// under typst 0.15.1 with no symbol-modifier patch, and austenite reads `#claim-label`/`#claim-refs` as
// builtins regardless of the `#let` definitions Typst uses to render them.
//
// The page geometry is austenite's own `PageGeometry::a4()` (A4, uniform 56.9 pt margins) and the body is
// set at austenite's default 11 pt Libertinus Serif, so the margin code lands at the same outer edge in
// both engines. The `claim-label` below reproduces the book's `claims.typ` placement: measure the code,
// then `place` it flush against the text block's outer edge -- its left edge at the block's right on a
// recto (odd) page, its right edge at the block's left on a verso (even) page. The document runs onto a
// second page on purpose, so a claim code is set on both a recto and a verso leaf.

#set page(width: 595.276pt, height: 841.89pt, margin: 56.9pt)
#set text(size: 11pt, font: "Libertinus Serif")
#set par(justify: true)

#let claim-refs(..codes) = { }
#let claim-label(..codes) = {
  let strs = codes.pos().map(c => if type(c) == label { str(c) } else { str(c) })
  let display = strs.join(" ")
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

= Marginalia

The opening paragraph carries a claim code#claim-label(<A1>) set in the outside margin of this recto
leaf, where the machinery seats a compressed proposition code in six-and-a-half point grey. The body
prose closes over the marker's place exactly as it would were the annotation absent, so the line breaks
and the page breaks are those of the plain paragraph and the code rides beside them without perturbing a
single measure of the running text.

A margin annotation cannot stream inline, because its extent would depend on a forward layout fact the
paragraph has not yet fixed. The stratified answer is to emit a zero-width anchor at the point of the call,
record where it lands, and draw the note in the outside margin only after the composition has converged.
Nothing about the anchor occupies horizontal space, so the words around it flow as if it were not there.

The claim family divides in two. A visible label registers its metadata for a reverse index and sets a
compressed code in the margin, while a bare reference registers the same metadata and sets nothing visible
at all. Both are consumed by the reader; only the label leaves ink, and that ink lives outside the text
column where a single-column engine could never reach before this primitive existed.

The compression rule collapses a run of three or more consecutive codes that share a letter prefix into a
range, leaves a pair expanded, and passes an unparseable code through untouched. A single code, the common
case in running prose, is set as written. The rule is a faithful port of the book's own claims machinery,
so a reproduced page reads the same codes the reference page does.

Reproduction is the instrument here. Every downstream document that leans on the margin is an opportunity
to harden the primitive, to break it against a real page and mend it upstream rather than papering over
the gap in the document that surfaced it. The margin code is one such exercise, long missing from the
reproduction because a single text column had nowhere to put it.

A second recto paragraph keeps the first page filling toward its foot, so that the material below it is
forced onto a fresh leaf and the reader can see a claim code set against the opposite edge. The mirror of
the margins is what makes the two edges symmetric: the binding sits at the spine on both sides of a leaf,
and the fore-edge carries the annotation.

The frame is laid once at the recto split, with the binding margin on the left, and a verso page is that
same frame shifted bodily to the fore-edge. Placing the folio at the block's left on a verso page therefore
lands it at the outer margin, and the margin code rides the very same mirror, so one placement rule serves
both parities without a special case for either.

Enough prose now stands above this point to carry the column past the foot of the first page and onto the
second, where the next annotated paragraph will set its code against the left edge of the block, in the
outside margin of a verso leaf. The overlay pass draws it there from the converged ledger, the same ledger
the running head reads, so neither can reopen the fixed point the composition settled on.

The stratification the primitive rests on is the discipline that user code may never observe a layout fact
directly. What it may see is the ledger: a map from an anchor's identity to the page and position it landed
on, filled during composition and content addressed by that identity, so a label keeps its meaning when a
paragraph carries it to another page. The margin note is the first construct to draw purely from that map.

Content addressing is what turns a convergence failure into a report rather than a guess. Two ledgers can be
differenced, and the difference names the anchor that moved and the pair of pages it moved between. A margin
anchor that stays put across passes contributes nothing to that difference, which is exactly why a page
carrying one still settles on the same fixed point a page without one would.

The anchor occupies no horizontal space and no vertical space, and it is never itself a legal breakpoint. A
glue beside it breaks as though it were absent, because the breaker looks straight through the anchor to the
box before it. This is the property the whole primitive turns on: weightlessness in the flow, so that the
body reads identically whether or not a code rides beside it.

A reproduction that could not place a margin note was a reproduction missing a whole register of the page.
The reference documents lean on the margin to carry their apparatus, their claim codes and their cross
references, and a single-column engine that dropped them silently was faithful only to the body. The
primitive restores the register without disturbing the column it sits beside.

Measuring the note is a boundary operation. Its width is taken at its natural set, so the code never wraps,
and that width decides where a verso note's right edge falls against the block. The measure runs the same
path the body does, through the shaper, so a code sets in the same face and at the same hinting as the prose
it annotates, only smaller and greyer.

The mirror of the margins is the quiet machinery under all of this. A leaf binds along one edge, so the
inside margin and the outside margin alternate between a recto and a verso, and the frame the driver lays at
the recto split is carried whole to the fore-edge on a verso. One shift, applied once after the page is
otherwise fixed, serves the folio, the running head and the margin note alike.

The apparatus of a scholarly page is mostly marginal, and mostly invisible until it is missing. A claim code
in the outside margin is a small thing on any one line and a substantial thing across a book, the thread that
ties a proposition in the body to its entry in an appendix. Reproducing it end to end is the difference
between a document that looks right and one that is right.

Enough further prose now stands here to be sure the column has crossed onto the second leaf, so that the
paragraph below sets its code in the outside margin of a verso page and the fixture exercises both parities
of the placement rule rather than only the recto one. The overlay draws each from the same converged ledger.

This paragraph opens the verso page and carries its own claim code#claim-label(<B2>) set against the outer
edge of the block, which on a left-hand leaf is the left edge. The reproduction has lacked exactly this for
as long as it has run a single text column, and the primitive closes the gap by drawing the note from the
ledger after the page is otherwise fixed.

A closing paragraph#claim-refs(<C3>) registers a metadata-only reference, which sets nothing in the margin
and nothing in the body, and the prose closes over it silently. The visible codes above it, one to a recto
and one to a verso leaf, are the whole of what this fixture asserts: that the margin annotation renders,
on both sides, at the corpus's own six-and-a-half point grey, in the outside margin, and nowhere else.
