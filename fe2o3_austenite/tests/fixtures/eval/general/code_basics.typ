// Bindings, destructuring, closures, control flow and operators.
#let (a, b, ..rest) = (1, 2, 3, 4)
#let (x: px, y: py) = (x: 10, y: 20)
#let add(p, q: 5) = p + q
#let fact(n) = if n <= 1 { 1 } else { n * fact(n - 1) }
#let acc = { let s = 0; for i in range(10) { s += i }; s }
#let w = { let n = 0; while n < 7 { n += 2 }; n }
#let brk = { let out = (); for i in range(10) { if i == 5 { break }; if calc.odd(i) { continue }; out.push(i) }; out }
#metadata((a, b, rest, px, py)) <probe>
#metadata((add(1), add(1, q: 2), fact(10), acc, w, brk)) <probe>
#metadata((7 / 2, 7.0 / 2, -7 / 2, 2 * 3 + 1, "ab" + "cd", "x" * 3, (1, 2) + (3,), (a: 1) + (b: 2))) <probe>
#metadata((1 < 2, "a" < "b", 1 == 1.0, (1, 2) == (1, 2), not true, true and false, false or true, 2 in (1, 2), "b" in "abc", "k" in (k: 1))) <probe>
#metadata((type(1), type(1.0), type("s"), type(none), type(auto), type(1pt), type((:)), type(())) .map(str)) <probe>
#let f = (..args) => args.pos().len() + args.named().len()
#metadata((f(1, 2, k: 3), (x => x * 2)(4), ((a, b)) => a + b)) <probe>
#metadata(range(3).map(i => i * i).filter(i => i > 0).fold(0, (s, i) => s + i)) <probe>
