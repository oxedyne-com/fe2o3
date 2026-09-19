// The Typst side of the oracle harness: includes a corpus root verbatim, then queries the compiled
// document for every heading's and figure's resolved page.
//
// `__ROOT__` is a placeholder, not a real Typst path: `tests/oracle/mod.rs` copies this file to a
// temporary sibling of the real corpus root, substituting the root's own file name, so a relative
// `#include` and a root-absolute `/assets/...` path resolve exactly as they do for a normal compile
// (Typst's default root is the input file's own directory, which the copy shares with the original).
//
// The result rides out through `#metadata`, keyed `<oracle-dump>`, as a plain array of records --
// `(kind, label, title, page)` -- rather than a `json.encode`d string: `typst query ... --field value`
// already serialises the metadata value as JSON on its own, so encoding it a second time here would
// only hand the driver a JSON string to decode before it could decode the JSON inside it. `title` is
// the heading's or the figure caption's own text, read where it is plain, and falls back to the empty
// string where it is not (an inline maths span or a glossary call) -- it is a debugging aid for a
// mismatch report, not a comparison key, since Austenite's own anchor keys are synthesised from a
// running count and a slug, not from the document's Typst labels.
#include "__ROOT__"

#context {
	let text-of(body) = {
		if type(body) == str { body }
		else if body.has("text") { body.text }
		else if body.has("body") { text-of(body.body) }
		else if body.has("children") { body.children.map(text-of).join("") }
		else { "" }
	}
	let heads = query(heading)
	let figs  = query(figure)
	// In-float placement probes (`<fp>`): a float's TRUE floated (page, y), read from a `#context
	// here().position()` marker laid out inside the floated box. Present only in a fixture that embeds them
	// (the float fixture); every other root queries none. `query(figure).location()` gives the float's
	// ANCHOR position, not where it lands, so this is what a float-placement parity check must read.
	let fps   = query(<fp>)
	let entries = heads.map(h => (
		kind:	"heading",
		label:	if h.has("label") { str(h.label) } else { "" },
		title:	text-of(h.body),
		page:	h.location().page(),
		y:	int(calc.round(h.location().position().y / 1pt)),
	)) + figs.map(f => (
		kind:	"figure",
		label:	if f.has("label") { str(f.label) } else { "" },
		title:	if f.has("caption") and f.caption != none { text-of(f.caption.body) } else { "" },
		page:	f.location().page(),
		// The figure's ANCHOR y (its in-flow declaration point), for the lenient count/page cross-check --
		// NOT where a float lands; the `<fp>` probe rows below carry the true floated position.
		y:	int(calc.round(f.location().position().y / 1pt)),
	)) + fps.map(m => (
		kind:	"floatpos",
		label:	"",
		title:	"",
		page:	m.value.page,
		y:	m.value.y,
	))
	[#metadata(entries) <oracle-dump>]
}
