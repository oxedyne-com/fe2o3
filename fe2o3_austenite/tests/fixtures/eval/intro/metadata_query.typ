// Metadata, and queries by label, by element and by field.
#metadata("a") <m>
#metadata((1, 2)) <m>
#metadata(none) <other>
#context [#metadata(query(<m>)) <probe>]
#metadata(3) <m>
#context [#metadata(query(metadata.where(value: 3))) <probe>]
#context [#metadata(query(<other>)) <probe>]
#context [#metadata(query(<absent>)) <probe>]
#metadata(4) <m>
