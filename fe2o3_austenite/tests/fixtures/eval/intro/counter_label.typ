// A label-keyed counter counts the labelled elements, and its own updates.
#metadata(1) <x>
#context [#metadata(counter(<x>).get()) <probe>]
#metadata(2) <x>
#metadata(9) <y>
#context [#metadata((counter(<x>).get(), counter(<x>).at(<y>), counter(<x>).final())) <probe>]
#counter(<x>).update(10)
#metadata(3) <x>
#context [#metadata(counter(<x>).get()) <probe>]
