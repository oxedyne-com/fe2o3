// Before and after bound a query by the first match of their bound; or and and combine.
// intro: needs selector
#metadata("a") <m>
#metadata((1, 2)) <m>
#metadata(none) <other>
#context [#metadata(query(selector(<m>).before(here()))) <probe>]
#metadata(3) <m>
#context [#metadata(query(selector(<m>).after(here()))) <probe>]
#context [#metadata(query(selector(<m>).before(<other>, inclusive: false))) <probe>]
#context [#metadata(query(selector(<m>).after(<other>))) <probe>]
#context [#metadata(query(selector(<m>).or(<other>))) <probe>]
#context [#metadata(query(selector(metadata).and(<m>))) <probe>]
#metadata(4) <m>
