// oracle: levels 3
// Data read by a path resolves like an import, from the file it is written in.
#import "sub/_reader.typ": from_sub
#read("_data.txt") #from_sub.join(" | ")
