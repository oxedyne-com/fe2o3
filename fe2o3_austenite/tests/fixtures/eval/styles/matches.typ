#metadata(1) <a>
#metadata(2) <b>
#metadata(3)
#context [#metadata(query(selector(<a>).or(<b>)).len()) <p>]
#context [#metadata(query(selector(<a>).and(metadata)).len()) <p>]
#context [#metadata(query(selector(<a>).and(<b>)).len()) <p>]
