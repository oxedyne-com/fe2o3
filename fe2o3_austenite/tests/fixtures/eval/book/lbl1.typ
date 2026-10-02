// oracle: levels 2 4
#set page(width: 220pt, height: 260pt, margin: 20pt)
#set heading(numbering: "1.")
= Chapter <ch1>
Text @ch1 and #ref(<ch1>) and #link(<ch1>)[here] and #context [page #locate(<ch1>).page()].
#figure(rect(width: 20pt), caption: [Cap]) <fig1>
See @fig1 and @fig1[Fig].
#label("dyn") #metadata("x") <meta1>
#context query(<meta1>).first().value
