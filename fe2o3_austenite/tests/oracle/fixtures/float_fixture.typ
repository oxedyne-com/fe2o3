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
// a `Float` anchor to austenite -- the two the oracle order-matches.
//
// The body is long and plain (both engines break it identically), so the document runs to several pages
// and the four asides are spread across them, each drawn to the top or foot of its page by the midpoint
// rule. The oracle checks that the floats resolve to the same pages in both engines; the aside is drawn
// as a callout by austenite and as a plain box by Typst, so the two are not pixel-identical, but they
// paginate alike, which is what a float engine must get right.

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
A little more prose keeps the paragraph honest and long enough to matter. Line after plain line, the same
in both engines, the measure the same, the leading the same, the breaks the same, until the aside is due.
The cheap thing to do is to write clearly and at length, and that is what this fixture does, filling the
column with identical material so the breaker has plenty to break at the same places in either engine.

#aside-box[A first aside, met high on the first page, so its midpoint sits in the upper half and it is
drawn to the top of the page above the prose that continues beneath it.]

The body resumes and now runs at real length, a solid block of ordinary prose whose only job is to fill
the rest of the first page and carry the reader onto the second, so the asides that follow land on
definite pages that both engines agree on. We keep the sentences plain and the paragraphs long. The point
is a page break at the same place in both, and the way to be sure of it is to give the breaker plenty of
identical material to break. So we continue, unhurried, filling the column to its foot and beginning again
at the head of the next page, where the prose carries on exactly as before, the same words wrapping at the
same points, the cursor descending at the same rate. Nothing here is anything but text at a fixed size on
a fixed measure with fixed leading, which is precisely what the two engines set identically, line for line
and break for break, so that a page turned in one is a page turned in the other at the very same word.
We write on, plainly, to be sure the second page is well begun before the next aside is due, so that its
midpoint falls low and the foot of the page is the near end the midpoint rule chooses for it.

#aside-box[A second aside, met once the second page is well under way, so its midpoint falls in the lower
half and it is drawn to the foot of the page rather than the top, the near end the midpoint rule picks.]

Again the body resumes at length, a third solid block of plain prose to carry the reader down the second
page and onto the third. There is nothing to read here and nothing to see but text, which is the whole
intent: text is what the two engines set identically, so text is what makes their page breaks agree. We
set it down line by line, plain and unadorned, at the one size and the one measure, the leading fixed and
the justification on, so the ragged edge never enters into it and every line is full to the same width in
both. On the prose goes, filling the column, turning the page, resuming at the head of the next, the same
words at the same places, until the third page is reached and its upper part is filled in step by both
engines. We continue a while longer so the third aside, when it arrives, sits high on the third page and
is drawn by its midpoint to the top, above the prose that resumes below it on the same page.

#aside-box[A third aside, met high on the third page, so its midpoint sits in the upper half and it is
drawn to the top of that page, above the prose that resumes beneath it.]

A fourth block of plain prose follows, enough to give the third aside company on its page and to carry the
document onward. The words remain ordinary and the measure fixed, so the last stretch breaks the same in
both engines. We write on at plain length, filling the column and turning to the next page, so the fourth
aside is met low on its page and settles, by its midpoint, at the foot. A little more prose keeps the
cursor descending at the same rate in both engines, the same words at the same breaks, so the last aside
falls due at the same depth in each and is drawn to the same end of the same page. Then a few closing
sentences round the fixture off, plain to the last, so nothing but text decides where the final break
falls, and the two engines agree on it as they have agreed on every break before.

#aside-box[A fourth aside, met near the foot of its page, so its midpoint falls in the lower half and it
is set at the foot of the page, its clearance laid above it where it meets the body.]

A closing paragraph of plain prose ends the document, part of the material that fills the page the last
aside settled on, so the fixture finishes cleanly with body below every float that was drawn to a foot,
and both engines set that closing prose at the same place because it is, like all the rest, only text.
