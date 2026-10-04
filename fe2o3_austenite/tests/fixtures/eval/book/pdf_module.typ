// oracle: levels 1 4
#set page(width: 200pt, height: 100pt, margin: 15pt)
First #pdf.artifact(kind: "header")[marked] #pdf.artifact[plain] #pdf.artifact("text") last.
#pdf.attach("a.txt", bytes("hi"), relationship: "data", mime-type: "text/plain", description: "a note")
#context [#metadata((attach: type(pdf.attach), artifact: type(pdf.artifact), module: type(pdf))) <probe>]
