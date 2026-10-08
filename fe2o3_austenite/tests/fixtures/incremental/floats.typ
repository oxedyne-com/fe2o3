// Floats: a figure and blocks placed at the top and the bottom of a page, one too tall for the room that is left and
// so deferred to the next page, and a reference to the figure from before and after it.
#set page(width: 220pt, height: 130pt, margin: 16pt, numbering: "1")
#set text(size: 9pt)

Opening text comes before the first float, with enough words to fill a few lines of the first page of this little document, see @fig-one.

#place(auto, float: true, clearance: 6pt)[#block(width: 100%, height: 44pt, fill: luma(220))[Float one]]

Text follows the float and runs on past it for a while, so that the float is set in the middle of a page before the flow carries on below it.

#place(top + center, float: true, clearance: 6pt)[#block(width: 80%, height: 30pt, fill: luma(200))[Float two]]

Another paragraph arrives here, and the second float will want the top of whichever page this one ends up on, pushing the text down the page.

#figure(placement: auto, rect(width: 60%, height: 26pt, fill: luma(235)), caption: [The figure.]) <fig-one>

The next paragraph talks about the figure again and sends the reader back to @fig-one so that the reference is read from both sides of it.

#place(bottom + center, float: true)[#block(width: 100%, height: 70pt, fill: luma(190))[Float three, tall]]

The last paragraph is here to carry on after the tall float, which is deferred to the next page when the room left on this one is too small for it.
