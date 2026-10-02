// oracle: levels 2 4
#set page(width: 220pt, height: 260pt, margin: 20pt)
#let wrap(..args, body) = block(..args.named(), body)
#let mk(prefix) = (x) => [#prefix#x]
#let f = mk("p-")
#f(3) #f[4]
#wrap(fill: red, inset: 4pt)[hi]
#let deco = (it) => it
#show: deco
#let apply(fn, ..xs) = xs.pos().map(fn).map(str).join(", ")
#apply(x => x * 2, 1, 2, 3)
