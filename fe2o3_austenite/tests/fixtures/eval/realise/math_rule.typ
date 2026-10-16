// A text show rule reaches the symbols of an equation, one element at a time: `x` and the arrow
// shorthand are symbols in maths. Not in the corpus (`none`): its answer is the PDF's text, which
// `tests/eval_realise.rs` reads from the oracle and compares with Austenite's realised equation.
// oracle: none
#show "x": "Z"
#show "→": "R"

$ x + x -> x $
