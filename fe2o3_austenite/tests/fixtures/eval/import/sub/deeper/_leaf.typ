// Two directories down: `..` reaches `sub/`, `../..` the root.
#import "../peer.typ": p
#import "../../_peer.typ": p as root_p
#let leaf = (p, root_p)
