// oracle: levels 2 4
#set page(width: 220pt, height: 260pt, margin: 20pt)
#let d = (a: 1, b: (c: 2, d: (1, 2, 3)))
#d.b.d.at(1) #d.keys().join(",") #d.values().len() #d.pairs().map(((k, v)) => k).join(",")
#let (a, b) = d
#a #b.c
#let arr = (3, 1, 2)
#arr.sorted().map(str).join(",") #arr.fold(0, (x, y) => x + y) #arr.contains(2) #arr.enumerate().map(((i, v)) => i + v).len()
#let f(x, y: 2, ..rest) = x * y + rest.pos().len()
#f(3) #f(3, y: 3) #f(1, 2, 3)
#for (i, w) in ("a", "b").enumerate() [#i:#w ]
#if arr.len() > 2 [long] else [short]
#let r = while arr.len() > 5 { break }
