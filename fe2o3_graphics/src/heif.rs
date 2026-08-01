//! A reader of the HEIF container: the boxes that say which picture a file holds and where its
//! bytes are.
//!
//! This is the half of HEIC that is not a codec. A HEIF file is ISO base media file format boxes --
//! the same length-prefixed structure [`crate::mp4`] writes -- carrying a set of *items* rather
//! than a track: one of them is the picture, the rest are its thumbnail, its Exif block, its colour
//! profile and, on a modern phone, the several dozen tiles the picture is actually cut into. None
//! of that is compressed and none of it needs a decoder, so it is read here and read completely,
//! and what a decoder is then handed is a run of bytes and the configuration record that describes
//! them.
//!
//! # Why this exists before any HEVC decoder does
//!
//! Two things a photograph library needs are in the container and not in the coded picture.
//!
//! The **size** is one. A reader that takes the first `ispe` box it meets gets the thumbnail's
//! extent, because a phone writes the thumbnail's properties first: a four-thousand-pixel
//! photograph is then indexed as five hundred and twelve pixels square. The size is only right if
//! the primary item is resolved -- `pitm` names it, `ipma` says which properties are its -- and, for
//! a picture stored as a grid, only the `grid` item itself carries the assembled extent, since no
//! tile knows how many tiles are beside it.
//!
//! The **Exif block** is the other, and it hangs off the primary item by an `iref` of type `cdsc`
//! rather than sitting in a box of its own.
//!
//! # What is read
//!
//! `ftyp`, and inside `meta`: `hdlr`, `pitm`, `iinf` and its `infe` entries, `iref`, `iprp` with the
//! property container `ipco` and the association table `ipma`, `iloc`, and `idat`. Of the
//! properties, `ispe` (extent), `hvcC` (the HEVC decoder configuration), `irot` (rotation), `pixi`
//! (bits a channel) and `clap` (a cropping window) are kept; the rest, including the ICC profile in
//! `colr`, are recorded as present and left where they are.
//!
//! # What is refused
//!
//! A file whose boxes do not tile it exactly; a box that claims to end beyond its parent; a `meta`
//! with no `pitm`, or a `pitm` naming an item that no `iinf` describes; an item whose extents fall
//! outside the file or outside `idat`; a grid naming a number of tiles that is not its rows times
//! its columns; and more items than [`MAX_ITEMS`], which no photograph is.
//!
//! Nothing here decodes a pixel. A single-tile picture yields one run of bytes and its `hvcC`; a
//! grid yields the geometry and each tile's bytes in raster order; and a caller with no HEVC
//! decoder can still say how big the picture is, which way up it goes, and what its camera wrote.
//!
//! # References
//!
//! The box structure is ISO/IEC 14496-12 (§8.11 for the metadata boxes). The image-specific
//! items, properties and the `grid` derivation are ISO/IEC 23008-12 (§6 for the item structure,
//! §6.5 for the properties, §6.6.2.3 for the grid). The decoder configuration record `hvcC` carries
//! is ISO/IEC 14496-15 §8.3.3. Each non-obvious constant below names the clause it comes from.

use crate::{
	hevc,
	pixmap::Pixmap,
};

use oxedyne_fe2o3_core::prelude::*;

use std::collections::BTreeMap;

/// The most items one file may hold.
///
/// A photograph cut into tiles of five hundred and twelve pixels needs one item a tile, so a very
/// large picture can reach a few hundred; sixty-five thousand is a ceiling against a length that is
/// a mistake rather than a limit anything real approaches.
pub const MAX_ITEMS: usize = 65_536;

/// How deep the box tree may nest before it is treated as malformed.
///
/// The deepest legal path here is `meta` > `iprp` > `ipco` > a property, which is three.
pub const MAX_DEPTH: usize = 8;

/// A run of bytes belonging to one item.
///
/// An item is allowed to be scattered, and a grid's tiles routinely are, so an item's bytes are a
/// list of these rather than one offset and one length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Extent {
	/// Where the run starts, as an offset into whatever [`Where`] says it is in.
	pub off:	u64,
	/// How long the run is, in bytes.
	pub len:	u64,
}

/// What an item's extents are offsets into.
///
/// `iloc` calls this the construction method (ISO/IEC 14496-12 §8.11.3.3). Method 2 -- an item
/// stored inside another item -- is legal and is not written by any camera; it is read as far as
/// saying so, and refused rather than guessed at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Where {
	/// Offsets into the file, which is where a coded picture lives.
	File,
	/// Offsets into the `idat` box, which is where a small derivation like a `grid` lives.
	Idat,
	/// Offsets into another item, which this reader refuses.
	Item,
}

/// One item: what it is, and where its bytes are.
#[derive(Clone, Debug)]
pub struct Item {
	/// The identifier `iinf`, `iref`, `ipma` and `iloc` all name it by.
	pub id:		u32,
	/// Its four-character type: `hvc1` for a coded picture, `grid` for a derivation, `Exif` for the
	/// camera's block, `mime` for XMP.
	pub kind:	[u8; 4],
	/// Whether `pitm` named this one.
	pub primary:	bool,
	/// What its extents are offsets into.
	pub place:	Where,
	/// Where its bytes are, in order.
	pub extents:	Vec<Extent>,
}

/// A picture assembled out of tiles, ISO/IEC 23008-12 §6.6.2.3.
///
/// The assembled extent is **not** the tiles' extent summed: the grid is allowed to be larger than
/// the picture and the picture is then cropped out of its top left, which is how a photograph of
/// twelve hundred and eighty by nine hundred and sixty comes out of six tiles of five hundred and
/// twelve square.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grid {
	/// How many tiles down.
	pub rows:	u16,
	/// How many tiles across.
	pub cols:	u16,
	/// The assembled picture's width in pixels, after cropping.
	pub width:	u32,
	/// Its height in pixels, after cropping.
	pub height:	u32,
}

/// One property out of `ipco`, in the order the box holds them.
///
/// The ones a decoder or a library needs are read into their own shape; everything else is kept as
/// its type and its span, so that a caller wanting the ICC profile can find it without this module
/// having an opinion about colour management.
#[derive(Clone, Debug)]
pub enum Prop {
	/// `ispe`: the item's extent in pixels, before any rotation.
	Extent {
		/// Width in pixels.
		w: u32,
		/// Height in pixels.
		h: u32,
	},
	/// `hvcC`: the HEVC decoder configuration record, as a span of the file.
	Config(Extent),
	/// `irot`: a rotation the viewer applies, in quarter turns anticlockwise (0 to 3).
	Rotation(u8),
	/// `pixi`: how many bits each channel carries.
	Depth(Vec<u8>),
	/// Anything else, as its type and the span of its body.
	Other([u8; 4], Extent),
}

/// A HEIF file's metadata, read whole.
///
/// It borrows the bytes it was read from, so an item's data is handed back as a slice rather than
/// copied.
#[derive(Clone, Debug)]
pub struct Heif<'a> {
	/// The bytes the boxes were read out of.
	bytes:		&'a [u8],
	/// The brand from `ftyp`, which says whether the pictures inside are HEVC or AV1.
	brand:		[u8; 4],
	/// Every item, in the order `iinf` listed them.
	items:		Vec<Item>,
	/// The identifier `pitm` named.
	primary:	u32,
	/// The properties in `ipco`, indexed from one by `ipma`.
	props:		Vec<Prop>,
	/// Which properties belong to which item, by `ipco` index.
	owned:		BTreeMap<u32, Vec<u16>>,
	/// What each item derives from, in order, out of the `dimg` references.
	derived:	BTreeMap<u32, Vec<u32>>,
	/// What each item describes, out of the `cdsc` references.
	describes:	BTreeMap<u32, Vec<u32>>,
	/// The span of `idat`, where a derivation's bytes live.
	idat:		Option<Extent>,
}

/// What the primary item turned out to be.
#[derive(Clone, Debug)]
pub enum Picture {
	/// One coded picture: its bytes, and the decoder configuration that reads them.
	One {
		/// The item holding the coded picture.
		item:	u32,
		/// Its extent in pixels.
		size:	(u32, u32),
	},
	/// A picture cut into tiles, in raster order from the top left.
	Tiled {
		/// The assembled geometry.
		grid:	Grid,
		/// The items holding the tiles, row by row.
		tiles:	Vec<u32>,
	},
	/// A picture in a codec this container reader identifies but does not describe further, which
	/// is how a JPEG inside a HEIF wrapper arrives.
	Foreign {
		/// The item holding it.
		item:	u32,
		/// Its four-character type.
		kind:	[u8; 4],
	},
}

impl<'a> Heif<'a> {

	/// Reads a file's boxes.
	///
	/// The whole file is wanted rather than its head: `iloc` addresses `mdat` by absolute offset,
	/// so an item's bytes cannot be handed back from a prefix. A caller with only a head can still
	/// call this and will get the metadata, and [`Self::data`] will then refuse the item rather
	/// than return a short slice.
	pub fn read(bytes: &'a [u8]) -> Outcome<Self> {
		Self::parse(bytes, true)
	}

	/// Reads the boxes out of the front of a file.
	///
	/// A library that has read the first few tens of kilobytes to work out what a file is has the
	/// metadata already, and the metadata is all that is needed to say how big the picture is. The
	/// last box is allowed to run past the end of what was read -- it is `mdat`, and its bytes were
	/// never asked for -- and no item is checked against the file's length, since the file is
	/// longer than the buffer by construction. [`Self::data`] refuses afterwards rather than
	/// returning a short slice.
	pub fn head(bytes: &'a [u8]) -> Outcome<Self> {
		Self::parse(bytes, false)
	}

	fn parse(bytes: &'a [u8], whole: bool) -> Outcome<Self> {
		// Asked before the walk rather than after it. A walk that meets a JPEG's first bytes
		// reports a box four gigabytes long, which is true and useless.
		if bytes.len() < 8 || &bytes[4..8] != b"ftyp" {
			return Err(err!(
				"The file does not open with a file type box, so it is not a HEIF file at all. \
				The extension is not evidence: a fifth of the .heic files in one real library are \
				JPEG under another name.";
			Invalid, Input, Decode));
		}
		let mut heif = Self {
			bytes,
			brand:		[0; 4],
			items:		Vec::new(),
			primary:	0,
			props:		Vec::new(),
			owned:		BTreeMap::new(),
			derived:	BTreeMap::new(),
			describes:	BTreeMap::new(),
			idat:		None,
		};
		let mut found_meta = false;
		res!(walk_from(bytes, 0, bytes.len(), 0, whole, &mut |kind, body| {
			match &kind {
				b"ftyp" => {
					if body.len < 4 {
						return Err(err!(
							"The file type box is {} bytes, and a brand is four.", body.len;
						Invalid, Input, Decode));
					}
					let at = body.off as usize;
					heif.brand.copy_from_slice(&bytes[at..at + 4]);
					Ok(Walk::Over)
				},
				b"meta" => {
					found_meta = true;
					// A full box: one byte of version and three of flags before the children.
					Ok(Walk::Into(4))
				},
				_ => Ok(Walk::Over),
			}
		}));
		if !found_meta {
			return Err(err!(
				"The file carries no metadata box, so it holds no items."; Invalid, Input, Missing));
		}
		// The second pass reads the boxes whose meaning depends on the others being in hand: the
		// properties have to exist before `ipma` can point at them, and the items before `iloc`
		// can place them. Walking twice costs nothing measurable -- these boxes are a few tens of
		// kilobytes and hold no compression -- and it keeps each reader below free of ordering
		// rules the format does not actually guarantee.
		res!(heif.read_meta(whole));
		if whole {
			res!(heif.check());
		}
		Ok(heif)
	}

	/// The brand from `ftyp`, which is `heic` for an HEVC picture and `avif` for an AV1 one.
	pub fn brand(&self) -> [u8; 4] {
		self.brand
	}

	/// Every item, in the order the file listed them.
	pub fn items(&self) -> &[Item] {
		&self.items
	}

	/// The item `pitm` named.
	pub fn primary(&self) -> Outcome<&Item> {
		match self.items.iter().find(|i| i.id == self.primary) {
			Some(item) => Ok(item),
			None => Err(err!(
				"The primary item is number {}, which no item information entry describes.",
				self.primary;
			Invalid, Input, Missing)),
		}
	}

	/// What the primary item is: one coded picture, a grid of them, or something foreign.
	pub fn picture(&self) -> Outcome<Picture> {
		let item = res!(self.primary());
		if &item.kind == b"grid" {
			let grid = res!(self.grid(item.id));
			let tiles = match self.derived.get(&item.id) {
				Some(ids) => ids.clone(),
				None => Vec::new(),
			};
			let want = grid.rows as usize * grid.cols as usize;
			if tiles.len() != want {
				return Err(err!(
					"The grid is {} by {} and so wants {} tiles, and {} are named.",
					grid.cols, grid.rows, want, tiles.len();
				Invalid, Input, Decode));
			}
			return Ok(Picture::Tiled { grid, tiles });
		}
		if &item.kind == b"hvc1" || &item.kind == b"hev1" || &item.kind == b"av01" {
			let size = res!(self.extent_of(item.id));
			return Ok(Picture::One { item: item.id, size });
		}
		Ok(Picture::Foreign { item: item.id, kind: item.kind })
	}

	/// The picture's width and height in pixels, as it is meant to be looked at.
	///
	/// This is the number a library indexes and lays a tile out by, and it is **not** the first
	/// `ispe` in the file: a grid's extent comes from the grid, and a quarter turn in `irot`
	/// exchanges the two.
	pub fn size(&self) -> Outcome<(u32, u32)> {
		let (w, h) = res!(self.extent());
		Ok(if res!(self.rotation()) % 2 == 1 { (h, w) } else { (w, h) })
	}

	/// The picture's width and height in pixels **as they are coded**, before any rotation.
	///
	/// This is the pair a caller wants where it applies the Exif orientation itself, which is the
	/// usual arrangement in a library that also reads JPEG: a phone writes `irot` *and* an Exif
	/// orientation saying the same thing, so a reader that turns the picture by both turns it
	/// twice and lays a portrait photograph out landscape again.
	pub fn extent(&self) -> Outcome<(u32, u32)> {
		match res!(self.picture()) {
			Picture::One { size, .. } => Ok(size),
			Picture::Tiled { grid, .. } => Ok((grid.width, grid.height)),
			Picture::Foreign { item, .. } => self.extent_of(item),
		}
	}

	/// How far the picture is turned, in quarter turns anticlockwise.
	///
	/// Zero where the file says nothing, which is the common case: a phone that writes `irot` also
	/// writes the Exif orientation, and a reader that applies both turns the picture twice.
	pub fn rotation(&self) -> Outcome<u8> {
		let item = res!(self.primary());
		for prop in self.props_of(item.id) {
			if let Prop::Rotation(turns) = prop {
				return Ok(*turns);
			}
		}
		Ok(0)
	}

	/// The HEVC decoder configuration record for an item, as bytes.
	///
	/// Every tile of a grid shares one of these, and it is associated with the tiles rather than
	/// with the grid, so a caller asks for a tile's.
	pub fn config(&self, item: u32) -> Outcome<&'a [u8]> {
		for prop in self.props_of(item) {
			if let Prop::Config(span) = prop {
				return self.slice(*span);
			}
		}
		Err(err!(
			"Item {} carries no decoder configuration record.", item; Invalid, Input, Missing))
	}

	/// An item's bytes, gathered out of the file in extent order.
	///
	/// A single-extent item -- which nearly every one is -- borrows rather than copies.
	pub fn data(&self, item: u32) -> Outcome<std::borrow::Cow<'a, [u8]>> {
		let found = match self.items.iter().find(|i| i.id == item) {
			Some(i) => i,
			None => return Err(err!("There is no item {} in this file.", item; Invalid, Input)),
		};
		let base = match found.place {
			Where::File => Extent { off: 0, len: self.bytes.len() as u64 },
			Where::Idat => match self.idat {
				Some(span) => span,
				None => return Err(err!(
					"Item {} says its bytes are in the item data box, and there is none.", item;
				Invalid, Input, Missing)),
			},
			Where::Item => return Err(err!(
				"Item {} is stored inside another item, which this reader does not follow.", item;
			Invalid, Input, Unknown)),
		};
		if found.extents.len() == 1 {
			let one = found.extents[0];
			return Ok(std::borrow::Cow::Borrowed(res!(self.slice(Extent {
				off: base.off + one.off, len: one.len }))));
		}
		let mut out = Vec::new();
		for span in &found.extents {
			out.extend_from_slice(res!(self.slice(Extent {
				off: base.off + span.off, len: span.len })));
		}
		Ok(std::borrow::Cow::Owned(out))
	}

	/// The Exif block the camera wrote, without the four-byte offset header that precedes it.
	///
	/// It hangs off the primary item by a `cdsc` reference, and its payload begins with a
	/// four-byte offset to the TIFF header (ISO/IEC 23008-12 §A.2.1) which is skipped here so that
	/// what comes back is what an Exif reader expects: `MM` or `II` and then the first directory.
	pub fn exif(&self) -> Outcome<Option<&'a [u8]>> {
		let primary = self.primary;
		for item in &self.items {
			if &item.kind != b"Exif" {
				continue;
			}
			let describes_primary = match self.describes.get(&item.id) {
				Some(ids) => ids.contains(&primary),
				// A file with one Exif item and no reference is common enough to accept: there is
				// nothing else it could describe.
				None => true,
			};
			if !describes_primary {
				continue;
			}
			let span = match item.extents.first() {
				Some(e) => *e,
				None => continue,
			};
			let base = match item.place {
				Where::File => 0,
				Where::Idat => match self.idat {
					Some(idat) => idat.off,
					None => continue,
				},
				Where::Item => continue,
			};
			let whole = res!(self.slice(Extent { off: base + span.off, len: span.len }));
			if whole.len() < 4 {
				return Err(err!(
					"The Exif item is {} bytes, which is shorter than its own header.", whole.len();
				Invalid, Input, Decode));
			}
			let skip = 4 + u32::from_be_bytes([whole[0], whole[1], whole[2], whole[3]]) as usize;
			if skip > whole.len() {
				return Err(err!(
					"The Exif item's header points {} bytes into a block of {}.", skip, whole.len();
				Invalid, Input, Decode));
			}
			return Ok(Some(&whole[skip..]));
		}
		Ok(None)
	}

	// ------------------------------------------------------------------ the reading itself

	/// Reads the boxes under `meta` that need the whole file in hand.
	fn read_meta(&mut self, whole: bool) -> Outcome<()> {
		let bytes = self.bytes;
		let mut items:		Vec<Item> = Vec::new();
		let mut primary:	Option<u32> = None;
		let mut props:		Vec<Prop> = Vec::new();
		let mut owned:		BTreeMap<u32, Vec<u16>> = BTreeMap::new();
		let mut derived:	BTreeMap<u32, Vec<u32>> = BTreeMap::new();
		let mut describes:	BTreeMap<u32, Vec<u32>> = BTreeMap::new();
		let mut places:		BTreeMap<u32, (Where, Vec<Extent>)> = BTreeMap::new();
		let mut idat:		Option<Extent> = None;
		res!(walk_from(bytes, 0, bytes.len(), 0, whole, &mut |kind, body| {
			match &kind {
				b"meta" => Ok(Walk::Into(4)),
				b"iprp" => Ok(Walk::Into(0)),
				// Read whole rather than descended into: every box inside `ipco` is a property
				// and its index is its position, so a walker that let them fall through with the
				// rest of the file would number them by what else it had met on the way.
				b"ipco" => {
					props = res!(read_ipco(bytes, body));
					Ok(Walk::Over)
				},
				b"pitm" => {
					primary = Some(res!(read_pitm(bytes, body)));
					Ok(Walk::Over)
				},
				b"iinf" => {
					items = res!(read_iinf(bytes, body));
					Ok(Walk::Over)
				},
				b"iref" => {
					res!(read_iref(bytes, body, &mut derived, &mut describes));
					Ok(Walk::Over)
				},
				b"ipma" => {
					res!(read_ipma(bytes, body, &mut owned));
					Ok(Walk::Over)
				},
				b"iloc" => {
					places = res!(read_iloc(bytes, body));
					Ok(Walk::Over)
				},
				b"idat" => {
					idat = Some(body);
					Ok(Walk::Over)
				},
				_ => Ok(Walk::Over),
			}
		}));
		let primary = match primary {
			Some(id) => id,
			None => return Err(err!(
				"The metadata box names no primary item, so there is no picture to show.";
			Invalid, Input, Missing)),
		};
		for item in &mut items {
			item.primary = item.id == primary;
			if let Some((place, extents)) = places.remove(&item.id) {
				item.place = place;
				item.extents = extents;
			}
		}
		self.items = items;
		self.primary = primary;
		self.props = props;
		self.owned = owned;
		self.derived = derived;
		self.describes = describes;
		self.idat = idat;
		Ok(())
	}

	/// Refuses a file whose parts do not agree with each other.
	///
	/// Each of these has been met in the wild, and each yields a picture that looks like a decoder
	/// fault when it is really a reader that trusted the file.
	fn check(&self) -> Outcome<()> {
		if self.items.len() > MAX_ITEMS {
			return Err(err!(
				"The file describes {} items, and {} is the most this reader will read.",
				self.items.len(), MAX_ITEMS;
			Invalid, Input, TooBig));
		}
		let _ = res!(self.primary());
		for item in &self.items {
			let limit = match item.place {
				Where::File => self.bytes.len() as u64,
				Where::Idat => match self.idat {
					Some(span) => span.len,
					None if item.extents.is_empty() => continue,
					None => return Err(err!(
						"Item {} is placed in the item data box, and the file has none.", item.id;
					Invalid, Input, Missing)),
				},
				Where::Item => continue,
			};
			for span in &item.extents {
				let end = span.off.saturating_add(span.len);
				if end > limit {
					return Err(err!(
						"Item {} claims bytes {} to {} of a {} the file ends at {}.",
						item.id, span.off, end,
						match item.place { Where::Idat => "region", _ => "file" }, limit;
					Invalid, Input, Decode));
				}
			}
		}
		Ok(())
	}

	/// The properties associated with one item, in association order.
	fn props_of(&self, item: u32) -> Vec<&Prop> {
		let mut out = Vec::new();
		if let Some(indices) = self.owned.get(&item) {
			for i in indices {
				// `ipma` indexes from one, and zero means "no property".
				if *i > 0 {
					if let Some(prop) = self.props.get(*i as usize - 1) {
						out.push(prop);
					}
				}
			}
		}
		out
	}

	/// One item's extent in pixels, out of its `ispe` property.
	fn extent_of(&self, item: u32) -> Outcome<(u32, u32)> {
		for prop in self.props_of(item) {
			if let Prop::Extent { w, h } = prop {
				return Ok((*w, *h));
			}
		}
		Err(err!(
			"Item {} carries no extent, so there is no telling how big it is.", item;
		Invalid, Input, Missing))
	}

	/// The geometry of a `grid` item, out of the sixteen bytes it holds.
	///
	/// ISO/IEC 23008-12 §6.6.2.3.2: a version, flags whose low bit chooses between sixteen- and
	/// thirty-two-bit extents, the counts less one, and then the assembled width and height.
	fn grid(&self, item: u32) -> Outcome<Grid> {
		let data = res!(self.data(item));
		if data.len() < 8 {
			return Err(err!(
				"A grid is described in {} bytes, and the shortest legal one is eight.", data.len();
			Invalid, Input, Decode));
		}
		let wide = data[1] & 1 == 1;
		// Rows first, then columns (ISO/IEC 23008-12 §6.6.2.3.2). The two were the other way round
		// here until something assembled a grid rather than merely counting its tiles: a
		// photograph 3,088 wide out of 512-sample tiles needs seven across and five down, and
		// reading them swapped gave five across and seven down -- which counts to the same
		// thirty-five tiles, so the check that a grid names as many tiles as it has rows times
		// columns passed all along.
		let rows = data[2] as u16 + 1;
		let cols = data[3] as u16 + 1;
		let (width, height) = if wide {
			if data.len() < 12 {
				return Err(err!(
					"A grid says its extent is thirty-two bits wide and gives {} bytes for it.",
					data.len();
				Invalid, Input, Decode));
			}
			(
				u32::from_be_bytes([data[4], data[5], data[6], data[7]]),
				u32::from_be_bytes([data[8], data[9], data[10], data[11]]),
			)
		} else {
			(
				u16::from_be_bytes([data[4], data[5]]) as u32,
				u16::from_be_bytes([data[6], data[7]]) as u32,
			)
		};
		if width == 0 || height == 0 {
			return Err(err!(
				"A grid assembles to {} by {} pixels.", width, height; Invalid, Input, Decode));
		}
		Ok(Grid { rows, cols, width, height })
	}

	/// A span of the file, refused rather than truncated where it runs past the end.
	fn slice(&self, span: Extent) -> Outcome<&'a [u8]> {
		let off = span.off as usize;
		let end = match off.checked_add(span.len as usize) {
			Some(end) => end,
			None => return Err(err!(
				"A span at {} of length {} overflows.", span.off, span.len; Invalid, Input, Decode)),
		};
		if end > self.bytes.len() {
			return Err(err!(
				"A span ends at byte {} of a file of {}. A file read in part cannot give up its \
				items, only its metadata.", end, self.bytes.len();
			Invalid, Input, Decode));
		}
		Ok(&self.bytes[off..end])
	}
}

// ---------------------------------------------------------------------------- the box walk

/// What a walker wants done with the box it was just handed.
enum Walk {
	/// Step over it, whatever it holds.
	Over,
	/// Walk its children, after the given number of bytes of its own header.
	Into(usize),
}

/// Walks a run of boxes, handing each one's type and the span of its body to a visitor.
///
/// The visitor decides what is descended into, which is what keeps the caller's reading of one box
/// beside its own knowledge of what that box contains. Every box is length-prefixed and the lengths
/// have to tile the parent exactly; a box that claims to end beyond its parent is a malformed file
/// and not a box to be clamped, since clamping turns one wrong length into a plausible-looking
/// picture.
fn walk<F>(bytes: &[u8], from: usize, to: usize, depth: usize, visit: &mut F) -> Outcome<()>
where
	F: FnMut([u8; 4], Extent) -> Outcome<Walk>,
{
	walk_from(bytes, from, to, depth, true, visit)
}

/// The same walk, with the choice of whether the run has to be tiled exactly.
///
/// `whole` is false when the caller holds the front of a file rather than the file: the last box
/// then legitimately runs past the end of what was read, and the walk stops there instead of
/// calling the file malformed. Everything inside a box that *is* complete is still checked, so a
/// truncated `meta` is a fault either way.
fn walk_from<F>(bytes: &[u8], from: usize, to: usize, depth: usize, whole: bool, visit: &mut F)
	-> Outcome<()>
where
	F: FnMut([u8; 4], Extent) -> Outcome<Walk>,
{
	if depth > MAX_DEPTH {
		return Err(err!(
			"The box tree nests more than {} deep, which no legal file does.", MAX_DEPTH;
		Invalid, Input, Decode));
	}
	if to > bytes.len() {
		return Err(err!(
			"A box tree was asked for out to byte {} of a file of {}.", to, bytes.len();
		Invalid, Input, Decode));
	}
	let mut at = from;
	while at + 8 <= to {
		let size = u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
		let mut kind = [0u8; 4];
		kind.copy_from_slice(&bytes[at + 4..at + 8]);
		let (size, head) = match size {
			// A size of one means the real one is the next eight bytes (ISO/IEC 14496-12 §4.2).
			1 => {
				if at + 16 > to {
					return Err(err!(
						"A box says its length is sixty-four bits and the file ends inside it.";
					Invalid, Input, Decode));
				}
				let mut wide = [0u8; 8];
				wide.copy_from_slice(&bytes[at + 8..at + 16]);
				(u64::from_be_bytes(wide), 16usize)
			},
			// A size of zero means the box runs to the end of its parent.
			0 => ((to - at) as u64, 8usize),
			n => (n as u64, 8usize),
		};
		let size = size as usize;
		if !whole && depth == 0 && size >= head && at.saturating_add(size) > to {
			// The front of a file, ending inside a box nobody asked for.
			return Ok(());
		}
		if size < head || at.saturating_add(size) > to {
			return Err(err!(
				"A {} box at byte {} says it is {} bytes long, and its parent ends at {}.",
				String::from_utf8_lossy(&kind), at, size, to;
			Invalid, Input, Decode));
		}
		let body = Extent { off: (at + head) as u64, len: (size - head) as u64 };
		match res!(visit(kind, body)) {
			Walk::Over => {},
			Walk::Into(skip) => {
				let start = at + head + skip;
				if start > at + size {
					return Err(err!(
						"A {} box is {} bytes long and its own header is {}.",
						String::from_utf8_lossy(&kind), size, head + skip;
					Invalid, Input, Decode));
				}
				res!(walk(bytes, start, at + size, depth + 1, visit));
			},
		}
		at += size;
	}
	if at != to && whole {
		return Err(err!(
			"The boxes between bytes {} and {} leave {} over, so they do not tile it.",
			from, to, to - at;
		Invalid, Input, Decode));
	}
	Ok(())
}

/// A reader of a box body, which refuses to run off its end rather than returning a short answer.
struct Body<'a> {
	/// The bytes.
	buf:	&'a [u8],
	/// Where the file the span came from starts, so that a span read out of this can be reported
	/// in the file's own coordinates.
	base:	u64,
	/// How far along.
	at:	usize,
}

impl<'a> Body<'a> {

	/// A reader over one box's body.
	fn new(bytes: &'a [u8], span: Extent) -> Outcome<Self> {
		let off = span.off as usize;
		let end = match off.checked_add(span.len as usize) {
			Some(end) if end <= bytes.len() => end,
			_ => return Err(err!(
				"A box body at {} of length {} runs past the end of a file of {}.",
				span.off, span.len, bytes.len();
			Invalid, Input, Decode)),
		};
		Ok(Self { buf: &bytes[off..end], base: span.off, at: 0 })
	}

	/// How many bytes are left.
	fn left(&self) -> usize {
		self.buf.len().saturating_sub(self.at)
	}

	/// The next `n` bytes as an unsigned integer, most significant first, for `n` up to eight.
	fn num(&mut self, n: usize) -> Outcome<u64> {
		if n > 8 {
			return Err(err!("A field of {} bytes was asked for, and eight is the widest.", n; Bug));
		}
		if self.left() < n {
			return Err(err!(
				"A box body of {} bytes ends before a field of {} at offset {}.",
				self.buf.len(), n, self.at;
			Invalid, Input, Decode));
		}
		let mut v = 0u64;
		for _ in 0..n {
			v = (v << 8) | self.buf[self.at] as u64;
			self.at += 1;
		}
		Ok(v)
	}

	/// The version and flags of a full box.
	fn full(&mut self) -> Outcome<(u8, u32)> {
		let v = res!(self.num(4));
		Ok(((v >> 24) as u8, (v & 0x00ff_ffff) as u32))
	}

	/// The next four bytes as a box type.
	fn kind(&mut self) -> Outcome<[u8; 4]> {
		let v = res!(self.num(4));
		Ok((v as u32).to_be_bytes())
	}

	/// Steps over a null-terminated string, which `infe` uses for a name.
	fn skip_string(&mut self) -> Outcome<()> {
		while self.at < self.buf.len() {
			let b = self.buf[self.at];
			self.at += 1;
			if b == 0 {
				return Ok(());
			}
		}
		Ok(())
	}

	/// The span, in the file's coordinates, of the rest of the body.
	fn rest(&self) -> Extent {
		Extent { off: self.base + self.at as u64, len: self.left() as u64 }
	}
}

/// `pitm`: which item is the picture (ISO/IEC 14496-12 §8.11.4).
fn read_pitm(bytes: &[u8], span: Extent) -> Outcome<u32> {
	let mut b = res!(Body::new(bytes, span));
	let (version, _) = res!(b.full());
	// Version 0 names the item in sixteen bits and version 1 in thirty-two.
	let width = if version == 0 { 2 } else { 4 };
	Ok(res!(b.num(width)) as u32)
}

/// `iinf` and its `infe` children: what each item is (ISO/IEC 14496-12 §8.11.6).
fn read_iinf(bytes: &[u8], span: Extent) -> Outcome<Vec<Item>> {
	let mut b = res!(Body::new(bytes, span));
	let (version, _) = res!(b.full());
	let count = res!(b.num(if version == 0 { 2 } else { 4 })) as usize;
	if count > MAX_ITEMS {
		return Err(err!(
			"The item information box lists {} entries, and {} is the most this reader will read.",
			count, MAX_ITEMS;
		Invalid, Input, TooBig));
	}
	let mut out = Vec::with_capacity(count.min(1024));
	let entries = Extent { off: span.off + b.at as u64, len: b.left() as u64 };
	res!(walk(bytes, entries.off as usize, (entries.off + entries.len) as usize, 0,
		&mut |kind, body| {
		if &kind != b"infe" {
			return Ok(Walk::Over);
		}
		let mut e = res!(Body::new(bytes, body));
		let (version, _) = res!(e.full());
		if version < 2 {
			// Versions 0 and 1 describe a track's items, not a picture's, and carry no type.
			return Err(err!(
				"An item information entry is version {}, and a picture's items are version two or \
				later.", version;
			Invalid, Input, Unknown));
		}
		let id = res!(e.num(if version == 2 { 2 } else { 4 })) as u32;
		let _protection = res!(e.num(2));
		let kind = res!(e.kind());
		res!(e.skip_string());
		out.push(Item { id, kind, primary: false, place: Where::File, extents: Vec::new() });
		Ok(Walk::Over)
	}));
	Ok(out)
}

/// `iref`: what refers to what (ISO/IEC 14496-12 §8.11.12).
///
/// Two reference types matter here. `dimg` runs from a derivation to the items it is derived from,
/// in order, which for a grid is its tiles in raster order. `cdsc` runs from a description -- an
/// Exif block, an XMP packet -- to the item it describes.
fn read_iref(
	bytes:		&[u8],
	span:		Extent,
	derived:	&mut BTreeMap<u32, Vec<u32>>,
	describes:	&mut BTreeMap<u32, Vec<u32>>,
)
	-> Outcome<()>
{
	let mut b = res!(Body::new(bytes, span));
	let (version, _) = res!(b.full());
	let width = if version == 0 { 2 } else { 4 };
	let entries = Extent { off: span.off + b.at as u64, len: b.left() as u64 };
	res!(walk(bytes, entries.off as usize, (entries.off + entries.len) as usize, 0,
		&mut |kind, body| {
		let mut e = res!(Body::new(bytes, body));
		let from = res!(e.num(width)) as u32;
		let count = res!(e.num(2)) as usize;
		let mut to = Vec::with_capacity(count.min(1024));
		for _ in 0..count {
			to.push(res!(e.num(width)) as u32);
		}
		match &kind {
			b"dimg" => { derived.insert(from, to); },
			b"cdsc" => { describes.insert(from, to); },
			_ => {},
		}
		Ok(Walk::Over)
	}));
	Ok(())
}

/// `ipma`: which properties belong to which item (ISO/IEC 23008-12 §6.5.2).
fn read_ipma(bytes: &[u8], span: Extent, owned: &mut BTreeMap<u32, Vec<u16>>) -> Outcome<()> {
	let mut b = res!(Body::new(bytes, span));
	let (version, flags) = res!(b.full());
	// The low bit of the flags says the index is fifteen bits rather than seven; the rest of the
	// byte is the "essential" flag, which a reader that keeps every property does not need.
	let wide = flags & 1 == 1;
	let count = res!(b.num(4)) as usize;
	if count > MAX_ITEMS {
		return Err(err!(
			"The property association box covers {} items, and {} is the most this reader will \
			read.", count, MAX_ITEMS;
		Invalid, Input, TooBig));
	}
	for _ in 0..count {
		let id = res!(b.num(if version < 1 { 2 } else { 4 })) as u32;
		let n = res!(b.num(1)) as usize;
		let mut indices = Vec::with_capacity(n.min(64));
		for _ in 0..n {
			let raw = res!(b.num(if wide { 2 } else { 1 }));
			let index = if wide { (raw & 0x7fff) as u16 } else { (raw & 0x7f) as u16 };
			indices.push(index);
		}
		owned.insert(id, indices);
	}
	Ok(())
}

/// `iloc`: where each item's bytes are (ISO/IEC 14496-12 §8.11.3).
fn read_iloc(bytes: &[u8], span: Extent) -> Outcome<BTreeMap<u32, (Where, Vec<Extent>)>> {
	let mut b = res!(Body::new(bytes, span));
	let (version, _) = res!(b.full());
	let sizes = res!(b.num(1));
	let widths = res!(b.num(1));
	let offset_size = (sizes >> 4) as usize;
	let length_size = (sizes & 0xf) as usize;
	let base_size = (widths >> 4) as usize;
	let index_size = if version == 1 || version == 2 { (widths & 0xf) as usize } else { 0 };
	let count = res!(b.num(if version < 2 { 2 } else { 4 })) as usize;
	if count > MAX_ITEMS {
		return Err(err!(
			"The item location box places {} items, and {} is the most this reader will read.",
			count, MAX_ITEMS;
		Invalid, Input, TooBig));
	}
	let mut out = BTreeMap::new();
	for _ in 0..count {
		let id = res!(b.num(if version < 2 { 2 } else { 4 })) as u32;
		let place = if version == 1 || version == 2 {
			let method = res!(b.num(2)) & 0xf;
			match method {
				0 => Where::File,
				1 => Where::Idat,
				_ => Where::Item,
			}
		} else {
			Where::File
		};
		let _data_reference = res!(b.num(2));
		let base = res!(b.num(base_size));
		let extents = res!(b.num(2)) as usize;
		let mut spans = Vec::with_capacity(extents.min(1024));
		for _ in 0..extents {
			if index_size > 0 {
				let _index = res!(b.num(index_size));
			}
			let off = res!(b.num(offset_size));
			let len = res!(b.num(length_size));
			spans.push(Extent { off: base.saturating_add(off), len });
		}
		out.insert(id, (place, spans));
	}
	Ok(out)
}

/// `ipco`: the properties, in the order `ipma` indexes them from one.
fn read_ipco(bytes: &[u8], span: Extent) -> Outcome<Vec<Prop>> {
	let mut out = Vec::new();
	res!(walk(bytes, span.off as usize, (span.off + span.len) as usize, 0, &mut |kind, body| {
		if out.len() >= u16::MAX as usize {
			return Err(err!(
				"The property container holds more than {} properties.", u16::MAX;
			Invalid, Input, TooBig));
		}
		out.push(res!(read_prop(bytes, kind, body)));
		Ok(Walk::Over)
	}));
	Ok(out)
}

/// One box out of `ipco`, read into the shape its type calls for.
fn read_prop(bytes: &[u8], kind: [u8; 4], span: Extent) -> Outcome<Prop> {
	match &kind {
		b"ispe" => {
			// A full box, then the width and height (ISO/IEC 23008-12 §6.5.3).
			let mut b = res!(Body::new(bytes, span));
			let _ = res!(b.full());
			let w = res!(b.num(4)) as u32;
			let h = res!(b.num(4)) as u32;
			Ok(Prop::Extent { w, h })
		},
		b"hvcC" => Ok(Prop::Config(span)),
		b"irot" => {
			// One byte, of which the low two bits are the quarter turns (§6.5.10).
			let mut b = res!(Body::new(bytes, span));
			Ok(Prop::Rotation((res!(b.num(1)) & 3) as u8))
		},
		b"pixi" => {
			// A full box, a count, then one byte a channel (§6.5.6).
			let mut b = res!(Body::new(bytes, span));
			let _ = res!(b.full());
			let channels = res!(b.num(1)) as usize;
			let mut depths = Vec::with_capacity(channels.min(8));
			for _ in 0..channels {
				depths.push(res!(b.num(1)) as u8);
			}
			Ok(Prop::Depth(depths))
		},
		other => {
			let mut kept = [0u8; 4];
			kept.copy_from_slice(other);
			Ok(Prop::Other(kept, span))
		},
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Builds a box: a big-endian length, a four-character type, and a body.
	fn bx(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
		let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
		out.extend_from_slice(kind);
		out.extend_from_slice(body);
		out
	}

	/// A file holding one coded picture of a known extent, and a thumbnail written first.
	///
	/// The thumbnail comes first on purpose: it is the shape a phone writes and the shape that
	/// makes a reader taking the first `ispe` it meets report the wrong size.
	fn one_picture() -> Vec<u8> {
		let mut ipco = Vec::new();
		// Property 1: the thumbnail's extent. Property 2: the picture's.
		let mut ispe_small = vec![0u8; 4];
		ispe_small.extend_from_slice(&512u32.to_be_bytes());
		ispe_small.extend_from_slice(&512u32.to_be_bytes());
		ipco.extend_from_slice(&bx(b"ispe", &ispe_small));
		let mut ispe_big = vec![0u8; 4];
		ispe_big.extend_from_slice(&4032u32.to_be_bytes());
		ispe_big.extend_from_slice(&3024u32.to_be_bytes());
		ipco.extend_from_slice(&bx(b"ispe", &ispe_big));
		ipco.extend_from_slice(&bx(b"hvcC", &[1, 2, 3, 4]));
		let iprp = bx(b"iprp", &bx(b"ipco", &ipco));

		// Item 1 is the thumbnail and item 2 the picture.
		let mut ipma = vec![0u8; 4];
		ipma.extend_from_slice(&2u32.to_be_bytes());
		ipma.extend_from_slice(&1u16.to_be_bytes());
		ipma.push(1);
		ipma.push(1);
		ipma.extend_from_slice(&2u16.to_be_bytes());
		ipma.push(2);
		ipma.push(2);
		ipma.push(3);
		let ipma = bx(b"ipma", &ipma);

		let mut infe1 = vec![2u8, 0, 0, 0];
		infe1.extend_from_slice(&1u16.to_be_bytes());
		infe1.extend_from_slice(&0u16.to_be_bytes());
		infe1.extend_from_slice(b"hvc1");
		infe1.push(0);
		let mut infe2 = vec![2u8, 0, 0, 0];
		infe2.extend_from_slice(&2u16.to_be_bytes());
		infe2.extend_from_slice(&0u16.to_be_bytes());
		infe2.extend_from_slice(b"hvc1");
		infe2.push(0);
		let mut iinf = vec![0u8; 4];
		iinf.extend_from_slice(&2u16.to_be_bytes());
		iinf.extend_from_slice(&bx(b"infe", &infe1));
		iinf.extend_from_slice(&bx(b"infe", &infe2));
		let iinf = bx(b"iinf", &iinf);

		let mut pitm = vec![0u8; 4];
		pitm.extend_from_slice(&2u16.to_be_bytes());
		let pitm = bx(b"pitm", &pitm);

		// Both items are placed at the front of the file, which is legal and enough for a reader
		// that is being checked on its metadata rather than on its pixels.
		let mut iloc = vec![0u8; 4];
		iloc.push(0x44);
		iloc.push(0x00);
		iloc.extend_from_slice(&2u16.to_be_bytes());
		for id in [1u16, 2u16] {
			iloc.extend_from_slice(&id.to_be_bytes());
			iloc.extend_from_slice(&0u16.to_be_bytes());
			iloc.extend_from_slice(&1u16.to_be_bytes());
			iloc.extend_from_slice(&0u32.to_be_bytes());
			iloc.extend_from_slice(&8u32.to_be_bytes());
		}
		let iloc = bx(b"iloc", &iloc);

		let mut meta = vec![0u8; 4];
		meta.extend_from_slice(&bx(b"hdlr", &[0u8; 20]));
		meta.extend_from_slice(&pitm);
		meta.extend_from_slice(&iinf);
		meta.extend_from_slice(&iprp);
		meta.extend_from_slice(&ipma);
		meta.extend_from_slice(&iloc);

		let mut out = bx(b"ftyp", b"heic\0\0\0\0mif1heic");
		out.extend_from_slice(&bx(b"meta", &meta));
		out.extend_from_slice(&bx(b"mdat", &[0u8; 64]));
		out
	}

	#[test]
	fn test_the_size_is_the_primary_item_s_and_not_the_first_ispe_00() -> Outcome<()> {
		let file = one_picture();
		let heif = res!(Heif::read(&file));
		req!(heif.brand(), *b"heic");
		req!(res!(heif.size()), (4032, 3024),
			"The thumbnail's extent was taken for the picture's.");
		req!(res!(heif.primary()).id, 2);
		Ok(())
	}

	#[test]
	fn test_the_configuration_record_comes_back_whole_01() -> Outcome<()> {
		let file = one_picture();
		let heif = res!(Heif::read(&file));
		req!(res!(heif.config(2)), &[1u8, 2, 3, 4][..]);
		Ok(())
	}

	#[test]
	fn test_a_box_that_ends_past_its_parent_is_refused_02() -> Outcome<()> {
		let mut file = one_picture();
		// The `meta` box's length, made longer than the file allows.
		let at = match (0..file.len() - 8).find(|i| &file[i + 4..i + 8] == b"meta") {
			Some(at) => at,
			None => return Err(err!("The fixture holds no metadata box."; Test, Missing)),
		};
		let was = u32::from_be_bytes([file[at], file[at + 1], file[at + 2], file[at + 3]]);
		file[at..at + 4].copy_from_slice(&(was + 4096).to_be_bytes());
		req!(Heif::read(&file).is_err(), true, "A box overrunning the file was read as if it fit.");
		Ok(())
	}

	#[test]
	fn test_a_grid_gives_the_assembled_extent_and_its_tiles_03() -> Outcome<()> {
		// Six tiles of 512 square assembling to 1280 by 960, which is what a phone writes and is
		// the case a reader that adds up its tiles gets wrong in both directions.
		//
		// **Rows before columns** (ISO/IEC 23008-12 §6.6.2.3.2). This fixture used to be written
		// the other way about, to match a reader that had them swapped, and the two wrongs made a
		// passing test. What settled it is a real photograph: 3,088 samples wide out of 512-sample
		// tiles needs seven across, and only one reading of the box gives seven.
		let mut grid = vec![0u8, 0, 1, 2];
		grid.extend_from_slice(&1280u16.to_be_bytes());
		grid.extend_from_slice(&960u16.to_be_bytes());
		let geometry = Grid { rows: 2, cols: 3, width: 1280, height: 960 };
		req!(grid.len(), 8);

		let mut ipco = Vec::new();
		let mut ispe = vec![0u8; 4];
		ispe.extend_from_slice(&512u32.to_be_bytes());
		ispe.extend_from_slice(&512u32.to_be_bytes());
		ipco.extend_from_slice(&bx(b"ispe", &ispe));
		let iprp = bx(b"iprp", &bx(b"ipco", &ipco));

		let mut iinf = vec![0u8; 4];
		iinf.extend_from_slice(&7u16.to_be_bytes());
		for id in 1u16..=6 {
			let mut infe = vec![2u8, 0, 0, 0];
			infe.extend_from_slice(&id.to_be_bytes());
			infe.extend_from_slice(&0u16.to_be_bytes());
			infe.extend_from_slice(b"hvc1");
			infe.push(0);
			iinf.extend_from_slice(&bx(b"infe", &infe));
		}
		let mut infe = vec![2u8, 0, 0, 0];
		infe.extend_from_slice(&7u16.to_be_bytes());
		infe.extend_from_slice(&0u16.to_be_bytes());
		infe.extend_from_slice(b"grid");
		infe.push(0);
		iinf.extend_from_slice(&bx(b"infe", &infe));
		let iinf = bx(b"iinf", &iinf);

		let mut pitm = vec![0u8; 4];
		pitm.extend_from_slice(&7u16.to_be_bytes());
		let pitm = bx(b"pitm", &pitm);

		// The grid item is derived from its six tiles, in raster order.
		let mut dimg = 7u16.to_be_bytes().to_vec();
		dimg.extend_from_slice(&6u16.to_be_bytes());
		for id in 1u16..=6 {
			dimg.extend_from_slice(&id.to_be_bytes());
		}
		let iref = bx(b"iref", &{
			let mut b = vec![0u8; 4];
			b.extend_from_slice(&bx(b"dimg", &dimg));
			b
		});

		// The grid's own bytes live in `idat`, which is where a derivation's do.
		let idat = bx(b"idat", &grid);
		let mut iloc = vec![1u8, 0, 0, 0];
		iloc.push(0x44);
		iloc.push(0x00);
		iloc.extend_from_slice(&1u16.to_be_bytes());
		iloc.extend_from_slice(&7u16.to_be_bytes());
		iloc.extend_from_slice(&1u16.to_be_bytes());
		iloc.extend_from_slice(&0u16.to_be_bytes());
		iloc.extend_from_slice(&1u16.to_be_bytes());
		iloc.extend_from_slice(&0u32.to_be_bytes());
		iloc.extend_from_slice(&(grid.len() as u32).to_be_bytes());
		let iloc = bx(b"iloc", &iloc);

		let mut meta = vec![0u8; 4];
		meta.extend_from_slice(&pitm);
		meta.extend_from_slice(&iinf);
		meta.extend_from_slice(&iref);
		meta.extend_from_slice(&iprp);
		meta.extend_from_slice(&idat);
		meta.extend_from_slice(&iloc);
		let mut file = bx(b"ftyp", b"heic\0\0\0\0mif1heic");
		file.extend_from_slice(&bx(b"meta", &meta));

		let heif = res!(Heif::read(&file));
		req!(res!(heif.size()), (1280, 960), "A grid was measured by one of its tiles.");
		match res!(heif.picture()) {
			Picture::Tiled { grid, tiles } => {
				req!(grid, geometry);
				req!(tiles, vec![1u32, 2, 3, 4, 5, 6]);
			},
			other => return Err(err!(
				"A grid was read as {:?}.", other; Test, Invalid)),
		}
		Ok(())
	}

	#[test]
	fn test_a_grid_naming_the_wrong_number_of_tiles_is_refused_04() -> Outcome<()> {
		// The same file as above with one tile taken out of the reference, which is the shape a
		// truncated copy takes and the shape that makes a decoder read past the end of its tiles.
		let mut grid = vec![0u8, 0, 2, 1];
		grid.extend_from_slice(&1280u16.to_be_bytes());
		grid.extend_from_slice(&960u16.to_be_bytes());
		let mut iinf = vec![0u8; 4];
		iinf.extend_from_slice(&2u16.to_be_bytes());
		let mut infe = vec![2u8, 0, 0, 0];
		infe.extend_from_slice(&1u16.to_be_bytes());
		infe.extend_from_slice(&0u16.to_be_bytes());
		infe.extend_from_slice(b"hvc1");
		infe.push(0);
		iinf.extend_from_slice(&bx(b"infe", &infe));
		let mut infe = vec![2u8, 0, 0, 0];
		infe.extend_from_slice(&7u16.to_be_bytes());
		infe.extend_from_slice(&0u16.to_be_bytes());
		infe.extend_from_slice(b"grid");
		infe.push(0);
		iinf.extend_from_slice(&bx(b"infe", &infe));
		let iinf = bx(b"iinf", &iinf);
		let mut pitm = vec![0u8; 4];
		pitm.extend_from_slice(&7u16.to_be_bytes());
		let pitm = bx(b"pitm", &pitm);
		let mut dimg = 7u16.to_be_bytes().to_vec();
		dimg.extend_from_slice(&1u16.to_be_bytes());
		dimg.extend_from_slice(&1u16.to_be_bytes());
		let iref = bx(b"iref", &{
			let mut b = vec![0u8; 4];
			b.extend_from_slice(&bx(b"dimg", &dimg));
			b
		});
		let idat = bx(b"idat", &grid);
		let mut iloc = vec![1u8, 0, 0, 0];
		iloc.push(0x44);
		iloc.push(0x00);
		iloc.extend_from_slice(&1u16.to_be_bytes());
		iloc.extend_from_slice(&7u16.to_be_bytes());
		iloc.extend_from_slice(&1u16.to_be_bytes());
		iloc.extend_from_slice(&0u16.to_be_bytes());
		iloc.extend_from_slice(&1u16.to_be_bytes());
		iloc.extend_from_slice(&0u32.to_be_bytes());
		iloc.extend_from_slice(&(grid.len() as u32).to_be_bytes());
		let iloc = bx(b"iloc", &iloc);
		let mut meta = vec![0u8; 4];
		meta.extend_from_slice(&pitm);
		meta.extend_from_slice(&iinf);
		meta.extend_from_slice(&iref);
		meta.extend_from_slice(&idat);
		meta.extend_from_slice(&iloc);
		let mut file = bx(b"ftyp", b"heic\0\0\0\0mif1heic");
		file.extend_from_slice(&bx(b"meta", &meta));

		let heif = res!(Heif::read(&file));
		req!(heif.picture().is_err(), true, "A grid of six tiles was read as a grid of one.");
		Ok(())
	}

	#[test]
	fn test_a_quarter_turn_exchanges_the_two_measurements_05() -> Outcome<()> {
		let mut file = one_picture();
		// An `irot` of one quarter turn, associated with the primary item. Rebuilding the whole
		// file is what it takes: the property has to go into `ipco` and its index into `ipma`.
		let mut ipco = Vec::new();
		let mut ispe_small = vec![0u8; 4];
		ispe_small.extend_from_slice(&512u32.to_be_bytes());
		ispe_small.extend_from_slice(&512u32.to_be_bytes());
		ipco.extend_from_slice(&bx(b"ispe", &ispe_small));
		let mut ispe_big = vec![0u8; 4];
		ispe_big.extend_from_slice(&4032u32.to_be_bytes());
		ispe_big.extend_from_slice(&3024u32.to_be_bytes());
		ipco.extend_from_slice(&bx(b"ispe", &ispe_big));
		ipco.extend_from_slice(&bx(b"hvcC", &[1, 2, 3, 4]));
		ipco.extend_from_slice(&bx(b"irot", &[1]));
		let iprp = bx(b"iprp", &bx(b"ipco", &ipco));
		let mut ipma = vec![0u8; 4];
		ipma.extend_from_slice(&1u32.to_be_bytes());
		ipma.extend_from_slice(&2u16.to_be_bytes());
		ipma.push(3);
		ipma.push(2);
		ipma.push(3);
		ipma.push(4);
		let ipma = bx(b"ipma", &ipma);
		let mut infe2 = vec![2u8, 0, 0, 0];
		infe2.extend_from_slice(&2u16.to_be_bytes());
		infe2.extend_from_slice(&0u16.to_be_bytes());
		infe2.extend_from_slice(b"hvc1");
		infe2.push(0);
		let mut iinf = vec![0u8; 4];
		iinf.extend_from_slice(&1u16.to_be_bytes());
		iinf.extend_from_slice(&bx(b"infe", &infe2));
		let iinf = bx(b"iinf", &iinf);
		let mut pitm = vec![0u8; 4];
		pitm.extend_from_slice(&2u16.to_be_bytes());
		let pitm = bx(b"pitm", &pitm);
		let mut meta = vec![0u8; 4];
		meta.extend_from_slice(&pitm);
		meta.extend_from_slice(&iinf);
		meta.extend_from_slice(&iprp);
		meta.extend_from_slice(&ipma);
		file = bx(b"ftyp", b"heic\0\0\0\0mif1heic");
		file.extend_from_slice(&bx(b"meta", &meta));

		let heif = res!(Heif::read(&file));
		req!(res!(heif.rotation()), 1);
		req!(res!(heif.size()), (3024, 4032), "A quarter turn left the measurements as they were.");
		Ok(())
	}
}

// ------------------------------------------------------------------- the whole photograph

/// Decodes a HEIC file into a picture.
///
/// The whole way: the container's boxes, the coded tiles, the HEVC decoder, the assembly of the
/// grid and the conversion out of colour difference into red, green and blue.
///
/// **The grid is larger than the photograph.** Tiles are whole coding units and a picture is not,
/// so the assembled grid is rounded up to the tile size and the photograph is cropped out of its
/// **top left**; the `ispe` property on the grid says how big the photograph is, and that is what
/// comes back here.
///
/// The camera's rotation is *not* applied. Ochre and everything like it turn a photograph by the
/// Exif orientation, and a phone writes both -- applying both turns a portrait twice.
pub fn decode(bytes: &[u8]) -> Outcome<Pixmap> {
	let (assembled, want) = res!(planes(bytes));
	let full = res!(hevc::colour::rgb(&assembled, hevc::colour::Matrix::Hd, false));
	// Cropped out of the top left, which is where the photograph sits in its grid.
	if want.0 >= full.width() && want.1 >= full.height() {
		return Ok(full);
	}
	crop(&full, want.0.min(full.width()), want.1.min(full.height()))
}

/// The same, stopping at the coded planes: brightness and colour difference, the grid assembled but
/// **not** cropped, and the size the photograph says it is.
///
/// Handed out so that a caller checking the assembly against another decoder can compare what came
/// out of the codec rather than what came out of a colour conversion neither of them is specified
/// to agree about.
pub fn planes(bytes: &[u8]) -> Outcome<(hevc::decode::Picture, (usize, usize))> {
	let heif = res!(Heif::read(bytes));
	let picture = res!(heif.picture());
	Ok(match picture {
		Picture::One { item, size } => {
			let config = res!(heif.config(item));
			let data = res!(heif.data(item));
			(res!(hevc::picture(config, &data)), (size.0 as usize, size.1 as usize))
		},
		Picture::Tiled { grid, tiles } => {
			let first = match tiles.first() {
				Some(t) => *t,
				None => return Err(err!("A grid with no tiles in it."; Invalid, Input, Decode)),
			};
			let config = res!(heif.config(first));
			// One tile decoded first, to learn how big a tile is; every tile of a grid shares one
			// decoder configuration, so they are all this size.
			let data = res!(heif.data(first));
			let one = res!(hevc::picture(config, &data));
			let (tw, th) = (one.y.w, one.y.h);
			let (gw, gh) = (tw * grid.cols as usize, th * grid.rows as usize);
			let mut out = hevc::decode::Picture {
				y:	hevc::decode::Plane::empty(gw, gh),
				cb:	hevc::decode::Plane::empty(gw / 2, gh / 2),
				cr:	hevc::decode::Plane::empty(gw / 2, gh / 2),
				depth:	one.depth,
			};
			for (i, tile) in tiles.iter().enumerate() {
				let (col, row) = (i % grid.cols as usize, i / grid.cols as usize);
				if row >= grid.rows as usize {
					break;
				}
				let piece = if i == 0 {
					one.clone()
				} else {
					let data = res!(heif.data(*tile));
					res!(hevc::picture(res!(heif.config(*tile)), &data))
				};
				out.paste(&piece, col * tw, row * th);
			}
			(out, (grid.width as usize, grid.height as usize))
		},
		Picture::Foreign { kind, .. } => return Err(err!(
			"This file holds {:?}, which is not HEVC.",
			String::from_utf8_lossy(&kind); Unimplemented)),
	})
}

/// The top left of a picture, at the size a photograph says it is.
fn crop(src: &Pixmap, w: usize, h: usize) -> Outcome<Pixmap> {
	let mut out = Vec::with_capacity(w * h * 4);
	let px = src.data();
	for y in 0..h {
		let row = y * src.width() * 4;
		out.extend_from_slice(&px[row..row + w * 4]);
	}
	Pixmap::from_data(w, h, out)
}
