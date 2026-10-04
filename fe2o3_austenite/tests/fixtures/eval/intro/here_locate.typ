// here(), locate() and a location's page, across page breaks.
#metadata(1) <a>
#context [#metadata((here().page(), locate(<a>).page(), locate(<b>).page())) <probe>]
#pagebreak()
#context [#metadata(here().page()) <probe>]
#metadata(2) <b>
#context [#metadata((here() == locate(here()), locate(<a>) == locate(<b>))) <probe>]
