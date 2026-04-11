# TODO for fe2o3_jdat/dat_map

Improvements to the `FromDatMap` / `ToDatMap` procedural macros that would
lift the derive from "just sufficient" toward a first-class
Dat/JSON-through-Dat (de)serialisation facility for Hematite. Captured after
writing `fe2o3_net::acme::rfc8555`, where every item below was felt as
friction during development.

The derive is finicky and should be improved carefully in a dedicated
session, with `fe2o3_net/src/acme/rfc8555.rs` (a non-trivial real-world
user) used as a stress test for each change. None of the items below block
the ACME migration currently under way.

## Ranked from biggest usability win down

### 1. Nested-struct field support (biggest payoff)

Today a field typed `Vec<Challenge>` or `Option<Order>` or
`BTreeMap<String, Foo>` panics at derive expansion with
`"from_datmap: Cannot find an equivalent Dat for type ..."`. The supported
type list in `fe2o3_jdat/dat_map/src/lib.rs` is a fixed whitelist of
stringified type names: primitives, `String`, `Dat`, `Box<Dat>`,
`Vec<u8>`, `Vec<Dat>`, `Vec<String>`, `DaticleMap`.

Consequences:

- Every compound field in a protocol struct has to fall back to `Dat` or
  `Vec<Dat>` at the field site and the enclosing type has to expose a
  `typed_*()` helper that iterates and calls `T::from_datmap(...)`
  explicitly.
- `fe2o3_net::acme::rfc8555::Authorization::typed_challenges()` is one such
  hand-rolled unwrap helper; `fe2o3_steel::srv::cfg::ServerConfig::get_vhosts`
  and `::get_acme` are two more. Every new protocol type in the workspace
  will need another.

Desired behaviour:

- For any field of a type `T` where `T: FromDatMap`, emit
  `res!(T::from_datmap(...))` in place of the current `get_*` lookup.
- For `Vec<T>`, `Option<T>` and `BTreeMap<String, T>` where `T: FromDatMap`,
  unwrap the generic wrapper and recurse.
- Keep the existing primitive/`Dat`/`Vec<Dat>` fast paths as they are.

Implementation sketch:

- When the derive encounters a type it does not recognise, stop panicking.
  Instead, generate code that assumes `<T as FromDatMap>::from_datmap(...)`
  and lets rustc emit a trait-bound error at the call site if the user's
  type does not implement `FromDatMap`. This gives the user a clear,
  compilable-code error message instead of a derive panic.
- For `Vec<T>`, parse the generic argument, emit a loop that pulls each
  element out of a `Dat::List(v)`, calls `T::from_datmap` on each element's
  `Dat::Map`, and collects.
- For `Option<T>`, emit presence check plus `T::from_datmap`.

### 2. Native `Option<T>` support

Separate from (1) but related. Today there is no way to express "may be
absent **and** distinguish between absent and a default value". The
`#[optional]` attribute makes an absent field default to `Default::default()`
for the field type, which for `String` is `""` -- indistinguishable from a
present `""` in the wire payload.

Desired behaviour: a `pub x: Option<T>` field **with no attribute** means
"absent ⇒ `None`, present ⇒ `Some(T)`", no confusion with a default value.
`#[optional]` stays as a separate escape hatch for the value-with-default
case.

ACME example: `Order.certificate` defaults to `""` today but semantically
means "not yet issued". An `Option<String>` field would make that explicit
and survive typo-style bugs where the caller checks `if order.certificate.is_empty()`
when they meant `.is_some()`.

### 3. Error messages should carry field and struct names

Currently the derive-generated code calls `res!(m.get_string(...))` etc. and
the resulting error is something like `"expected Str, got X"`. Nothing in
that message tells you **which field** of **which struct** blew up, so
debugging a malformed CA response turns into a println/eprintln binary
search.

Desired behaviour: the derive wraps every field extraction with an
`err!` context carrying the field name and the enclosing struct name.
Something like `"while decoding field `new_nonce` of struct `Directory`"`.

Cheap to implement (quote!-interpolate the literals), huge debugging win.

### 4. JSON `null` versus missing key

RFC 8555 permits CAs to emit either `"error": null` or to omit the `error`
key entirely. Both paths currently end up indistinguishable: the field
defaults to `Dat::Empty`. This is the same shape as issue (2); resolving
both at once by adopting `Option<T>` semantics with explicit null handling
covers this case too.

### 5. `#[rename_all = "camelCase"]` at the struct level

The current attribute is `#[rename(name = "newNonce")]` -- wordy, and
easily repeated across every field of a camelCase wire format. Serde's
`#[serde(rename_all = "camelCase")]` at the struct level would let us
delete every per-field `#[rename(...)]` line in `rfc8555.rs` and remove a
whole class of typos.

Relatedly: the simpler per-field form `#[rename = "newNonce"]` (without the
`name =` wrapper) would match serde's de facto standard and be nicer to
type.

### 6. Support for simple enums mapped to `Dat::Str`

ACME status is a closed string set (`"pending"`, `"ready"`, `"valid"`,
`"invalid"`, `"processing"`, `"revoked"`, `"deactivated"`, `"expired"`).
Today we store it as `String` and compare with `==`, losing compile-time
safety.

Desired: `#[derive(FromDatMap, ToDatMap)]` works on unit-only enums whose
variants map to `Dat::Str` values. Serde supports this via
`#[serde(rename_all = "lowercase")]` at the enum level. The derive would
need to accept a second input kind (enum) and emit a match.

### 7. Compile-time errors should be actionable

Unsupported types currently hit
`unimplemented!("from_datmap: Cannot find an equivalent Dat for type '{}'.")`
at derive expansion time. That manifests as a derive panic, which is one
of the harder errors in Rust to interpret.

Desired: emit `compile_error!` with a message like
`"field `challenges` of struct `Authorization` has type `Vec<Challenge>`, which is not currently supported by `FromDatMap`. Either use `#[skip]` on the field, or change the field type to `Vec<Dat>` and write a typed getter, or add nested-struct support (see TODO #1)."`

### 8. Attribute hygiene

Attributes currently live in the global attribute namespace: `#[optional]`,
`#[skip]`, `#[rename(...)]`. A shared namespace such as `#[datmap(optional)]`,
`#[datmap(rename = "x")]`, `#[datmap(skip)]` would:

- make them easier to discover and grep for;
- reduce the risk of name collisions with other attribute macros;
- leave room to add new attribute options like
  `#[datmap(default = "expression")]` or
  `#[datmap(skip_serializing_if = "Vec::is_empty")]` without polluting the
  global namespace further.

## Practical takeaway

Items (1) and (5) would individually eliminate most of the boilerplate in
any fe2o3 module that parses a wire protocol. (2) closes a real semantic
gap that will bite us in future protocol parsers. (3) is cheap and makes
debugging bearable. The rest are quality-of-life polish.

None of these are prerequisites for finishing the ACME migration. This
list exists so the work is not lost and so whoever picks up the derive
improvements next starts with context for why each item matters, from a
real, non-trivial user of the derive (`fe2o3_net::acme::rfc8555`).
