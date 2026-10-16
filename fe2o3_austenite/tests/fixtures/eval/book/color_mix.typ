// oracle: levels 4
#set page(width: 200pt, height: 100pt, margin: 15pt)
#let c = rgb("#336699")
#c.mix(red).to-hex() #color.mix(c, red).to-hex() #c.mix(red, space: oklab).to-hex() #c.mix((red, 25%)).to-hex()
