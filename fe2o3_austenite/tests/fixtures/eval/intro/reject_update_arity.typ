// oracle: rejects
#counter("c").update((5, 6))
#counter("c").update(n => n)
#context [#metadata(counter("c").get()) <probe>]
