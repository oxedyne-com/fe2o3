// oracle: levels 4
// `marker-align` with a vertical part puts the marker against the body's first frame rather than on its baseline.
#set page(width: 300pt, height: 400pt, margin: 25pt)
#set list(marker-align: horizon)
- #text(size: 24pt)[Tall] \ item
- plain
#set list(marker-align: bottom)
- #text(size: 24pt)[Tall] \ item two with enough words to wrap onto another line of this narrow page
- plain
#set list(marker-align: top)
- #text(size: 24pt)[Tall] \ item
