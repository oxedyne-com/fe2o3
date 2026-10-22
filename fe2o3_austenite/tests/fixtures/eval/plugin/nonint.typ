// oracle: rejects
// The module loads; the check of the signature waits for the call.
#let p = plugin("nonint.wasm")
#p.wide(bytes("x"))
