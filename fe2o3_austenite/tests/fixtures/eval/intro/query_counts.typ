// Query results measured with array methods: every probe is itself metadata, so it is counted too.
// intro: needs array
#metadata(1) <m>
#context [#metadata(query(metadata).len()) <probe>]
#context [#metadata(query(selector(<m>).before(here())).len()) <probe>]
#metadata(2) <m>
#context [#metadata(query(<m>).map(it => it.value)) <probe>]
#context [#metadata((type(here()), query(<m>).first().location() == locate(selector(<m>).before(here(), inclusive: false).and(metadata.where(value: 1))))) <probe>]
