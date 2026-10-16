// The term dictionary this fixture's `root.typ` shares with the reader: `book::install_term_dict` walks
// up from `root.typ`'s own directory looking for a file named exactly `terms.typ`, finds this one at hop
// zero, and reads its `term-dict` literal directly (see `book::parse_term_dict`) -- the same file real
// Typst imports below, so both engines resolve the same two keys to the same values.
#let term-dict = (
	"org":     "Elearnity Pty Ltd",
	"website": "elearnity.oxegen.io",
)
