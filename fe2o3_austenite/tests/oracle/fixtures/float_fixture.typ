// The oracle harness's float fixture -- the corpus's first root exercising the A1 FLOAT primitive
// (`figure(placement: auto | top | bottom)`). Self-contained -- no `#import` of any shared template --
// so it compiles under typst 0.15.1 with no symbol-modifier patch, and austenite reads the `aside-box`
// furniture from its own `#let` definition exactly as it does in the books.
//
// The page geometry is austenite's own `PageGeometry::a4()` (A4, uniform 56.9 pt margins) set explicitly
// here so Typst lays the body on the same measure -- austenite ignores a lone `#set page(width/height)`
// and always uses A4, so the two only agree when the fixture names A4 (see `marginalia_fixture.typ` for
// the same idiom). The floats are aside-boxes carrying plain text, not `rect`/image figures: austenite
// renders an image figure as a measure-wide PLACEHOLDER and always sets a "Figure N." caption, so an
// image float could never match Typst, whereas an aside-box of text sets from the same body in both. Each
// aside-box is a `figure(placement: auto, ...)` under the bonnet, so it is a floating figure to Typst and
// a `Float` anchor to austenite -- the two the oracle order-matches, and for THIS root the harness holds
// them to exact page agreement and a per-float side (top/foot) and y check.
//
// The body is long and plain (both engines break it identically), so the document spans more than one page
// and the four asides are spread across it -- some drawn to the top, some to the foot, one deferring to a
// later page -- so the engine's queue, midpoint and document-order stacking are all exercised.
//
// REGRESSION GUARD, NOT A TYPST-PARITY GATE (this pass). Austenite's float placement is pinned here against
// ITS OWN baseline (expected.json's `floats`), so a placement regression fails loudly, but it is NOT yet
// Typst-exact: aside-2 goes to the FOOT under Austenite where Typst sets it at the TOP, and the cascade
// defers aside-3 to the next page where Typst keeps it at the foot -- a known midpoint subtlety tracked for
// a follow-up. Two obstacles keep this from being a live Typst-parity gate today: (1) Typst's `query`
// reports a float's ANCHOR position, not where it floats to, so the dump cannot be compared directly; the
// fix is to embed in-float `#context here().position()` probes once the midpoint is corrected, upgrading
// this to true Typst parity. (2) the midpoint itself must first be corrected to match Typst.

#set page(width: 595.276pt, height: 841.89pt, margin: 56.9pt)
#set text(size: 11pt, font: "Libertinus Serif")
#set par(justify: true, leading: 0.65em, first-line-indent: 0pt)

#let aside-box(float: true, body) = {
  let inner = box(
    width: 100%,
    inset: (x: 10pt, y: 10pt),
    fill: luma(240),
    stroke: (left: 2pt + luma(80)),
    [#text(size: 0.9em)[#body]],
  )
  if float { figure(placement: auto, inner, caption: none) } else { inner }
}

= Float Placement

This opening section runs on at plain length so that both engines break its lines at exactly the same
points and the reading cursor reaches the same depth in each. The words carry no emphasis and no special
construct, only ordinary prose set to the same measure, so that the page fills identically and the first
aside, when it is met, has the same space above it in both engines. We write on to fill the upper part of
the first page before the aside arrives, so its midpoint sits high and it is drawn to the top of the page.
Line after plain line, the same in both engines, the measure the same, the leading the same, the breaks
the same, until the aside is due.

#aside-box[A first aside, met high on the first page, so its midpoint sits in the upper half and it is
drawn to the top of the page above the prose that continues beneath it.]

The body continues after the first aside with a solid block of ordinary prose whose only job is to carry
the reading cursor down the first page, so that the second aside, arriving low, has its midpoint in the
lower half and is drawn to the foot. We keep writing plainly and at length, without emphasis or any
special construct, so that both engines break these lines at exactly the same points and the cursor
reaches the same depth in each before the second aside is met. A little more prose keeps the measure
honest and the paragraph long enough to matter for the placement decision that follows it, which is the
whole point of setting this fixture down at this deliberate length. Line after plain line accumulates,
each the same in both engines, until the page is better than half full and the midpoint rule has a clear
answer to give for the aside that comes next in the source, drawing it down to the foot of the page.

#aside-box[A second aside, met once the cursor has run well down the first page, so its midpoint falls in
the lower half and the engine draws it to the foot of the page instead of the top -- the midpoint rule
that tells an automatic float which end of the page it belongs on. It carries a couple of sentences so it
has real height, and its clearance is laid above it where it meets the body rather than below.]

Now a longer passage of prose carries the reader onto the second page and down it. There is nothing to
read here and nothing to see but text, which is the whole intent: text is what the two engines set
identically, so text is what makes their page breaks agree. We set it down line by line, plain and
unadorned, at the one size and the one measure, the leading fixed and the justification on, so the ragged
edge never enters into it and every line is full to the same width in both. On the prose goes, filling the
column, turning the page, resuming at the head of the next, the same words at the same places, carrying
the cursor down the second page toward its foot, so that the tall aside that comes next cannot fit in the
space that remains and must defer. We write on to be sure the cursor is deep when the tall aside is
reached, and that there is real prose after it to do the backfilling, because a float that fits where it
stands is not deferred and would not exercise the queue at all.

#aside-box[A third and much taller aside. It carries several sentences so that its height is large enough
that, arriving low on the second page, it cannot fit in the space that remains. It therefore defers to the
queue and is set at the top of the next page, while the prose written after it in the source flows up to
fill the foot of the second page. This is the backfill behaviour that distinguishes a real float queue
from the old in-flow lowering, and it is the reason a document with floats paginates as Typst does. A few
more sentences give the aside the height it needs to be certain of deferring rather than fitting in
whatever slack the page still holds, so the scenario is robust to small differences in where the cursor
happens to sit when the aside is reached, and the deferred aside opens the third page at its very top.]

A short paragraph of plain prose follows the tall aside in the source and backfills the foot of the second
page, set before the deferred aside on the page it was carried to. It is ordinary text, the same in both
engines, so it fills the same space in each and the backfill is identical.

#aside-box[A fourth aside, met just after the tall one, so it queues behind it and is set on the third
page beneath the deferred aside -- document order running down the top band, the earlier float highest.]

A closing paragraph of plain prose ends the document on the third page, part of the material that fills
the page the floats settled on, so the fixture finishes cleanly with body below the floats drawn there,
and both engines set that closing prose at the same place because it is, like all the rest, only text.
