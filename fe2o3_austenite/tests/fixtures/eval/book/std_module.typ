// oracle: levels 4
#set page(width: 220pt, height: 120pt, margin: 15pt)
#let text(body) = std.text(red, body)
#let lorem = "shadowed"
#text[Red through the std module.] #lorem #std.lorem(3) #std.calc.abs(-4) #std.str(7)
#std.upper[std upper] #std.emph[std emph]
