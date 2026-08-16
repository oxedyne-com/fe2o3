# SBJ v0 — Signed Binary JDAT

The file format of the oxeweb. An SBJ file is a signed envelope wrapping a tree
of typed nodes, encoded in BDAT (JDAT's binary form).

This specification is normative. Where it and the implementation disagree, the
implementation is wrong. The conformance fixtures in `fixtures/` are its teeth.

Companion design document: `~/usr/complement/projects/oxegen/doc/Oxeweb`.
Implementation plan: `~/usr/complement/projects/oxegen/plan/oxeweb_impl.md`.

---

## 1. File layout

```
+---------+------------+---------------------+---------------+
| header  | envelope   | tree, in BDAT       | index         |
|         | key, time, |                     | (optional,    |
| 8 bytes | hash, sig  |                     |  derived)     |
+---------+------------+---------------------+---------------+
                        \___________________/
                          hashed and signed
```

Read in order: header, envelope, tree, then anything trailing.

**The hash covers the tree region only.** Not the header, not the envelope, not
the index. Two files carrying the same tree are the same document at the same
address, whether or not either carries an index, and whoever holds a document may
compute, append, or discard its index freely.

### 1.1 Header

8 bytes, fixed:

| Offset | Bytes | Value | Meaning |
|---|---|---|---|
| 0 | 4 | `0x53 0x42 0x4A 0x00` (`SBJ\0`) | Magic |
| 4 | 2 | `u16` big-endian | Format major version. v0 is `0` |
| 6 | 2 | `u16` big-endian | Length of the envelope region, in bytes |

A reader that does not recognise the magic, or that reads a major version it does
not implement, stops. It does not guess.

### 1.2 Envelope

A BDAT-encoded `Dat::Map` with exactly these keys, all required:

| Key | Type | Meaning |
|---|---|---|
| `"schema"` | `str` | Payload schema, e.g. `"oxeweb/doc/0"` |
| `"author"` | `bu8` | Author's public key, raw bytes |
| `"sig_scheme"` | `u32` | Namex id of the signature scheme |
| `"hash_scheme"` | `u32` | Namex id of the hash scheme |
| `"time"` | `u64` | Unix milliseconds |
| `"hash"` | `bu8` | Hash of the tree region |
| `"sig"` | `bu8` | Signature over the signing input (§1.3) |
| `"tree_len"` | `c64` | Length of the tree region, in bytes |

The envelope map obeys the canonical encoding rules of §3, like everything else.

The `schema` key is what makes SBJ general. An oxeweb document declares
`"oxeweb/doc/0"` and its payload is a node tree (§4). A signed administrative
command would declare its own schema and carry a different payload. The container
does not care.

v0 defaults: Ed25519 signatures (matching the keys an oxenym already holds) and
SHA3-256 hashes (which is what `fe2o3_hash` implements). Both are named, never
assumed. A signature scheme may be replaced freely, since a signature is checked
once and discarded. A hash scheme may not, because **the hash is the address**.

### 1.3 Signing input

The signature covers, in this order, with no separators:

```
schema length (u32 BE) || schema bytes || sig_scheme (u32 BE) || hash_scheme (u32 BE)
    || time (u64 BE) || hash bytes
```

Signing the hash rather than the tree is what binds the document's permanent
address to its author. Including the schema and the scheme ids stops an attacker
re-labelling a signed payload as a different schema, or claiming a weaker hash
function produced the same address.

**The schema carries its length because it is variable-length and it is not the
last field.** Without the prefix the preimage is ambiguous: `schema` and `hash`
are both variable-length, with only fixed-width fields between them, so a byte
moved from the front of one field into the back of the one before it produces the
same bytes under a different reading. Two envelopes agreeing on nothing — a
different schema, a different pair of scheme ids, a different time and a different
hash — can share one signing input, and therefore one signature. A signature over
an ambiguous preimage does not say what the signer meant it to say, which is
exactly the property §1.3 exists to provide.

Whether that is reachable depends on what else the format admits, and v0 makes it
hard rather than impossible: one hash scheme, of one digest width, means the
alternative reading needs a hash of the wrong length, which §2 step 4 rejects.
The prefix is here because that is a fact about today's vocabulary and not about
the construction. A second hash scheme of a different width, or a second schema,
removes the accident that is doing the work. The moment to fix a preimage is
before there is more than one thing signed under it.

Only `schema` needs the prefix. `hash` is variable-length too, but it is the last
field, so its extent is whatever remains — there is nothing after it to steal
from or lend to.

### 1.4 Index (optional)

If present, the region after the tree is a BDAT-encoded `Dat::Map` from node id
(`c64`) to byte offset from the start of the tree region (`c64`). It is derived
data, outside the hash, and is never trusted: whatever it points at is verified
by decoding it. A reader may ignore it entirely.

---

## 2. Verification order

A document is verified before it is parsed, and content that fails is never
parsed at all.

1. Read the header. Check magic and major version.
2. Decode the envelope. Check every required key is present and typed correctly.
3. Check `tree_len` against the bytes available. A tree region shorter or longer
   than declared is a rejection, not a truncation.
4. Hash the tree region with the named hash scheme. Compare with `hash`.
   Mismatch is a rejection.
5. Verify `sig` over the signing input (§1.3) under `author`. Failure is a
   rejection.
6. Only now decode the tree, enforcing the depth limit (§5) *during* decoding.
7. Validate the decoded tree against the schema and the remaining limits (§5).

Steps 1 through 5 touch no content. A caller may perform them and never decode.

---

## 3. Canonical encoding

The hash is taken over the encoded bytes, so a document must encode to exactly
one byte string, or it has more than one address. Non-canonical bytes are
**rejected**, never accepted and silently re-encoded.

1. **Field types are fixed by the schema.** A heading's `level` is a `u8`. JDAT
   would happily encode the number 2 as a `u8` (two bytes) or an `i32` (five),
   and both decode to the same heading level, giving one document two addresses.
   The schema decides, and the decoder checks what it got.
2. **Maps are `Dat::Map`**, never `Dat::OrdMap`. `Dat::Map` is a `BTreeMap`, so
   key order follows the keys. `OrdMap` follows the author's typing.
3. **Map keys are strings** (`Dat::Str`), lowercase ASCII, and no key may appear
   twice.
4. **No redundant wrappers.** No `Dat::Box`. `Dat::Opt` only where the schema
   declares a field optional, and an absent optional field is omitted from the
   map rather than encoded as `none`.
5. **Strings are well-formed UTF-8, and in Unicode NFC**, with no unpaired
   surrogates and no control characters (the Unicode `Cc` category: C0, C1, and
   delete) other than tab and newline. Carriage return is rejected too, so one
   line ending has one encoding and cannot split a document's address from its
   twin.

   NFC is what closes the last route by which one logical document could hold two
   addresses. The letter é may be written as a single code point, or as an `e`
   followed by a combining acute accent. The two display identically, mean the
   same thing, and hash differently. Requiring the composed form makes the map
   from a document's meaning to its address a function again. The normalisation
   is `fe2o3_text`'s, over tables generated from a pinned Unicode Character
   Database, and it is verified against the Unicode Consortium's own conformance
   suite.
6. **Integers are exactly the declared width.** No promotion, no demotion.
7. **Lists are `Dat::List`**, not `Dat::Vek`, even where every element shares a
   kind.
8. **Nothing is carried that has no effect.** A thing a reader would render
   identically whether it were present or absent gives one document two
   encodings, and so two addresses. Three such things exist, and all are
   rejected:
   - a `styles` table that is empty,
   - a style record that is empty,
   - a style defined in the table that no node references.

   This is the same rule as rule 4's "no redundant wrappers", applied to the
   style table rather than to the encoding. It is stated separately because it
   cannot be checked while decoding: whether a style is referenced is only known
   once the whole tree has been walked.

The authoring compiler canonicalises before signing. The reader pays nothing,
since it is hashing the bytes anyway.

---

## 4. The node tree (`oxeweb/doc/0`)

### 4.1 How a node carries its kind

A node is a JDAT `usr` daticle: a `u16` kind code, then a `Dat::Map` of fields.

```
(heading|{
    "level":    (u8|2),
    "children": [(text|"Style without a cascade")],
})
```

The kind code sits *in front of* the payload on the wire, so the decoder knows a
node is a heading before it reads a byte of the heading. An unknown or forbidden
kind is refused on the spot, and the byte length JDAT puts in front of every
compound says exactly how far to seek to reach the next node. A `"kind"` field
inside the map would require reading the node to find out whether it was allowed
to.

### 4.2 v0 node kinds

Deliberately small. Growth beyond this is handled two ways: a compatible addition
carries a fallback (§4.5), and a breaking change bumps the schema version.

| Code | Kind | Fields | Children |
|---|---|---|---|
| 1 | `doc` | `title: str`, `lang: str`, `styles: map?` | flow* |
| 2 | `section` | `title: str?` | flow* |
| 3 | `para` | | inline* |
| 4 | `heading` | `level: u8` (1..=6) | inline* |
| 5 | `list` | `ordered: bool` | `item`+ |
| 6 | `item` | | flow* |
| 7 | `box` | | flow* |
| 8 | `image` | `hash: b32`, `alt: str`, `w: u32?`, `h: u32?` | none |
| 9 | `text` | *(the daticle is a `str`, not a map)* | none |
| 10 | `emph` | `strong: bool` | inline* |
| 11 | `link` | `to: address` | inline* |
| 12 | `code` | `lang: str?`, `text: str` | none |
| 13 | `quote` | `cite: str?` | flow* |

*flow* is `section`, `para`, `heading`, `list`, `box`, `image`, `code`, `quote`.
*inline* is `text`, `emph`, `link`.

Three further codes are **reserved**. They name facilities the engine has and a
document does not:

| Code | Kind | Fields | What it is | Where it is legal |
|---|---|---|---|---|
| 14 | `edit` | `name: str`, `placeholder: str?` | An editable text field | A chrome or an application tree |
| 15 | `surface` | `app: b32`, `alt: node+` | A pane an application paints | An application tree |
| 16 | `icon` | `name: str` | One of the engine's own icons | A chrome or an application tree |

**`oxeweb/doc/0` admits the kinds 1 to 13 and no others.** Its admitted set is
closed. The chrome's address bar and an application's form field are the same engine
facility, reached through the same `edit`, and a document simply cannot name it: a
document carrying any of the three is refused, whole, by the same rule that refuses a
`para` inside a `para`. That is what makes "a document is never a program" structural
rather than conventional. The kinds are reserved here, in the document's own schema,
so that the vocabulary a document is held to cannot grow into them by accident, and
so that a reader meeting one knows what it is refusing.

An `icon` is reserved for a reason of its own, and it is not that an icon is
dangerous. A document carries a picture as an `image`, which is a content hash: the
picture is the author's, it is held, and it is the same picture wherever it is read.
An icon is the opposite — a name by which a tree reaches the *reader's* drawing. A
document naming one would be letting whichever reader opened it supply the document's
content, and two readers would show two documents. Style may be the reader's, because
style is how a thing looks; content may not, because content is what the author said.
The icon names are also browser actions, and `back` means nothing in a document.

An `icon` names one of a **closed set**, and a name outside it is refused rather than
drawn as a gap:

```
back  forward  home  add  close  find
```

The set is closed for the same reason a reserved code is refused where an unknown
code may be admitted by a fallback (§4.5): a fallback exists for what the reader
cannot know, and the reader knows exactly which icons it has. A chrome naming an icon
this version does not draw is a fault in the chrome, and the validator is where it is
caught. Growing the set is a versioned decision, as growing the kinds is.

An `icon` carries **no geometry and no colour**, as a `surface` carries no geometry.
It is a glyph: it takes its size from `size` and its colour from `fill`, through the
universal `style` field, so an icon in a bar is sized and inked by the same two
properties as the text beside it.

`text` is the one exception to "payload is a map": its payload is a `Dat::Str`
directly, because a text run wrapping a single string in a map would double the
bytes of the commonest node in every document. `code` carries its source in a
`text` field rather than as children, since a listing is one preserved string
rather than a run of formatted spans.

A schema fixes two vocabularies, not one: the node kinds above, and the style
properties of §4.4. Both are closed, and a chrome tree is wider than a document in
both — it may carry an `edit`, and it may name `grid`, `border` and `shadow` —
because it is the same engine drawing a real interface. A document may do neither.

Every node with a map payload may also carry an optional **`style`** field
(§4.4), a string naming an entry in the document's style table. It is left out of
the rows above because it is universal.

A content hash reference (`image.hash`, and the `hash` form of an address) is a
`b32`, a fixed 32-byte string, matching the width of the v0 hash scheme
(SHA3-256). A variable-length byte string would let the same reference encode two
ways.

The root node of an `oxeweb/doc/0` payload is always `doc`.

### 4.3 Link addresses

A `link`'s `to` field is a typed address, not a string the renderer parses: a map
with exactly one entry, whose key selects the address kind.

```
(link|{ "to": { "name": "news.cricket" },        "children": [...] })
(link|{ "to": { "hash": (b32|9f86d081...) },     "children": [...] })
```

`name` carries a NAMES name (`str`); `hash` carries a content address (`b32`). An
address with no entry, more than one entry, an unknown key, or a mistyped value
is rejected. Making the address typed rather than a string means the decoder
tells a name from a hash, and a malformed target is refused at the door rather
than misread by the renderer.

### 4.4 Styling

The oxeweb replaces the web's cascade with locality. A node names a style; the
style is defined once, in the document's style table; and a short inherited set
flows down the tree. No rule reaches across the document, so a style error cannot
escape the node that made it.

The **style table** is the `doc` node's optional `styles` field: a `Dat::Map`
from style name (`str`, the same lowercase-ASCII form as a map key) to a **style
record**. A node's `style` field names an entry, which must exist.

```
(doc|{
    "title": "Style without a cascade", "lang": "en",
    "styles": {
        "callout": { "bg": "muted", "pad": (u8|3), "fill": "ink" },
        "lede":    { "size": (i8|1) },
    },
    "children": [
        (box|{ "style": "callout", "children": [ (para|{ "style": "lede",
            "children": [(text|"...")] }) ] }),
    ],
})
```

A style record is a `Dat::Map` carrying any of these optional properties, and
nothing else. **`oxeweb/doc/0` admits these eight and no others, and the eight are
frozen.**

| Property | Type | Inherited | Meaning |
|---|---|---|---|
| `fill` | `str` | yes | Text colour, a palette name |
| `size` | `i8` | yes | Type scale step; 0 is the reader's base |
| `lang` | `str` | yes | Language, BCP-47 |
| `dir` | `str` | yes | `ltr` or `rtl` |
| `bg` | `str` | no | Background colour, a palette name |
| `pad` | `u8` | no | Spacing scale index |
| `align` | `str` | no | `start`, `center`, `end`, or `justify` |
| `radius` | `u8` | no | Corner radius, on the spacing scale `pad` uses; 0 is a square corner |

`bg` and `pad` make a tinted, padded box a thing a document draws, and `radius`
says what shape that box's corners are. It adds no element, no geometry and no
authority, and a soft-cornered callout, pull-quote or listing is ordinary
typography — which is what tells it from the properties below, whose subject is
the layout of a bar and a shelf and which mean nothing in prose.

The style vocabulary is the schema's, exactly as the node vocabulary is (§4.2).
A chrome tree and an application tree admit the eight above and five more:

| Property | Type | Inherited | Meaning |
|---|---|---|---|
| `grid` | `u8` | no | Lay the children out as a grid whose tiles are at least this many base sizes wide, wrapping to as many columns as the width allows, and sharing the width out among them |
| `pack` | `u8` | no | Lay the children out in a row of tiles exactly this many base sizes wide, packed from the start edge and wrapping when they run out of room |
| `grow` | `u8` | no | Take this share of the room a packed row has left over; 0 takes none of it |
| `border` | `[str, u8]` | no | A line round the edge: a palette name, and a width in pixels |
| `shadow` | `u8` | no | How far the node stands off the surface behind it, in whole steps; 0 is lying flat on it |

```
"shelf": { "grid": (u8|14) },
"tile":  { "bg": "muted", "pad": (u8|3), "radius": (u8|2),
           "border": ["muted", (u8|1)], "shadow": (u8|1) },
```

The browser's chrome is an SBJ tree laid out by the same engine as a document, and
it is a real interface: a library of tiles, a navigation bar, a control with an
edge. These five are what it takes to draw one. They are the chrome's and not the
document's because a document is not an interface and must not be able to dress as
one — a published document cannot name them, whatever a later reader learns to
draw, so what a document may look like is settled by the vocabulary it was signed
under. A `shadow` is elevation, the language of a thing that lifts toward the hand,
and a paragraph claiming to float above its page is precisely that dressing-up; a
`border` is the line round the control with an edge. An application's tree admits
them for the same reason it admits `edit` and `surface`: it is a declared program,
drawn by the same engine.

That rule is enforced by **addressing**, not by the vocabulary. The band and the
bars are painted into pixmaps a document is never handed, so it cannot draw there
whatever it names, and a document that drew a perfect likeness of a control could
still make nothing happen by it — a document carries no program, and a link goes to
an address or nowhere. The vocabulary is drawn where it is because there is nothing
in prose for a shelf's layout to mean, not because a corner is dangerous. All are self-only but `grow`, which is the one property a node names for its
PARENT to read. It is still locality and not cascade -- a parent looks at the
children it is laying and at nothing else, so no node's width depends on anything
outside the row it sits in -- but it is the one place the rule reads sideways
rather than downwards. It is here because a bar of controls cannot be spelt
without it: a bar is some controls and one thing that fills, and something has to
take what the controls did not. A row whose leftover is narrower than one of its
own tiles wraps instead, and the grower, alone on the next line, takes the whole
width -- which is a bar folding on a narrow window, and falls out of the rule
rather than being a case in it.

`grid` and `pack` are two layouts, and a style naming both is **refused**. A grid
shares its width out among its tiles, which is what a shelf of cards wants -- a
row with a gap at its end reads as a row missing a card. A packed row leaves each
tile at the width it names and ends where its tiles do, which is what a bar of
controls wants -- a lone button stretched across half a window is not a button. A
style asking for both would leave the reader to choose, and which it chose would
be a fact about that reader rather than about the tree.

A property's **type is the same in every schema; only its admission differs**.
`grid` is a `u8` wherever it is legal, so the canonical encoding rules of §3 pin
its width without consulting the schema, and a document naming `grid` is refused
for naming it, not for how it wrote it.

A `border` is one property and not two: a colour with no width draws nothing, and
a width with no colour draws nothing, so the two are named together, as a
two-element list, and a half-written border cannot be spelt. Its width is in
pixels rather than on the spacing scale, because a border is a boundary and not a
measure of room: a hairline is one pixel at any text size.

A `shadow` names an **elevation** and not a shadow: how far the node stands off
what is behind it, and nothing about what that costs in pixels or in ink. The
offset, the softness and the colour are the reader's theme's, for the same reason
the palette is — how a shadow must be drawn depends entirely on what it is drawn
on, and only the theme knows that. A shadow is a dark stain, and a dark stain on a
dark page is nearly nothing, so a theme with a near-black page must stain far
harder than one with a white page to say the same thing. A style that named a
colour would work in one theme and be invisible in the other; a style that names a
height works in both. A shadow is ink and not room: it changes nothing about where
anything sits, so a node that names one occupies exactly the box it would have
occupied without it.

The **palette** names are `ink`, `muted`, `accent`, and `bg`. They are semantic,
not literal: the reader's theme resolves them to actual colours, so switching to a
dark palette recolours every document without touching one. Sizes are scale
steps, not pixels, so enlarging type reflows rather than clips. There are no raw
pixels and no hex anywhere in v0; that exactness is fixed mode's business, which
v0 defers.

**Reader preferences are applied after author styles and always win.** An author
declares intent — a scale step, a semantic colour — and the reader's base size,
palette, and direction have the final say.

A `styles` table with a non-string key, a style record with an unknown property
or an out-of-enum value, or a `style` field naming an entry that does not exist,
is rejected, naming the offending node or style. A record naming a property that
exists but that its schema does not admit — `grid` in a document — is rejected
saying so, and naming the schema that refused it and the trees it is legal in.
"Unknown style property `grid`" would be a lie told to an author whose chrome
draws one.

### 4.5 Unknown kinds and forward compatibility

A reader will meet node kinds it does not implement: a document written against a
later vocabulary, seen by an older client. The web renders unknown tags as their
raw children and never breaks, which is why it can never be versioned or made
strict. Hard rejection is the opposite failure: one unknown node would make a
whole document unreadable to every client that had not yet updated.

The oxeweb takes neither. A node whose kind code is outside the vocabulary is
permitted **only if** its payload is a map carrying a non-empty **`fallback`**: a
list of nodes drawn from the kinds the reader *does* know. A reader that does not
implement the kind renders and validates the fallback; a reader that does uses
the kind's own fields. An unknown kind whose payload is not a map, or that lacks a
non-empty fallback, is rejected.

```
(table 20|{
    "fallback": [ (list|{ "ordered": (false), "children": [
        (item|{ "children": [(text|"Q1 revenue: 1.2M")] }),
        (item|{ "children": [(text|"Q2 revenue: 1.5M")] }),
    ] }) ],
    "rows": [ ... ]           # a reader that knows kind 20 uses this
})
```

This is the same discipline the `surface` node already carries, a mandatory
semantic alternative, lifted to the vocabulary as a whole. It keeps "reject at the
door" for genuinely malformed content while letting the vocabulary grow within a
major version without stranding readers. The fallback is validated in full,
against the known schema; the unknown kind's other fields are not interpreted, but
are still held to the canonical encoding rules of §3, since they were decoded to
get here.

**A kind the reader knows, and the schema does not admit, is refused
unconditionally.** A fallback does not admit it, and there is no other way in.

The rule above is for a code this version has *never heard of*. A fallback earns
such a code its place because the reader cannot know what it means and can still
render something faithful: ignorance is the reason to be generous. The reserved
codes of §4.2 are the opposite case. The reader knows exactly what code 15 is, and
knows that `oxeweb/doc/0` admits it nowhere, so it refuses it on sight, fallback or
no fallback, naming the kind and saying that a document may not carry it.

The two paths must stay two paths. Collapsing them — treating a reserved code as
merely unknown, and letting a fallback wave it through — would let an author put a
`surface` in a document today, under a fallback that renders innocently, and have
every reader that later learned what code 15 meant begin honouring it: a document
that became a program by waiting. An unknown code is admitted by a fallback because
ignorance is the reason to be generous. A known and inadmissible code is refused
because knowledge is the reason not to be.

### 4.6 Node ids

Nodes are identified by their position in a depth-first, pre-order walk of the
tree, counting from 0 at the root. Ids are not stored in the document. They are
what the optional index (§1.4) maps to byte offsets, and what an error message
names when it rejects a node.

---

## 5. Limits

| Limit | Value | Enforced | What it stops |
|---|---|---|---|
| Tree region size | 4 MiB | Before decoding | Runaway documents. Media and fonts are references, not bytes; 100,000 words of prose is ~600 KB |
| Node count | 100,000 | During validation | Layout cost tracks nodes, not bytes |
| Nesting depth | 64 | **During decoding** | Stack exhaustion. A recursive decoder spends a stack frame per level, and a stack overflow aborts the process rather than returning an error, so a legal document must decode within a standard 2 MiB worker-thread stack. 64 levels dwarfs any real document, which nests perhaps 20 deep, and a tiny file describing a million nested boxes is the cheapest attack against a recursive decoder |
| Envelope size | 4 KiB | Before decoding | A header claiming a 64 KiB envelope should not be believed for free |

The depth limit is the decoder's, not the validator's, and applies before
anything has been verified. The others may be raised on evidence, which is
easier than imposing them later.

---

## 6. Rejection

Every rejection names the failing thing: the byte offset, or the node id and its
kind, and the rule broken. "Invalid document" is not an error message.

A document that fails at any step renders as an error card and is never
partially displayed. There is no repair, no quirks mode, and no best effort. The
web gave that up in 1993 and spent thirty years paying for it.

---

## 7. Conformance fixtures

`fixtures/` holds the format's teeth. Each fixture is a directory:

```
fixtures/<name>/
    doc.jdat      the document, in JDAT text form (the source of truth)
    doc.sbj       the canonical signed binary artefact
    meta.jdat     expected hash, expected node count, expected depth
```

Rejection fixtures carry `reject.jdat` instead, declaring the expected error
(rule broken, offset or node id) so that "it was rejected" cannot pass for "it
was rejected *for the right reason*".

The v0 suite must include, at minimum: an empty document; one paragraph; every
node kind once; a styled document exercising the style table, an inherited
property, and a self-only property; a link by name and a link by hash; an unknown
kind carrying a valid fallback (accepted); nesting at the depth limit and one past
it; a tree at the size limit and one past it; each canonicalisation rule of §3
violated exactly once; a truncated tree; a tree one byte longer than `tree_len`; a
corrupted hash; a corrupted signature; a signature by the wrong key; a forbidden
child (a `para` inside a `para`); an empty list; a heading with `level: 0` and one
with `level: 7`; a `style` naming an entry absent from the table; a style record
with an out-of-enum value; an unknown kind with no fallback (rejected); a document
carrying an `edit` node, one carrying a `surface` node, one carrying an `icon` node,
and one carrying a `surface` node that also carries a valid fallback, all four
rejected (§4.2, §4.5); an `icon` naming each of the closed set (accepted in a chrome
and an application tree) and one naming an icon outside it (rejected); a malformed
link address (two entries); and a document whose bytes are valid BDAT but not valid
SBJ.
