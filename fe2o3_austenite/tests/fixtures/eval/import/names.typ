// oracle: levels 3
// Named imports, one renamed; the closure `g` still reads `hidden`, which was not imported.
#import "_lib.typ": a, b as other, g
Values #a and #other.first(), then #g(2): #other.map(str).join("-").
