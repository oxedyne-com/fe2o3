// A plugin from a path and from bytes: every exported function is a member, its arguments are bytes and so
// is its result (linguify, cetz, tiaoma).
#let p = plugin("echo.wasm")
#let q = plugin(read("echo.wasm", encoding: none))
#metadata((str(p.echo(bytes("hello"))), str(q.two(bytes("ab"), bytes("cd"))), p.none().len(), type(p), type(p.echo), repr(p.echo))) <probe>
#str(p.echo(bytes("set as text")))
#let f = p.two.with(bytes("x"))
#str(f(bytes("y")))
