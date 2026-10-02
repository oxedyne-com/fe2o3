// A labelled sequence is locatable: a label counter counts it and locate finds it.
// intro: needs par
#[Some text, then more.] <s>
#metadata(1) <s>
#context [#metadata((counter(<s>).get(), counter(<s>).final(), locate(<m>).page())) <probe>]
#metadata(2) <m>
