// `plugin.transition` makes a new plugin from the state after a call; the old one is untouched, and the same
// call on each answers by its own state, never from the other's memoised result.
#let n(b) = array(b).at(0)
#let p = plugin("counter.wasm")
#let one = plugin.transition(p.inc)
#let two = plugin.transition(one.inc)
#let again = plugin.transition(p.inc)
#metadata((n(p.get()), n(one.get()), n(two.get()), n(p.get()), n(again.get()), n(two.get()))) <probe>
#n(two.get())
