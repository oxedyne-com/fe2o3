// oracle: levels 4
// The marker sits on the first baseline of the body, so a body that opens larger than the marker moves the
// marker down, and a marker larger than the body moves the body down.
#set page(width: 300pt, height: 400pt, margin: 25pt)
- small
- #text(size: 24pt)[Large first line] and a second line that wraps around the measure of the narrow page here
- small again
#set list(marker: text(size: 24pt)[#sym.bullet])
- small body beside a large marker
- another
