// The sequence constructor casts each child as content: a symbol, a string and `none` are content
// (zero's `make-equation((sym.minus, [2]))`).
#let seq = $$.body.func()
#assert.eq(math.equation(seq((sym.minus, [2]))).body, $-2$.body)
#math.equation(seq((sym.minus, [2]))) #seq(("a", none, [b]))
