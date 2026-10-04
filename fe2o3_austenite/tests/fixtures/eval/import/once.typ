// oracle: levels 3
// A module that warns is evaluated once, whichever way its path is spelled and whoever imports it.
#import "_warn.typ": w
#import "./_warn.typ": w as w2
#import "sub/_again.typ": w as w3
#w #w2 #w3
