// Set and show rules, observed after realisation and in layout.
#set page(width: 300pt, height: 300pt, margin: 25pt)
#set text(size: 10pt)
#set heading(numbering: "1.a")
#show heading.where(level: 1): set text(size: 14pt)
#show "fox": [*FOX*]
#show emph: it => [\<#it.body\>]
#show <shout>: upper
= Chapter
The quick brown fox jumps over _the_ lazy dog.
== Section
Text in section. #text(fill: red)[Red text]. [loud] <shout>
== Another
#context [#metadata((text.size, par.leading, counter(heading).get())) <probe>]
