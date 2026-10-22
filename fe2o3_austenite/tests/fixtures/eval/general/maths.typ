// Equations inline and in display, numbered.
#set page(width: 300pt, height: 300pt, margin: 30pt)
#set math.equation(numbering: "(1)")
Inline $a^2 + b^2 = c^2$ in text.
$ sum_(i=1)^n i = (n (n + 1)) / 2 $ <sum>
$ integral_0^1 x dif x = 1/2 $
See @sum.
#metadata($x_1 + y^2$) <probe>
#context [#metadata(counter(math.equation).get()) <probe>]
