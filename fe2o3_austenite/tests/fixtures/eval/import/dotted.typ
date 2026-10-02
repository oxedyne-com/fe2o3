// oracle: levels 3
// Three spellings of one path name one file: plain, with a leading `./`, and through a file that climbs to it.
#import "_lib.typ": a
#import "./_lib.typ": b
#import "sub/_up.typ": c
#a #b.len() #c
