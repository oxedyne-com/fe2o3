//! The v0 node vocabulary, and the schema each kind obeys.
//!
//! A node is a JDAT `usr` daticle: a `u16` kind code followed by the node's payload. The code sits
//! in front of the payload on the wire, so a decoder knows what a node is before it reads a byte of
//! it, and an unknown or forbidden kind is refused without inspecting its contents.

use crate::{
	SCHEMA_APP,
	SCHEMA_CHROME,
	SCHEMA_DOC,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;

/// The type a field must carry, checked against the decoded daticle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldType {
	/// A UTF-8 string.
	Str,
	/// An unsigned 8-bit integer.
	U8,
	/// A signed 8-bit integer.
	I8,
	/// An unsigned 32-bit integer.
	U32,
	/// A boolean.
	Bool,
	/// A fixed 32-byte string, the width of a v0 content hash.
	Hash32,
	/// A typed link address: a single-entry map naming a `name` or a `hash`. See [`check_address`].
	Address,
	/// A non-empty list of nodes, walked and validated like any other, and carried in a field rather
	/// than in `children`. It is what a `surface` names its semantic alternative by (§4.2).
	Nodes,
}

/// The schema a payload declares, and with it the vocabulary the payload is held to (`SPEC.md` §4.2).
///
/// The schema is the enforcement. "A document is never a program" is not a rule written on top of the
/// format; it is the fact that `oxeweb/doc/0` admits the kinds 1 to 13 and nothing else, that the
/// schema sits in the envelope, and that the envelope is signed (§1.3), so a payload cannot be
/// re-labelled into a vocabulary its author never claimed.
///
/// Each schema's admitted set is closed. A chrome tree may carry an `edit`, because the address bar
/// is one; an application's tree may carry an `edit` and a `surface`, because a game's pane is one;
/// and a document may carry neither, because a document that could name either would be a program.
///
/// A schema fixes two vocabularies, not one: the node kinds it admits ([`Schema::admits`]) and the
/// style properties it admits ([`Schema::admits_style`]). Both are closed, and both are the schema's
/// alone. What a property MEANS, and the width it is written at, are the same in every schema; only
/// whether a tree may name it at all depends on which schema the envelope declared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Schema {
	/// An oxeweb document: the kinds 1 to 13, neither reserved kind, and the seven document style
	/// properties.
	Doc,
	/// The browser's own chrome: the document kinds and style properties, the `edit` node, and the
	/// interface style properties.
	Chrome,
	/// An application's tree: everything a chrome admits, and the `surface` node.
	App,
}

impl Schema {

	/// The name an envelope declares this schema by.
	pub fn name(&self) -> &'static str {
		match self {
			Self::Doc	=> SCHEMA_DOC,
			Self::Chrome	=> SCHEMA_CHROME,
			Self::App	=> SCHEMA_APP,
		}
	}

	/// The schema a name declares, or an error naming the schema it was handed.
	///
	/// A payload declaring a schema this build does not read is refused rather than read as though it
	/// were one of these, since the container carries any schema at all (§1.2) and a reader that
	/// validates three must refuse the rest.
	pub fn from_name(name: &str) -> Outcome<Self> {
		match name {
			SCHEMA_DOC	=> Ok(Self::Doc),
			SCHEMA_CHROME	=> Ok(Self::Chrome),
			SCHEMA_APP	=> Ok(Self::App),
			_	=> Err(err!(
				"Schema '{}' is none of the schemas this validator reads: '{}', '{}', and '{}'.",
				name, SCHEMA_DOC, SCHEMA_CHROME, SCHEMA_APP;
			Invalid, Input, Mismatch)),
		}
	}

	/// Whether this schema admits the given reserved kind.
	///
	/// This is the whole of the reserved kinds' admission rule, and it is a table rather than a
	/// special case anywhere else: a surface is refused in a document because `oxeweb/doc/0` does not
	/// admit it here, and for no other reason.
	pub fn admits(&self, reserved: ReservedKind) -> bool {
		match (self, reserved) {
			(Self::Doc,	_)	=> false,
			(Self::Chrome,	ReservedKind::Edit)	=> true,
			(Self::Chrome,	ReservedKind::Icon)	=> true,
			(Self::Chrome,	ReservedKind::Surface)	=> false,
			(Self::App,	_)	=> true,
		}
	}

	/// Whether this schema admits the given style property (§4.4).
	///
	/// This is the whole of the style vocabulary's admission rule, and it is a table for the same
	/// reason [`Schema::admits`] is one: `grid` is refused in a document because `oxeweb/doc/0` does
	/// not admit it here, and for no other reason.
	///
	/// The v0 document vocabulary is frozen at the eight properties of [`STYLE_FIELDS`]. A published
	/// document cannot name an interface property, whatever a later reader learns to draw, so a
	/// document's appearance is settled by the vocabulary it was published under.
	pub fn admits_style(&self, prop: &StyleField) -> bool {
		match (self, prop.scope) {
			(Self::Doc,	StyleScope::Doc)	=> true,
			(Self::Doc,	StyleScope::Interface)	=> false,
			(Self::Chrome,	_)	=> true,
			(Self::App,	_)	=> true,
		}
	}

	/// The style-record property of the given name that this schema admits, if it admits one.
	///
	/// A property this schema does not admit comes back as `None` exactly as an unknown one does, so
	/// no caller can reach a property by name without passing the admission rule first. A caller that
	/// must tell the two apart, to say which it refused and why, asks [`known_style_field`] as well.
	pub fn style_field(&self, name: &str) -> Option<&'static StyleField> {
		match known_style_field(name) {
			Some(field) if self.admits_style(field)	=> Some(field),
			_	=> None,
		}
	}

	/// The tree this schema describes, as an error message names it.
	pub fn tree_label(&self) -> &'static str {
		match self {
			Self::Doc	=> "a document",
			Self::Chrome	=> "a chrome tree",
			Self::App	=> "an application tree",
		}
	}

	/// The vocabulary this schema admits, as an error message spells it.
	pub fn admits_label(&self) -> &'static str {
		match self {
			Self::Doc	=> "the kinds 1 to 13 and no others",
			Self::Chrome	=> "the kinds 1 to 13 and the reserved kinds 'edit' and 'icon', and no \
				others",
			Self::App	=> "the kinds 1 to 13 and the reserved kinds 'edit', 'surface' and 'icon', \
				and no others",
		}
	}

	/// The style vocabulary this schema admits, as an error message spells it.
	pub fn style_admits_label(&self) -> &'static str {
		match self {
			Self::Doc	=> "the eight document properties 'fill', 'size', 'lang', 'dir', 'bg', \
				'pad', 'align' and 'radius', and no others",
			Self::Chrome	=> "the eight document properties and the interface properties 'grid', \
				'pack', 'grow', 'max', 'border', 'shadow', 'nowrap' and 'ends', and no others",
			Self::App	=> "the eight document properties and the interface properties 'grid', \
				'pack', 'grow', 'max', 'border', 'shadow', 'nowrap' and 'ends', and no others",
		}
	}
}

/// One field of a node's payload map.
#[derive(Clone, Copy, Debug)]
pub struct Field {
	/// The map key.
	pub name:	&'static str,
	/// The type the value must carry.
	pub typ:	FieldType,
	/// Whether the field may be omitted.
	pub opt:	bool,
}

/// What a node may contain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Content {
	/// No children at all.
	None,
	/// Flow content: sections, paragraphs, headings, lists, boxes, images.
	Flow,
	/// Inline content: text runs, emphasis, links.
	Inline,
	/// List items only.
	Items,
}

/// The v0 node kinds. Deliberately small; growth is a versioned decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
	/// Root of a document.
	Doc,
	/// A titled division.
	Section,
	/// A paragraph.
	Para,
	/// A heading, levels 1 to 6.
	Heading,
	/// An ordered or unordered list.
	List,
	/// One item of a list.
	Item,
	/// A generic structural box.
	Boxx,
	/// An image, referenced by content hash.
	Image,
	/// A run of text. Its payload is a string rather than a map.
	Text,
	/// Emphasised inline content.
	Emph,
	/// A link to a name or a hash address.
	Link,
	/// A preserved run of source code.
	Code,
	/// A block quotation.
	Quote,
}

impl NodeKind {

	/// The wire code for this kind.
	pub fn code(&self) -> u16 {
		match self {
			Self::Doc	=> 1,
			Self::Section	=> 2,
			Self::Para	=> 3,
			Self::Heading	=> 4,
			Self::List	=> 5,
			Self::Item	=> 6,
			Self::Boxx	=> 7,
			Self::Image	=> 8,
			Self::Text	=> 9,
			Self::Emph	=> 10,
			Self::Link	=> 11,
			Self::Code	=> 12,
			Self::Quote	=> 13,
		}
	}

	/// The kind for a wire code, or an error naming the code the document schema does not admit.
	///
	/// A reserved code (§4.2) is not a document kind, so it is an error here like any other code
	/// outside 1..=13. The error says which of the two it is, because the two are refused for
	/// opposite reasons: an unknown code may be admitted by a fallback (§4.5), and a reserved code
	/// never is.
	pub fn from_code(code: u16) -> Outcome<Self> {
		match code {
			1	=> Ok(Self::Doc),
			2	=> Ok(Self::Section),
			3	=> Ok(Self::Para),
			4	=> Ok(Self::Heading),
			5	=> Ok(Self::List),
			6	=> Ok(Self::Item),
			7	=> Ok(Self::Boxx),
			8	=> Ok(Self::Image),
			9	=> Ok(Self::Text),
			10	=> Ok(Self::Emph),
			11	=> Ok(Self::Link),
			12	=> Ok(Self::Code),
			13	=> Ok(Self::Quote),
			_	=> match ReservedKind::from_code(code) {
				Some(reserved) => Err(err!(
					"Node kind code {} is the reserved kind '{}', which is legal in {} and never \
					in a document (SPEC.md §4.2).", code, reserved.label(), reserved.legal_in();
				Invalid, Input)),
				None => Err(err!(
					"Unknown node kind code {}, the v0 document vocabulary runs from 1 to 13, and \
					the codes 14 to 16 are reserved.", code;
				Invalid, Input)),
			},
		}
	}

	/// The label used in JDAT's text form, e.g. `(heading|{..})`.
	pub fn label(&self) -> &'static str {
		match self {
			Self::Doc	=> "doc",
			Self::Section	=> "section",
			Self::Para	=> "para",
			Self::Heading	=> "heading",
			Self::List	=> "list",
			Self::Item	=> "item",
			Self::Boxx	=> "box",
			Self::Image	=> "image",
			Self::Text	=> "text",
			Self::Emph	=> "emph",
			Self::Link	=> "link",
			Self::Code	=> "code",
			Self::Quote	=> "quote",
		}
	}

	/// The fields this kind's payload map must and may carry.
	pub fn fields(&self) -> &'static [Field] {
		match self {
			Self::Doc => &[
				Field { name: "title",	typ: FieldType::Str,	opt: false },
				Field { name: "lang",	typ: FieldType::Str,	opt: false },
			],
			Self::Section => &[
				Field { name: "title",	typ: FieldType::Str,	opt: true },
			],
			Self::Para => &[],
			Self::Heading => &[
				Field { name: "level",	typ: FieldType::U8,	opt: false },
			],
			Self::List => &[
				Field { name: "ordered",	typ: FieldType::Bool,	opt: false },
			],
			Self::Item => &[],
			// A box declares no fields of its own. The universal `style` field is not listed here,
			// nor on any other kind, because §4.4 gives it to every map payload alike, and both the
			// canonical check and the validator handle it before they consult this table.
			Self::Boxx => &[],
			Self::Image => &[
				Field { name: "hash",	typ: FieldType::Hash32,	opt: false },
				Field { name: "alt",	typ: FieldType::Str,	opt: false },
				Field { name: "w",	typ: FieldType::U32,	opt: true },
				Field { name: "h",	typ: FieldType::U32,	opt: true },
			],
			Self::Text => &[],
			Self::Emph => &[
				Field { name: "strong",	typ: FieldType::Bool,	opt: false },
			],
			Self::Link => &[
				Field { name: "to",	typ: FieldType::Address,	opt: false },
			],
			Self::Code => &[
				Field { name: "lang",	typ: FieldType::Str,	opt: true },
				Field { name: "text",	typ: FieldType::Str,	opt: false },
			],
			Self::Quote => &[
				Field { name: "cite",	typ: FieldType::Str,	opt: true },
			],
		}
	}

	/// What this kind may contain.
	pub fn content(&self) -> Content {
		match self {
			Self::Doc	=> Content::Flow,
			Self::Section	=> Content::Flow,
			Self::Para	=> Content::Inline,
			Self::Heading	=> Content::Inline,
			Self::List	=> Content::Items,
			Self::Item	=> Content::Flow,
			Self::Boxx	=> Content::Flow,
			Self::Image	=> Content::None,
			Self::Text	=> Content::None,
			Self::Emph	=> Content::Inline,
			Self::Link	=> Content::Inline,
			Self::Code	=> Content::None,
			Self::Quote	=> Content::Flow,
		}
	}

	/// Whether this kind is flow content, permitted where a section's children go.
	pub fn is_flow(&self) -> bool {
		matches!(self,
			Self::Section | Self::Para | Self::Heading | Self::List | Self::Boxx | Self::Image
			| Self::Code | Self::Quote)
	}

	/// Whether this kind is inline content, permitted inside a paragraph.
	pub fn is_inline(&self) -> bool {
		matches!(self, Self::Text | Self::Emph | Self::Link)
	}

	/// Whether a child of the given kind may appear inside this one.
	pub fn allows(&self, child: &Self) -> bool {
		match self.content() {
			Content::None	=> false,
			Content::Flow	=> child.is_flow(),
			Content::Inline	=> child.is_inline(),
			Content::Items	=> matches!(child, Self::Item),
		}
	}

	/// Whether this kind's payload is a bare string rather than a map. Only `text` is.
	pub fn payload_is_str(&self) -> bool {
		matches!(self, Self::Text)
	}

	/// Whether this kind must carry at least one child, the `+` of the SPEC §4.2 vocabulary.
	///
	/// A list with no items is a construction error rather than intent, so the format refuses it. A
	/// document or section may be empty, since a stub with only a title is a legitimate thing to
	/// publish.
	pub fn requires_child(&self) -> bool {
		matches!(self, Self::List)
	}
}

/// A node kind the format reserves, and which the document schema admits nowhere (`SPEC.md` §4.2).
///
/// These are the engine's own facilities: an editable text field, a pane an application paints, and
/// the engine's own icons. The chrome's address bar and an application's form field reach the same
/// `edit`, and a document may name none of the three, which is what makes "a document is never a
/// program" structural rather than conventional. Which trees may name them is [`Schema::admits`], and
/// nothing else.
///
/// An `icon` is reserved for a reason of its own, and it is not that an icon is dangerous. A document
/// carries its pictures as an `image`, which is a content hash: the picture is the author's, it is
/// held, and it is the same picture wherever it is read. An icon is the opposite -- a name the tree
/// reaches the *reader's* drawing by. A document naming one would be letting whichever reader opened
/// it supply its content, and two readers would show two documents. Style may be the reader's, because
/// style is how a thing looks; content may not, because content is what the author said.
///
/// The codes are known here rather than left outside the vocabulary on purpose. A code this version
/// has never heard of may be admitted by a fallback (§4.5), because the reader cannot know what it
/// means and a fallback lets it render something faithful anyway. A reserved code is the opposite
/// case: the reader knows exactly what it has been handed, and knows the schema it was handed under
/// does not admit it, so it refuses it whether or not a fallback is offered. Knowing the code is what
/// makes that refusal possible, and what stops a document smuggling a `surface` past a reader that
/// has not yet learnt what code 15 means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReservedKind {
	/// An editable text field, code 14.
	Edit,
	/// A pane an application paints, code 15.
	Surface,
	/// One of the engine's own icons, code 16.
	Icon,
}

impl ReservedKind {

	/// The wire code this kind is reserved at.
	pub fn code(&self) -> u16 {
		match self {
			Self::Edit	=> 14,
			Self::Surface	=> 15,
			Self::Icon	=> 16,
		}
	}

	/// The reserved kind a wire code names, or `None` if the code reserves nothing.
	pub fn from_code(code: u16) -> Option<Self> {
		match code {
			14	=> Some(Self::Edit),
			15	=> Some(Self::Surface),
			16	=> Some(Self::Icon),
			_	=> None,
		}
	}

	/// The label an error message names this kind by.
	pub fn label(&self) -> &'static str {
		match self {
			Self::Edit	=> "edit",
			Self::Surface	=> "surface",
			Self::Icon	=> "icon",
		}
	}

	/// The label with its indefinite article, so that a refusal reads as English.
	pub fn with_article(&self) -> &'static str {
		match self {
			Self::Edit	=> "an edit",
			Self::Surface	=> "a surface",
			Self::Icon	=> "an icon",
		}
	}

	/// The trees this kind is legal in, which never include a document.
	pub fn legal_in(&self) -> &'static str {
		match self {
			Self::Edit	=> "a chrome or an application tree",
			Self::Surface	=> "an application tree",
			Self::Icon	=> "a chrome or an application tree",
		}
	}

	/// The fields this kind's payload map must and may carry.
	///
	/// An `edit` carries the name the shell keeps its state under, and the hint it shows while it is
	/// empty. The name is required, since a field the shell cannot key state by can never hold any.
	///
	/// A `surface` carries the content hash of the application module it is a pane for, and the
	/// semantic alternative that stands in for that application. It carries **no geometry at all**: no
	/// width, no height, no minimum, no aspect. A surface is laid out like any box, by the host, in
	/// the flow of the tree, and an application learns the size it was given rather than stating the
	/// size it wants. An application that could size itself could size itself over the trust band, and
	/// the band's whole guarantee would be gone; a field the format does not have cannot be asked for.
	///
	/// The universal `style` field (§4.4) is legal on both, as it is on every map payload, and is not
	/// listed here for the same reason it is not listed on a document kind.
	pub fn fields(&self) -> &'static [Field] {
		match self {
			Self::Edit => &[
				Field { name: "name",	typ: FieldType::Str,	opt: false },
				Field { name: "placeholder",	typ: FieldType::Str,	opt: true },
			],
			Self::Surface => &[
				Field { name: "app",	typ: FieldType::Hash32,	opt: false },
				Field { name: KEY_ALT,	typ: FieldType::Nodes,	opt: false },
			],
			Self::Icon => &[
				Field { name: "name",	typ: FieldType::Str,	opt: false },
			],
		}
	}

	/// What this kind may contain, which is nothing: every reserved kind is a leaf.
	pub fn content(&self) -> Content {
		Content::None
	}

	/// The content class a parent must admit to hold this kind, which is where it may sit.
	///
	/// This is not [`Self::content`] read backwards. That says what a kind may hold; this says what
	/// may hold it, and the two are independent -- every reserved kind is a leaf, and they do not all
	/// sit in the same place.
	///
	/// An `edit` and a `surface` are blocks: a field is a line of its own and a pane is a rectangle,
	/// and each sits exactly where a `box` may. An `icon` is a glyph, and sits exactly where a `text`
	/// run may -- in the line, on the baseline, at the size and in the colour of whatever it stands
	/// beside. A bar's button is an icon and a word next to each other, and an icon that were flow
	/// content could only ever be a block above the word.
	pub fn sits_in(&self) -> Content {
		match self {
			Self::Edit	=> Content::Flow,
			Self::Surface	=> Content::Flow,
			Self::Icon	=> Content::Inline,
		}
	}

	/// Why this kind carries no children, as a refusal spells it.
	pub fn no_children(&self) -> &'static str {
		match self {
			Self::Edit => "an editable field is one line of text with a caret in it, and not a \
				container",
			Self::Surface => "a surface's content is the application's, and its 'alt' is what stands \
				in for the application when it is not running. A surface is a leaf that happens to \
				be alive",
			Self::Icon => "an icon is a glyph the engine draws, and a glyph holds nothing",
		}
	}
}

/// The icons the engine draws, which is the whole of what an `icon` node may name (§4.2).
///
/// The set is closed, and a name outside it is refused rather than drawn as a gap. An icon is not
/// content the tree supplies but a name the tree *reaches* the engine's own drawing by, so a name the
/// engine has never heard of is a caller's mistake, not a payload's, and there is nothing faithful to
/// render in its place. That is the same reason a reserved code is refused where an unknown code may
/// be admitted by a fallback (§4.5): a fallback exists for what the reader cannot know, and the reader
/// knows exactly which icons it has.
///
/// The names are browser actions, which is the second reason a document cannot name one: `back` means
/// nothing in a document, and a document that could say it would be reaching for a facility it does
/// not have.
///
/// Growing this set is a versioned decision, as growing the kinds is. A chrome naming an icon this
/// build does not draw is a bug in the chrome, and the validator is where it should be caught.
pub const ICON_NAMES: &[&str] = &[
	"back",		// Back to the previously held document.
	"forward",	// Forward again, after going back.
	"home",		// To the library, which is the home.
	"add",		// Open another tab.
	"close",	// Close this tab.
	"find",		// Search what is held: by name, by hash, by author.
	"reload",	// Reload the current document.
	"menu",		// Open the browser's menu.
	"more",		// Open the overflow of a control that holds more than it shows.
	"page",		// A document standing in for a tab whose favicon is not yet known.
	"panel",	// Toggle the side panel rail.
	"bookmark",	// Mark the held document, or open the marks.
	"download",	// Save the held document, or open what has been saved.
	"history",	// The documents held before this one, in time.
	"note",		// Annotate the held document.
	"tile",		// Split the view into tiled panes.
	"grip",		// The drag handle of a dockable bar: a two-by-three of dots.
	"minimise",	// Send the window to the taskbar.
	"maximise",	// Fill the screen with the window.
	"restore",	// Return a maximised window to its former size.
];

/// Whether the engine draws an icon of this name.
pub fn known_icon(name: &str) -> bool {
	ICON_NAMES.contains(&name)
}

/// The icon set an error message spells, so a refusal names what it would have taken.
pub fn icon_names_label() -> String {
	ICON_NAMES.join("', '")
}

/// The map key under which a node's children are carried.
pub const KEY_CHILDREN: &'static str = "children";
/// The map key under which a node names a document style (§4.4). Permitted on any map payload.
pub const KEY_STYLE: &'static str = "style";
/// The `doc` node's map key for the document style table (§4.4).
pub const KEY_STYLES: &'static str = "styles";
/// The map key under which an unknown kind carries its fallback (§4.5).
pub const KEY_FALLBACK: &'static str = "fallback";
/// The `surface` node's map key for its semantic alternative (§4.2).
///
/// The alternative is inert content, always: it is what a screen reader reads, what a search indexes,
/// and what the reader sees when the application is not running, was granted no drawing capability,
/// or was stopped for misbehaving. It is not the same key as an `image`'s `alt`, which is a string;
/// this one is a non-empty list of nodes.
pub const KEY_ALT: &'static str = "alt";
/// The `link` node's map key for its typed address (§4.3).
pub const KEY_TO: &'static str = "to";
/// The address-map key selecting a NAMES name.
pub const ADDR_NAME: &'static str = "name";
/// The address-map key selecting a content hash.
pub const ADDR_HASH: &'static str = "hash";

/// Returns the children of a node payload, or an empty slice if it declares none.
pub fn children_of(payload: &Dat) -> Outcome<Vec<Dat>> {
	match payload {
		Dat::Map(map) => {
			match map.get(&dat!(KEY_CHILDREN)) {
				None => Ok(Vec::new()),
				Some(Dat::List(v)) => Ok(v.clone()),
				Some(d) => Err(err!(
					"Node children must be a list, found {:?}.", d.kind();
				Invalid, Input)),
			}
		},
		_ => Ok(Vec::new()),
	}
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ STYLING (§4.4)                                                             │
// └───────────────────────────────────────────────────────────────────────────┘

/// The semantic palette names a colour property may carry. The reader's theme resolves them.
///
/// `line` is the rule between one region and the next: a colour quieter than the muted ink, for a surface
/// that draws its own edge. An outline in a text colour reads as a box drawn AROUND something rather than
/// as the edge OF it, so a border wants a name of its own.
pub const PALETTE: [&'static str; 5] = ["ink", "muted", "accent", "bg", "line"];
/// The text-direction values.
pub const DIRECTIONS: [&'static str; 2] = ["ltr", "rtl"];
/// The alignment values.
pub const ALIGNMENTS: [&'static str; 4] = ["start", "center", "end", "justify"];

/// How a style-record property's value is checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StyleCheck {
	/// A palette name (`str`), one of [`PALETTE`].
	Palette,
	/// A type scale step (`i8`); 0 is the reader's base size.
	ScaleStep,
	/// A language tag (`str`), BCP-47.
	Lang,
	/// A direction (`str`), one of [`DIRECTIONS`].
	Direction,
	/// A spacing scale index (`u8`).
	Spacing,
	/// An alignment (`str`), one of [`ALIGNMENTS`].
	Alignment,
	/// A grid's minimum tile width, as a PERCENTAGE of a base size (`u16`), so a measure that does not
	/// fall on a whole em can still be named.
	Tile,
	/// A share of the room a packed row has left over (`u8`); 0 takes none of it.
	Share,
	/// A border: a palette name and a width in pixels, as a two-element list. See [`check_border`].
	Border,
	/// How far a node is lifted off the surface behind it (`u8`); 0 is lying flat on it.
	Elevation,
}

/// Which schemas a style property belongs to (§4.4).
///
/// It is the property's own scope rather than a list kept on the schema, so a property is written
/// down once, beside the check its value is held to, and cannot be admitted somewhere its author
/// never wrote.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StyleScope {
	/// The v0 document vocabulary, which every schema admits and which is frozen.
	Doc,
	/// The interface vocabulary: admitted by a chrome and an application tree, and by no document.
	Interface,
}

impl StyleScope {

	/// The trees a property of this scope is legal in, as an error message names them.
	pub fn legal_in(&self) -> &'static str {
		match self {
			Self::Doc	=> "any tree",
			Self::Interface	=> "a chrome or an application tree",
		}
	}
}

/// One property of a style record.
#[derive(Clone, Copy, Debug)]
pub struct StyleField {
	/// The map key.
	pub name:	&'static str,
	/// How the value is checked.
	pub check:	StyleCheck,
	/// Whether the property flows down to descendants until overridden.
	pub inherited:	bool,
	/// The schemas that admit the property. See [`Schema::admits_style`].
	pub scope:	StyleScope,
}

/// The document style vocabulary (§4.4): the eight properties `oxeweb/doc/0` admits, and no others.
///
/// **This list is frozen.** A published document is signed under the vocabulary of its schema, and
/// its address is the hash of its bytes, so the set of properties it could ever have named is settled
/// the day it is published. Growing this list would change what an existing document is allowed to
/// mean; growth belongs in a schema version, or in [`INTERFACE_STYLE_FIELDS`], which no document may
/// name.
///
/// # It grew once, before v0 was published, and this is what was asked
///
/// `radius` was an interface property and is now a document one. The question put was whether a corner
/// radius is a property a DOCUMENT legitimately has, and the answer turns on three things.
///
/// It decorates a surface the document already draws. A style may name `bg` and `pad`, so a tinted,
/// padded box is already a thing a document makes; `radius` says what shape that box's corners are. It
/// adds no element, no geometry and no authority.
///
/// It has a meaning in prose. That is what tells it from the six it left behind -- `grid`, `pack`,
/// `grow`, `max`, `nowrap`, `ends` are the layout of a BAR and a SHELF and mean nothing in a document,
/// while a soft-cornered callout, pull-quote or listing is ordinary typography and has been for forty
/// years. A property whose absence every document would have to work around forever, for a rule that
/// was not protecting anything, is a property in the wrong list.
///
/// And the rule it seemed to break is not the rule that does the work. "A document must not dress as an
/// interface" is real, but what enforces it is ADDRESSING: the band and the bars are painted into
/// pixmaps a document is never handed, so it cannot draw there whatever it names. A document with `bg`,
/// `pad`, `fill` and `align` can already draw a filled box with a word in it -- a corner is a rounding,
/// not the difference between plausible and implausible -- and a document that drew a perfect forgery
/// of a control could still make nothing happen by it, because a document has no script and a link goes
/// to an address or nowhere. §4.4 says as much in its own words: a rounded corner is not a capability.
///
/// **`border` and `shadow` stayed where they are, and the distinction is the point.** A shadow is
/// ELEVATION -- the language of a thing that lifts toward the hand -- and a paragraph claiming to float
/// above its page is a document dressing as an interface in exactly the way the rule means. A border is
/// the line round a control with an edge, which is the spec's own phrase for what a chrome draws. It was
/// not asked about and it is not moved. One property moved because one property was argued.
pub const STYLE_FIELDS: [StyleField; 8] = [
	StyleField { name: "fill",	check: StyleCheck::Palette,	inherited: true,	scope: StyleScope::Doc },
	StyleField { name: "size",	check: StyleCheck::ScaleStep,	inherited: true,	scope: StyleScope::Doc },
	StyleField { name: "lang",	check: StyleCheck::Lang,	inherited: true,	scope: StyleScope::Doc },
	StyleField { name: "dir",	check: StyleCheck::Direction,	inherited: true,	scope: StyleScope::Doc },
	StyleField { name: "bg",	check: StyleCheck::Palette,	inherited: false,	scope: StyleScope::Doc },
	StyleField { name: "pad",	check: StyleCheck::Spacing,	inherited: false,	scope: StyleScope::Doc },
	StyleField { name: "align",	check: StyleCheck::Alignment,	inherited: false,	scope: StyleScope::Doc },
	StyleField { name: "radius",	check: StyleCheck::Spacing,	inherited: false,	scope: StyleScope::Doc },
];

/// The interface style vocabulary (§4.4): what a chrome or an application tree may name, and a
/// document may not.
///
/// The browser's own chrome is an SBJ tree laid out by the same engine as a document, and it is a
/// real interface: a library of tiles, a navigation bar, a control with an edge. These are what it
/// takes to draw one, and they are here rather than in [`STYLE_FIELDS`] because a document is not an
/// interface and must not be able to dress as one.
///
/// Every one of them is self-only. A property that cannot flow down cannot reach a node that did not
/// name it, so admitting three more of them widens what a chrome may say about itself and nothing
/// else.
pub const INTERFACE_STYLE_FIELDS: [StyleField; 8] = [
	StyleField { name: "grid",	check: StyleCheck::Tile,	inherited: false,	scope: StyleScope::Interface },
	StyleField { name: "pack",	check: StyleCheck::Tile,	inherited: false,	scope: StyleScope::Interface },
	StyleField { name: "grow",	check: StyleCheck::Share,	inherited: false,	scope: StyleScope::Interface },
	StyleField { name: "max",	check: StyleCheck::Tile,	inherited: false,	scope: StyleScope::Interface },
	StyleField { name: "border",	check: StyleCheck::Border,	inherited: false,	scope: StyleScope::Interface },
	StyleField { name: "shadow",	check: StyleCheck::Elevation,	inherited: false,	scope: StyleScope::Interface },
	StyleField { name: "nowrap",	check: StyleCheck::Share,	inherited: false,	scope: StyleScope::Interface },
	StyleField { name: "ends",	check: StyleCheck::Spacing,	inherited: false,	scope: StyleScope::Interface },
];

/// Returns the style-record property of the given name in ANY schema's vocabulary, if it is one.
///
/// This answers what a property IS -- its width, its check, its scope -- and never whether a tree may
/// name it. It exists for the canonical encoding check, which has no schema and needs none: a
/// property's wire type does not depend on who may name it, so `grid` is a `u8` in every tree, and
/// pinning that width is the same work in all three schemas.
///
/// **A validator must not call this.** Admission is [`Schema::style_field`], which asks this and then
/// applies the schema's rule; calling this instead would admit `grid` in a document.
pub fn known_style_field(name: &str) -> Option<&'static StyleField> {
	STYLE_FIELDS.iter()
		.chain(INTERFACE_STYLE_FIELDS.iter())
		.find(|f| f.name == name)
}

/// A decoded, validated border: the line's palette name, and how thick it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Border<'a> {
	/// The palette name the line is drawn in. The reader's theme resolves it, as it does any other.
	pub colour:	&'a str,
	/// The width of the line, in pixels.
	pub width:	u8,
}

/// Checks a style's `border` value: a two-element list of a palette name and a width in pixels (§4.4).
///
/// A border is one property and not two: a colour with no width draws nothing, and a width with no
/// colour draws nothing, so the two are named together or not at all, and a half-written border
/// cannot be spelt.
///
/// The width is in pixels rather than on the spacing scale that `pad` and `radius` ride, because a
/// border is a boundary and not a measure of room: a hairline is one pixel at any text size.
pub fn check_border(d: &Dat) -> Outcome<Border<'_>> {
	let list = match d {
		Dat::List(list) => list,
		other => return Err(err!(
			"A border is a palette name and a width in pixels, written as a two-element list, \
			e.g. [\"muted\", (u8|1)]; found a {:?}.", other.kind();
		Invalid, Input)),
	};
	let (colour, width) = match list.as_slice() {
		[Dat::Str(colour), Dat::U8(width)] => (colour.as_str(), *width),
		_ => return Err(err!(
			"A border is a palette name and a width in pixels, in that order, written as a \
			two-element list, e.g. [\"muted\", (u8|1)]; found a list of {} element(s).", list.len();
		Invalid, Input)),
	};
	if !PALETTE.contains(&colour) {
		return Err(err!(
			"A border is drawn in a palette colour ({}), but one names '{}'.",
			PALETTE.join(", "), colour;
		Invalid, Input));
	}
	Ok(Border {
		colour,
		width,
	})
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ LINK ADDRESSES (§4.3)                                                      │
// └───────────────────────────────────────────────────────────────────────────┘

/// A decoded, validated link address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Address {
	/// A NAMES name.
	Name(String),
	/// A 32-byte content hash.
	Hash(Vec<u8>),
}

/// Checks a `link`'s `to` value: a map with exactly one entry, `name` (str) or `hash` (b32).
///
/// Returning the typed address means the decoder tells a name from a hash, and a malformed target
/// is refused here rather than misread by the renderer.
pub fn check_address(d: &Dat) -> Outcome<Address> {
	let map = match d {
		Dat::Map(map) => map,
		other => return Err(err!(
			"A link address is a single-entry map, found a {:?}.", other.kind();
		Invalid, Input)),
	};
	if map.len() != 1 {
		return Err(err!(
			"A link address is a single-entry map naming a name or a hash, found {} entries.",
			map.len();
		Invalid, Input));
	}
	match map.get(&dat!(ADDR_NAME)) {
		Some(Dat::Str(s)) => return Ok(Address::Name(s.clone())),
		Some(other) => return Err(err!(
			"A link address \"name\" is a str, found a {:?}.", other.kind();
		Invalid, Input)),
		None => (),
	}
	match map.get(&dat!(ADDR_HASH)) {
		Some(Dat::B32(b)) => Ok(Address::Hash(b.to_vec())),
		Some(other) => Err(err!(
			"A link address \"hash\" is a b32, found a {:?}.", other.kind();
		Invalid, Input)),
		None => Err(err!(
			"A link address names \"name\" or \"hash\", found neither."; Invalid, Input)),
	}
}
