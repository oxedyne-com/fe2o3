// oracle: levels 3
// A file in `sub/` imports by a path that climbs, one from the root, a sibling and a child two levels down.
#import "sub/_inner.typ" as inner
#inner.sees.a #inner.sees.b.len() #inner.sees.p, #inner.sees.leaf.join(" / ").
