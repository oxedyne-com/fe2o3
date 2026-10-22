// A rotated, scaled, skewed or moved body keeps its ink: text, shapes and nested boxes under each transform.
#set page(width: 300pt, height: 420pt, margin: 25pt)
A #rotate(45deg)[Rotated] B

#rotate(30deg, reflow: true)[Reflow]

#scale(50%)[Scaled]

#scale(x: 150%, reflow: true)[Reflow scaled]

#move(dx: 1cm, dy: 2pt)[Moved]

#skew(ax: 15deg)[Skewed]

#box(rotate(90deg)[InBox])

#rotate(45deg, rect(width: 1cm, height: 1cm, fill: aqua))
End.
