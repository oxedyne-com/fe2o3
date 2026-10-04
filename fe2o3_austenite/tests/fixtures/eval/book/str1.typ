// oracle: levels 2 4
#set page(width: 220pt, height: 260pt, margin: 20pt)
#let s = "The quick brown fox"
#s.split(" ").map(w => upper(w.first()) + lower(w.slice(1))).join(" ") \
#s.replace(regex("[aeiou]"), m => upper(m.text)) \
#s.codepoints().len() #s.clusters().len() #s.contains("fox") \
#str(12) #int("5") #float("2.5") #s.at(0) #s.position("q") \
#(s.len()) #lower(s) #s.starts-with("The") #s.ends-with("x") \
#"a,b;c".split(regex("[,;]")).join("|") \
#("x" * 3) #"abc".rev() #range(5).map(i => str(i)).join("-")
