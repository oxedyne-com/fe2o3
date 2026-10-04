// A nested file: a path climbs with `..`, starts at the root with `/`, and names a sibling and a child.
#import "../_lib.typ": a
#import "/_lib.typ": b
#import "peer.typ": p
#import "deeper/_leaf.typ": leaf
#let sees = (a: a, b: b, p: p, leaf: leaf)
