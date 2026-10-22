// `first`, `last` and `at` are accessors: a mutating method or an assignment through them changes the array in place.
#metadata({
  let a = ((1, 2), (3, 4), (5,))
  a.last().push(6)
  a.first().push(0)
  a.at(1).push(9)
  a
}) <probe>
#metadata({
  let ctx = (groups: ((), ("a",)), names: (:))
  ctx.groups.last().push("b")
  ctx.groups.first().push("z")
  if ctx.groups.len() > 0 { ctx.groups.last().push("c") }
  ctx.names.at("k", default: none)
  ctx
}) <probe>
#metadata({
  let n = (1, 2, 3)
  n.last() = 9
  n.first() += 10
  n.at(1) *= 4
  n
}) <probe>
#metadata({
  let d = (rows: ((1, 2), (3,)))
  d.rows.last().push(4)
  let _ = d.rows.first().remove(0)
  d.rows.last().insert(0, 7)
  d.rows.first().last() = 5
  d
}) <probe>
