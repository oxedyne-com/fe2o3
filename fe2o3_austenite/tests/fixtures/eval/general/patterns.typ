// Patterns in parentheses: nested, spread, dictionary and placeholder, bound by `let`, `for` and parameters.
#let ((pa, pb)) = (1, 2)
#let (((pc, pd))) = (3, 4)
#let ((pe, pf), pg) = ((5, 6), 7)
#let (ph, (pi, pj), ..pk) = (8, (9, 10), 11, 12)
#let ((pl, ..pm)) = (13, 14, 15)
#let ((x: pn, ..po)) = (x: 16, y: 17, z: 18)
#let ((_, pp)) = (19, 20)
#metadata((pa, pb, pc, pd, pe, pf, pg, ph, pi, pj, pk, pl, pm, pn, po, pp)) <probe>
#metadata({
  let out = ()
  for ((k, ..r)) in (("a", 1, 2), ("b", 3)) { out.push((k, r)) }
  for ((l, m)) in ((1, 2), (3, 4)) { out.push((l, m)) }
  for ((k, v)) in (a: 1, b: 2) { out.push((k, v)) }
  for (n, (o, p)) in ((1, (2, 3)), (4, (5, 6))) { out.push((n, o, p)) }
  for ((n, (o, p))) in ((1, (2, 3)), (4, (5, 6))) { out.push((n, o, p)) }
  for ((x: q, ..w)) in ((x: 1, y: 2), (x: 3, y: 4, z: 5)) { out.push((q, w)) }
  out
}) <probe>
#let sum((x, y)) = x + y
#let prod(((x, y))) = x * y
#let diff = ((x, y)) => x - y
#let mix = (p, (x, y), ..rest) => p + x + y + rest.pos().len()
#metadata((sum((1, 2)), prod((3, 4)), diff((9, 4)), mix(1, (2, 3), 4, 5))) <probe>
#metadata({
  let a = 0
  let b = 0
  ((a, b) = (7, 8))
  (a, b)
}) <probe>
