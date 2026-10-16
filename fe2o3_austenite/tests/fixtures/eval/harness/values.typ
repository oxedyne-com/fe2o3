// oracle: levels 1
// The harness's own serialisation check: every value here is one the harness also builds by hand
// in `tests/eval_oracle.rs`, so a reader or serialiser fault shows as a difference with Typst.
#metadata(1) <probe>
#metadata(-7) <probe>
#metadata(1.5) <probe>
#metadata(1.0) <probe>
#metadata("quoted \"text\", tab\tand ünïcode →") <probe>
#metadata(true) <probe>
#metadata(none) <probe>
#metadata((1, (2, 3), ())) <probe>
#metadata((b: 1, a: (c: "x"))) <probe>
#metadata(calc.inf) <probe>
#metadata(9007199254740993) <probe>
