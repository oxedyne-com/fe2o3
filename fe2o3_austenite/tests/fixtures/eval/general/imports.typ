// Imports from a helper module in the area.
#import "_lib.typ": greet, double
#import "_lib.typ" as lib
#metadata((double(21), lib.tau, lib.double(2))) <probe>
#metadata(greet("World")) <probe>
#greet("Reader")
