// The string, array and dictionary library.
#let s = "Hello, World"
#metadata((s.len(), s.first(), s.last(), s.at(4), s.slice(0, 5), s.contains("World"), s.starts-with("He"), s.ends-with("ld"))) <probe>
#metadata((s.find("o"), s.position("o"), s.replace("o", "0"), s.split(", "), s.trim("H"), upper(s), lower(s), s.rev())) <probe>
#metadata(("a,b,,c".split(","), " pad ".trim(), "x" * 3, "abc".clusters(), "é".codepoints(), str(42), str(1.5), str(0x1F), int("12"), float("2.5"))) <probe>
#let arr = (3, 1, 4, 1, 5, 9, 2, 6)
#metadata((arr.len(), arr.first(), arr.last(), arr.sorted(), arr.dedup(), arr.rev(), arr.slice(2, 5), arr.contains(9), arr.find(x => x > 3), arr.position(x => x == 5))) <probe>
#metadata((arr.sum(), arr.product(), calc.max(..arr), calc.min(..arr), arr.map(x => x * 2), arr.enumerate().slice(0, 2), arr.zip(range(8)).slice(0, 2), arr.map(str).join(), (("a", "b").join(", ")), arr.chunks(3), arr.windows(7).len())) <probe>
#let d = (name: "Ada", year: 1815)
#metadata((d.keys(), d.values(), d.pairs(), d.at("name"), d.at("x", default: 0), d.len(), d.name)) <probe>
#let d2 = d
#{ d2.insert("field", "maths"); d2.remove("year") }
#metadata((d2, d)) <probe>
#metadata((("b", "a", "c").sorted(key: x => x), ((1, "z"), (0, "y")).sorted(key: p => p.at(0)))) <probe>
