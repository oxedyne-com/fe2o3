// Typst 0.15's `path` value: resolved where it is written, so a template's `path("d.json")` names the
// template's file wherever it is used; every loader takes one (parcio-thesis, kamk-thesis, gairm-import).
#import "pathlib/lib.typ": p
#repr(p) #(type(p) == path) #(p == path("pathlib/d.json"))
#json(p).b.at(1) #read(path("pathlib/d.json")).len()
#let q = path("/pathlib/../pathlib/d.json")
#repr(q) #repr(type(q))
