// oracle: rejects
// A plugin that returns 1 fails with its message.
#let p = plugin("err.wasm")
#p.fail(bytes("the input was bad"))
