// The heading counter: numbered headings step their level, unnumbered ones do not.
// intro: needs heading numbering
#set heading(numbering: "1.a")
= A
#context [#metadata((counter(heading).get(), counter(heading).display())) <probe>]
== B
== C
#context [#metadata((counter(heading).get(), counter(heading).display(), counter(heading).display("I.1"), counter(heading).display(both: true))) <probe>]
#set heading(numbering: none)
= Unnumbered
#context [#metadata(counter(heading).get()) <probe>]
#set heading(numbering: "1.")
#counter(heading).update(5)
= D
#context [#metadata((counter(heading).get(), counter(heading).final(), query(heading).len(), query(heading.where(level: 2)).len())) <probe>]
#heading(level: 3)[E]
#context [#metadata((counter(heading).get(), counter(heading).display((..n) => n.pos().len()))) <probe>]
