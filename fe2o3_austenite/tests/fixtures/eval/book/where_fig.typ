// oracle: levels 2 4
#set page(width: 220pt, height: 260pt, margin: 20pt)
#show figure.where(kind: image): set align(right)
#show figure.where(kind: table): it => [TF #it.caption]
#show figure.where(kind: "algo"): it => [AF]
#figure(rect(), caption: [r])
#figure(table(columns: 1)[a], caption: [t])
#figure(kind: "algo", supplement: [Alg], caption: [c])[body]
