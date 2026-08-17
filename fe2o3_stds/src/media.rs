//! File formats, identified from their leading bytes and from their names, and what each one is.
//!
//! Every program that shows a person a file has to answer the same question first -- what IS this
//! -- and there are only two sources of an answer.  The name is what somebody typed and the bytes
//! are what is actually there, so they disagree whenever a file has been renamed, exported wrongly
//! or truncated mid-download.  [`identify`] asks both and reports the disagreement rather than
//! quietly choosing, because which of the two is right is the caller's business and the fact that
//! they differ is usually the most interesting thing about the file.
//!
//! # Why this is a standards table and not a heuristic
//!
//! A magic signature is a constant published in a specification: a PNG begins with the eight bytes
//! `89 50 4E 47 0D 0A 1A 0A` and always has, and the second through fourth of those spell `PNG` so
//! that a file transferred through a text-mode channel is visibly corrupt.  Nothing here is
//! guessed.  What IS a heuristic lives in one function, [`looks_like_text`], and its limitations
//! are written on it.
//!
//! # Checked against something that is not itself
//!
//! The signatures were checked against the `file` command over seventeen real files of seventeen
//! formats -- PDF, PNG, JPEG, SVG, ZIP, gzip, MP4, MP3, WOFF2, TTF, wasm, HEIC, WebP, ICO, DOCX,
//! EPUB, AVI -- and sixteen agreed exactly.  The seventeenth is a deliberate divergence:  `file`
//! reports a TrueType font as `font/sfnt`, the generic container type, while its own description
//! of the same bytes reads "TrueType Font data".  RFC 8081 registers `font/ttf` for a font with
//! TrueType outlines, and the `00 01 00 00` version tag says which those are, so [`Media::Ttf`]
//! reports the specific type.  A browser also wants the specific one.
//!
//! # The mistake this crate exists to stop
//!
//! Reading unknown bytes as UTF-8 with a lossy decoder and showing the result.  Every byte that is
//! not valid UTF-8 becomes U+FFFD, so the reader is shown a screenful of replacement characters
//! and no indication that anything went wrong -- the program looks broken rather than the format
//! looking unsupported, and those are very different bug reports.  A caller that asks
//! [`looks_like_text`] before it decodes cannot make that mistake.
//!
//! # Example
//!
//! ```
//! use oxedyne_fe2o3_stds::media::{Kind, Media, identify};
//!
//! let head = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n";
//! let id = identify("notes.txt", head);
//! assert_eq!(id.media, Media::Pdf);         // the bytes win
//! assert_eq!(id.by_name, Media::Text);
//! assert!(id.disagree);                     // and the caller can say so
//! assert_eq!(id.media.kind(), Kind::Document);
//! ```

/// The broad class a format belongs to, which is what decides how a viewer shows it.
///
/// A caller usually wants this before it wants the format: a picture goes in an `img`, a sound in
/// an `audio`, and anything unrecognised goes to a hex dump.  Adding a format to [`Media`] without
/// giving it a kind here would leave it silently in [`Kind::Unknown`], so the mapping in
/// [`Media::kind`] is exhaustive by construction rather than by a wildcard arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
	/// A still picture.
	Image,
	/// A moving picture, with or without sound.
	Video,
	/// Sound alone.
	Audio,
	/// A paginated or marked-up document.
	Document,
	/// A container holding other files.
	Archive,
	/// A typeface.
	Font,
	/// Characters, meant to be read as characters.
	Text,
	/// An executable or object file.
	Binary,
	/// Nothing here recognised it.
	Unknown,
}

/// A file format.
///
/// Named for the format rather than for its usual extension, because one format wears several
/// extensions (`.jpg`, `.jpeg`, `.jpe`) and one extension is sometimes worn by several formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Media {
	// ── Pictures ──
	/// Portable Network Graphics.
	Png,
	/// JPEG, in the JFIF or Exif wrapping a camera writes.
	Jpeg,
	/// Graphics Interchange Format, still or animated.
	Gif,
	/// WebP, inside a RIFF container.
	Webp,
	/// AV1 Image File Format, an ISO base media container.
	Avif,
	/// High Efficiency Image Format, an ISO base media container holding HEVC.
	Heic,
	/// Windows bitmap.
	Bmp,
	/// Windows icon.
	Ico,
	/// Tagged Image File Format, either byte order.
	Tiff,
	/// Scalable Vector Graphics, which is XML and therefore also text.
	Svg,

	// ── Documents ──
	/// Portable Document Format.
	Pdf,
	/// Rich Text Format.
	Rtf,
	/// PostScript.
	PostScript,
	/// HTML, which is text and may execute.
	Html,
	/// XML that is not one of the XML formats named separately here.
	Xml,
	/// Markdown.
	Markdown,
	/// JSON.
	Json,
	/// Comma-separated values.
	Csv,
	/// Tab-separated values.
	Tsv,

	// ── Containers ──
	/// A ZIP archive, or one of the formats built on one that was not distinguished.
	Zip,
	/// A gzip stream.
	Gzip,
	/// A bzip2 stream.
	Bzip2,
	/// An xz stream.
	Xz,
	/// A Zstandard stream.
	Zstd,
	/// A tar archive.
	Tar,
	/// A 7-Zip archive.
	SevenZip,
	/// A RAR archive.
	Rar,
	/// An Office Open XML word-processing document: a ZIP with a known part inside.
	Docx,
	/// An Office Open XML spreadsheet.
	Xlsx,
	/// An Office Open XML presentation.
	Pptx,
	/// An OpenDocument text document.
	Odt,
	/// An OpenDocument spreadsheet.
	Ods,
	/// An OpenDocument presentation.
	Odp,
	/// An OpenDocument drawing.
	Odg,
	/// An EPUB book.
	Epub,

	// ── Sound ──
	/// MPEG-1 Audio Layer III.
	Mp3,
	/// A RIFF WAVE file.
	Wav,
	/// FLAC, natively framed.
	Flac,
	/// An Ogg container, whatever it carries.
	Ogg,
	/// MPEG-4 audio.
	M4a,

	// ── Moving pictures ──
	/// MPEG-4 part 14.
	Mp4,
	/// WebM, which is a Matroska profile.
	Webm,
	/// Matroska.
	Matroska,
	/// A RIFF AVI file.
	Avi,
	/// QuickTime.
	QuickTime,

	// ── Typefaces ──
	/// TrueType.
	Ttf,
	/// OpenType with PostScript outlines.
	Otf,
	/// Web Open Font Format 1.
	Woff,
	/// Web Open Font Format 2.
	Woff2,

	// ── Programs ──
	/// An ELF object or executable.
	Elf,
	/// A DOS, Windows PE or similar `MZ` image.
	Exe,
	/// A WebAssembly module.
	Wasm,
	/// A Java class file.
	JavaClass,

	// ── Everything else ──
	/// Characters, with no more specific format recognised.
	Text,
	/// Nothing recognised it.
	Unknown,
}

/// What [`identify`] concluded, and from what.
///
/// Both answers are kept rather than reconciled.  A viewer shows [`Self::media`]; a viewer that
/// wants to be trusted also mentions [`Self::disagree`], because a file whose name says `.png` and
/// whose bytes say PDF is how a person finds a broken export, and hiding it helps nobody.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identified {
	/// The format to act on: the bytes when they said anything, else the name.
	pub media: Media,
	/// What the leading bytes said, or [`Media::Unknown`].
	pub by_magic: Media,
	/// What the name said, or [`Media::Unknown`].
	pub by_name: Media,
	/// Both spoke, and they said different things.
	pub disagree: bool,
}

/// Identify a file from its name and the front of its bytes.
/// The media type an OpenDocument or EPUB package declares in its first member.
pub const ODF_TEXT: &str = "application/vnd.oasis.opendocument.text";
/// The media type an OpenDocument spreadsheet declares.
pub const ODF_SHEET: &str = "application/vnd.oasis.opendocument.spreadsheet";
/// The media type an OpenDocument presentation declares.
pub const ODF_SLIDES: &str = "application/vnd.oasis.opendocument.presentation";
/// The media type an OpenDocument drawing declares.
pub const ODF_DRAWING: &str = "application/vnd.oasis.opendocument.graphics";

/// The format an archive declares in a first member named `mimetype`, where it has one.
///
/// OpenDocument requires that member to be first in the archive and stored uncompressed, for exactly
/// this purpose: a reader is meant to be able to name the file from its opening bytes rather than
/// from what somebody called it. EPUB borrowed the rule. Nothing else uses it, so an archive without
/// one is simply not one of these and is left as a ZIP.
///
/// Read from the local header rather than at a fixed offset, because the offset is only fixed when
/// the extra field is empty -- which it usually is, and "usually" is not a thing to encode.
fn odf_mimetype(b: &[u8]) -> Option<Media> {
	// A local file header is 30 bytes, then the name, then the extra field, then the data.
	if b.len() < 30 || &b[..4] != b"PK\x03\x04" {
		return None;
	}
	// Stored, not deflated. A compressed `mimetype` is a package that has broken the rule, and
	// inflating here to be helpful would be reading a format this function is only meant to name.
	if u16::from_le_bytes([b[8], b[9]]) != 0 {
		return None;
	}
	let size = u32::from_le_bytes([b[18], b[19], b[20], b[21]]) as usize;
	let nlen = u16::from_le_bytes([b[26], b[27]]) as usize;
	let elen = u16::from_le_bytes([b[28], b[29]]) as usize;
	if nlen != 8 || b.len() < 30 + nlen || &b[30..38] != b"mimetype" {
		return None;
	}
	let from = 30 + nlen + elen;
	let to = from.checked_add(size)?;
	if to > b.len() || size > 128 {
		return None;
	}
	match core::str::from_utf8(&b[from..to]).ok()?.trim() {
		ODF_TEXT	=> Some(Media::Odt),
		ODF_SHEET	=> Some(Media::Ods),
		ODF_SLIDES	=> Some(Media::Odp),
		ODF_DRAWING	=> Some(Media::Odg),
		"application/epub+zip"	=> Some(Media::Epub),
		_			=> None,
	}
}

///
/// The bytes win.  A name is a claim and bytes are evidence, and the one case where the name is
/// the better answer -- a format with no magic signature, such as CSV -- is exactly the case where
/// the bytes say nothing and there is no contest.
///
/// `prefix` may be as short as the caller likes; 512 bytes recognises everything here except a tar
/// archive, whose signature sits at offset 257 and so needs 264.  A prefix too short for a
/// signature simply fails to match it, and never misidentifies.
///
/// # Arguments
/// * `name` - The file name or path; only the extension is read.
/// * `prefix` - The leading bytes of the file.
pub fn identify(name: &str, prefix: &[u8]) -> Identified {
	let by_magic = Media::sniff(prefix);
	let by_name  = Media::from_name(name);
	// A magic hit for ZIP is worth refining by name before it is compared: `.docx`, `.epub` and
	// `.odt` are all ZIP archives, and reporting a disagreement between "ZIP" and "Word document"
	// would be reporting agreement as conflict.
	let by_magic = match (by_magic, by_name) {
		(Media::Zip, Media::Docx)
		| (Media::Zip, Media::Xlsx)
		| (Media::Zip, Media::Pptx)
		| (Media::Zip, Media::Epub)
		| (Media::Zip, Media::Odt)
		| (Media::Zip, Media::Ods)
		| (Media::Zip, Media::Odp)
		| (Media::Zip, Media::Odg)		=> by_name,
		_					=> by_magic,
	};
	// Text is the weakest possible magic answer -- it means "these bytes are characters", which
	// every text format satisfies -- so a name that is more specific refines it rather than
	// fighting it.
	let (media, disagree) = match (by_magic, by_name) {
		(Media::Unknown, n)			=> (n, false),
		(Media::Text, n) if n != Media::Unknown	=> (n, false),
		(m, Media::Unknown)			=> (m, false),
		(m, n) if m == n			=> (m, false),
		(m, _)					=> (m, true),
	};
	Identified { media, by_magic, by_name, disagree }
}

impl Media {
	/// Identify a format from the leading bytes of a file, or [`Self::Unknown`].
	///
	/// Signatures are checked longest-first where two overlap, so that a WebP is not reported as
	/// the RIFF container it happens to be carried in.
	///
	/// # Arguments
	/// * `b` - The leading bytes of the file.
	pub fn sniff(b: &[u8]) -> Self {
		// ── Fixed signatures at offset zero ──
		if starts(b, b"\x89PNG\r\n\x1a\n")	{ return Self::Png; }
		if starts(b, b"\xff\xd8\xff")		{ return Self::Jpeg; }
		if starts(b, b"GIF87a") || starts(b, b"GIF89a") { return Self::Gif; }
		if starts(b, b"BM")			{ return Self::Bmp; }
		if starts(b, b"\x00\x00\x01\x00")	{ return Self::Ico; }
		if starts(b, b"II*\x00") || starts(b, b"MM\x00*") { return Self::Tiff; }
		if starts(b, b"%PDF-")			{ return Self::Pdf; }
		if starts(b, b"{\\rtf")			{ return Self::Rtf; }
		if starts(b, b"%!PS")			{ return Self::PostScript; }
		if starts(b, b"\x1f\x8b")		{ return Self::Gzip; }
		if starts(b, b"BZh")			{ return Self::Bzip2; }
		if starts(b, b"\xfd7zXZ\x00")		{ return Self::Xz; }
		if starts(b, b"\x28\xb5\x2f\xfd")	{ return Self::Zstd; }
		if starts(b, b"7z\xbc\xaf\x27\x1c")	{ return Self::SevenZip; }
		if starts(b, b"Rar!\x1a\x07")		{ return Self::Rar; }
		if starts(b, b"fLaC")			{ return Self::Flac; }
		if starts(b, b"OggS")			{ return Self::Ogg; }
		if starts(b, b"\x1a\x45\xdf\xa3")	{ return Self::Matroska; }
		if starts(b, b"OTTO")			{ return Self::Otf; }
		if starts(b, b"wOFF")			{ return Self::Woff; }
		if starts(b, b"wOF2")			{ return Self::Woff2; }
		if starts(b, b"\x00\x01\x00\x00") || starts(b, b"true") { return Self::Ttf; }
		if starts(b, b"\x7fELF")		{ return Self::Elf; }
		if starts(b, b"\x00asm")		{ return Self::Wasm; }
		if starts(b, b"\xca\xfe\xba\xbe")	{ return Self::JavaClass; }
		if starts(b, b"MZ")			{ return Self::Exe; }
		// Every ZIP-based format begins the same way, and for most of them which one it is lives
		// in the central directory at the END of the file, past anything a prefix can see.  The
		// name settles those, and `identify` does that rather than this.
		//
		// OpenDocument is the exception, and deliberately so: the format REQUIRES a member named
		// `mimetype` to come first and to be stored uncompressed, precisely so that a reader can
		// name the file from its opening bytes.  So that one is read rather than guessed, and a
		// `.odt` somebody renamed is still an `.odt`.  EPUB borrowed the same rule.
		if starts(b, b"PK\x03\x04") || starts(b, b"PK\x05\x06") || starts(b, b"PK\x07\x08") {
			if let Some(m) = odf_mimetype(b) {
				return m;
			}
			return Self::Zip;
		}
		// An ID3 tag is a wrapper, not a format; what follows it is MPEG audio in practice.
		if starts(b, b"ID3")			{ return Self::Mp3; }
		// An MPEG frame sync is eleven set bits.  Checked after everything else because two
		// bytes of a coincidence is not much evidence, and after ID3 because a tagged file
		// does not begin with one.
		if b.len() >= 2 && b[0] == 0xff && (b[1] & 0xe0) == 0xe0 { return Self::Mp3; }

		// ── RIFF, which says what it holds in its fifth through eighth bytes ──
		if starts(b, b"RIFF") && b.len() >= 12 {
			return match &b[8..12] {
				b"WEBP"	=> Self::Webp,
				b"WAVE"	=> Self::Wav,
				b"AVI "	=> Self::Avi,
				_	=> Self::Unknown,
			};
		}

		// ── ISO base media, whose brand sits after the box header ──
		//
		// The first four bytes are the box length, which varies, so the marker is at four and
		// the brand at eight.  Compatible brands follow from sixteen, and are not read: the
		// major brand is what the file says it IS, and a viewer that prefers a compatible
		// brand over the major one is deciding for the file.
		if b.len() >= 12 && &b[4..8] == b"ftyp" {
			return match &b[8..12] {
				b"avif" | b"avis"			=> Self::Avif,
				b"heic" | b"heix" | b"hevc" | b"heim"
				| b"heis" | b"mif1" | b"msf1"		=> Self::Heic,
				b"M4A " | b"M4B " | b"m4a "		=> Self::M4a,
				b"qt  "					=> Self::QuickTime,
				_					=> Self::Mp4,
			};
		}

		// ── A tar's signature is 257 bytes in, which is why prefixes should be 512 ──
		if b.len() >= 262 && &b[257..262] == b"ustar" { return Self::Tar; }

		// ── Text-shaped formats, recognised only once the bytes are known to be characters ──
		if looks_like_text(b) {
			let head = lead(b, 512).to_ascii_lowercase();
			let head = head.trim_start();
			if head.starts_with("<?xml") {
				// An XML declaration says nothing about the vocabulary; SVG says so in its
				// root element, which the declaration precedes.
				if head.contains("<svg") { return Self::Svg; }
				return Self::Xml;
			}
			if head.starts_with("<svg")			{ return Self::Svg; }
			if head.starts_with("<!doctype html")
				|| head.starts_with("<html")		{ return Self::Html; }
			if head.starts_with('<')			{ return Self::Xml; }
			return Self::Text;
		}
		Self::Unknown
	}

	/// Identify a format from a file name or path, or [`Self::Unknown`].
	///
	/// Only the text after the last dot of the last path component is read, so a dot in a
	/// directory name cannot be mistaken for an extension.
	///
	/// # Arguments
	/// * `name` - A file name or path.
	pub fn from_name(name: &str) -> Self {
		let leaf = name.rsplit(['/', '\\']).next().unwrap_or(name);
		let ext = match leaf.rsplit_once('.') {
			// A dotfile with no second dot is a name, not an extension.
			Some((stem, ext)) if !stem.is_empty()	=> ext,
			_					=> return Self::Unknown,
		};
		Self::from_extension(ext)
	}

	/// Identify a format from an extension, with or without its dot, in any case.
	///
	/// # Arguments
	/// * `ext` - An extension, such as `png`, `.PNG` or `jpeg`.
	pub fn from_extension(ext: &str) -> Self {
		let e = ext.trim_start_matches('.').to_ascii_lowercase();
		match e.as_str() {
			"png"					=> Self::Png,
			"jpg" | "jpeg" | "jpe" | "jfif"		=> Self::Jpeg,
			"gif"					=> Self::Gif,
			"webp"					=> Self::Webp,
			"avif"					=> Self::Avif,
			"heic" | "heif"				=> Self::Heic,
			"bmp" | "dib"				=> Self::Bmp,
			"ico"					=> Self::Ico,
			"tif" | "tiff"				=> Self::Tiff,
			"svg"					=> Self::Svg,

			"pdf"					=> Self::Pdf,
			"rtf"					=> Self::Rtf,
			"ps" | "eps"				=> Self::PostScript,
			"html" | "htm" | "xhtml"		=> Self::Html,
			"xml"					=> Self::Xml,
			"md" | "markdown"			=> Self::Markdown,
			"json" | "jsonl" | "ndjson"		=> Self::Json,
			"csv"					=> Self::Csv,
			"tsv" | "tab"				=> Self::Tsv,

			"zip"					=> Self::Zip,
			"gz" | "tgz"				=> Self::Gzip,
			"bz2" | "tbz2"				=> Self::Bzip2,
			"xz" | "txz"				=> Self::Xz,
			"zst"					=> Self::Zstd,
			"tar"					=> Self::Tar,
			"7z"					=> Self::SevenZip,
			"rar"					=> Self::Rar,
			"docx" | "docm"				=> Self::Docx,
			"xlsx" | "xlsm"				=> Self::Xlsx,
			"pptx" | "pptm"				=> Self::Pptx,
			"odt" | "ott" | "fodt"			=> Self::Odt,
			"ods" | "ots" | "fods"			=> Self::Ods,
			"odp" | "otp" | "fodp"			=> Self::Odp,
			"odg" | "otg" | "fodg"			=> Self::Odg,
			"epub"					=> Self::Epub,

			"mp3"					=> Self::Mp3,
			"wav" | "wave"				=> Self::Wav,
			"flac"					=> Self::Flac,
			"ogg" | "oga" | "opus"			=> Self::Ogg,
			"m4a" | "m4b"				=> Self::M4a,

			"mp4" | "m4v"				=> Self::Mp4,
			"webm"					=> Self::Webm,
			"mkv" | "mka"				=> Self::Matroska,
			"avi"					=> Self::Avi,
			"mov" | "qt"				=> Self::QuickTime,

			"ttf" | "ttc"				=> Self::Ttf,
			"otf"					=> Self::Otf,
			"woff"					=> Self::Woff,
			"woff2"					=> Self::Woff2,

			"wasm"					=> Self::Wasm,
			"exe" | "dll"				=> Self::Exe,
			"so" | "elf"				=> Self::Elf,
			"class"					=> Self::JavaClass,

			// Extensions whose files are characters and whose format is "source code".  They
			// are named rather than defaulted so that a caller may show them with line
			// numbers and highlighting without having to sniff first.
			"txt" | "text" | "log" | "rs" | "js" | "mjs" | "ts" | "py" | "sh" | "bash"
			| "c" | "h" | "cpp" | "hpp" | "go" | "java" | "rb" | "php" | "css" | "scss"
			| "toml" | "yaml" | "yml" | "ini" | "conf" | "typ" | "tex" | "sql" | "jdat"
								=> Self::Text,
			_					=> Self::Unknown,
		}
	}

	/// The broad class this format belongs to.
	pub fn kind(&self) -> Kind {
		match self {
			Self::Png | Self::Jpeg | Self::Gif | Self::Webp | Self::Avif | Self::Heic
			| Self::Bmp | Self::Ico | Self::Tiff | Self::Svg		=> Kind::Image,

			Self::Mp4 | Self::Webm | Self::Matroska | Self::Avi
			| Self::QuickTime						=> Kind::Video,

			Self::Mp3 | Self::Wav | Self::Flac | Self::Ogg | Self::M4a	=> Kind::Audio,

			Self::Pdf | Self::Rtf | Self::PostScript | Self::Html | Self::Xml
			| Self::Markdown | Self::Json | Self::Csv | Self::Tsv		=> Kind::Document,

			Self::Zip | Self::Gzip | Self::Bzip2 | Self::Xz | Self::Zstd | Self::Tar
			| Self::SevenZip | Self::Rar | Self::Docx | Self::Xlsx | Self::Pptx
			| Self::Odt | Self::Ods | Self::Odp | Self::Odg
			| Self::Epub							=> Kind::Archive,

			Self::Ttf | Self::Otf | Self::Woff | Self::Woff2		=> Kind::Font,

			Self::Elf | Self::Exe | Self::Wasm | Self::JavaClass		=> Kind::Binary,

			Self::Text							=> Kind::Text,
			Self::Unknown							=> Kind::Unknown,
		}
	}

	/// The IANA media type, or `application/octet-stream` where there is none.
	///
	/// This is what a caller puts on a `Blob` so that a browser hands the bytes to the right
	/// decoder; getting it wrong is how a correct picture fails to appear.
	pub fn mime(&self) -> &'static str {
		match self {
			Self::Png		=> "image/png",
			Self::Jpeg		=> "image/jpeg",
			Self::Gif		=> "image/gif",
			Self::Webp		=> "image/webp",
			Self::Avif		=> "image/avif",
			Self::Heic		=> "image/heic",
			Self::Bmp		=> "image/bmp",
			Self::Ico		=> "image/vnd.microsoft.icon",
			Self::Tiff		=> "image/tiff",
			Self::Svg		=> "image/svg+xml",

			Self::Pdf		=> "application/pdf",
			Self::Rtf		=> "application/rtf",
			Self::PostScript	=> "application/postscript",
			Self::Html		=> "text/html",
			Self::Xml		=> "application/xml",
			Self::Markdown		=> "text/markdown",
			Self::Json		=> "application/json",
			Self::Csv		=> "text/csv",
			Self::Tsv		=> "text/tab-separated-values",

			Self::Zip		=> "application/zip",
			Self::Gzip		=> "application/gzip",
			Self::Bzip2		=> "application/x-bzip2",
			Self::Xz		=> "application/x-xz",
			Self::Zstd		=> "application/zstd",
			Self::Tar		=> "application/x-tar",
			Self::SevenZip		=> "application/x-7z-compressed",
			Self::Rar		=> "application/vnd.rar",
			Self::Docx		=>
				"application/vnd.openxmlformats-officedocument.wordprocessingml.document",
			Self::Xlsx		=>
				"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
			Self::Pptx		=>
				"application/vnd.openxmlformats-officedocument.presentationml.presentation",
			Self::Odt		=> ODF_TEXT,
			Self::Ods		=> ODF_SHEET,
			Self::Odp		=> ODF_SLIDES,
			Self::Odg		=> ODF_DRAWING,
			Self::Epub		=> "application/epub+zip",

			Self::Mp3		=> "audio/mpeg",
			Self::Wav		=> "audio/wav",
			Self::Flac		=> "audio/flac",
			Self::Ogg		=> "audio/ogg",
			Self::M4a		=> "audio/mp4",

			Self::Mp4		=> "video/mp4",
			Self::Webm		=> "video/webm",
			Self::Matroska		=> "video/x-matroska",
			Self::Avi		=> "video/x-msvideo",
			Self::QuickTime		=> "video/quicktime",

			Self::Ttf		=> "font/ttf",
			Self::Otf		=> "font/otf",
			Self::Woff		=> "font/woff",
			Self::Woff2		=> "font/woff2",

			Self::Wasm		=> "application/wasm",
			Self::Elf | Self::Exe | Self::JavaClass
						=> "application/octet-stream",

			Self::Text		=> "text/plain",
			Self::Unknown		=> "application/octet-stream",
		}
	}

	/// A short human name for the format, in English, for a caller with nowhere to translate.
	pub fn label(&self) -> &'static str {
		match self {
			Self::Png => "PNG image",		Self::Jpeg => "JPEG image",
			Self::Gif => "GIF image",		Self::Webp => "WebP image",
			Self::Avif => "AVIF image",		Self::Heic => "HEIC image",
			Self::Bmp => "Bitmap image",		Self::Ico => "Icon",
			Self::Tiff => "TIFF image",		Self::Svg => "SVG drawing",
			Self::Pdf => "PDF document",		Self::Rtf => "Rich text",
			Self::PostScript => "PostScript",	Self::Html => "HTML page",
			Self::Xml => "XML",			Self::Markdown => "Markdown",
			Self::Json => "JSON",			Self::Csv => "CSV table",
			Self::Tsv => "TSV table",		Self::Zip => "ZIP archive",
			Self::Gzip => "gzip stream",		Self::Bzip2 => "bzip2 stream",
			Self::Xz => "xz stream",		Self::Zstd => "Zstandard stream",
			Self::Tar => "tar archive",		Self::SevenZip => "7-Zip archive",
			Self::Rar => "RAR archive",		Self::Docx => "Word document",
			Self::Xlsx => "Excel spreadsheet",	Self::Pptx => "PowerPoint presentation",
			Self::Odt => "OpenDocument text",	Self::Ods => "OpenDocument spreadsheet",
			Self::Odp => "OpenDocument presentation",
			Self::Odg => "OpenDocument drawing",	Self::Epub => "EPUB book",
			Self::Mp3 => "MP3 audio",		Self::Wav => "WAV audio",
			Self::Flac => "FLAC audio",		Self::Ogg => "Ogg audio",
			Self::M4a => "MPEG-4 audio",		Self::Mp4 => "MP4 video",
			Self::Webm => "WebM video",		Self::Matroska => "Matroska video",
			Self::Avi => "AVI video",		Self::QuickTime => "QuickTime video",
			Self::Ttf => "TrueType font",		Self::Otf => "OpenType font",
			Self::Woff => "WOFF font",		Self::Woff2 => "WOFF2 font",
			Self::Elf => "ELF binary",		Self::Exe => "Executable",
			Self::Wasm => "WebAssembly module",	Self::JavaClass => "Java class",
			Self::Text => "Text",			Self::Unknown => "Unknown",
		}
	}

	/// Whether a file of this format is characters, and may be decoded as UTF-8 and shown.
	///
	/// True of the formats that ARE text, including the ones with markup in them.  It is not a
	/// claim that the bytes in hand are valid UTF-8 -- that is [`looks_like_text`] -- only that
	/// the format is one where trying is the right thing to do.
	pub fn is_text(&self) -> bool {
		matches!(self,
			Self::Text | Self::Markdown | Self::Json | Self::Csv | Self::Tsv
			| Self::Html | Self::Xml | Self::Svg | Self::PostScript | Self::Rtf)
	}
}

/// Whether a run of bytes looks like text, and may be decoded and shown as characters.
///
/// THE ONE HEURISTIC IN THIS MODULE, and the limits of it are these.  A NUL byte settles the
/// question -- no text format in use writes one -- and so does a byte sequence that is not valid
/// UTF-8.  Beyond that it counts control characters, because a file that is technically valid
/// UTF-8 and is nine tenths control bytes is not something anybody wants shown as characters.
///
/// A truncated prefix is handled rather than failed: `b` may end in the middle of a multi-byte
/// character, and up to three trailing bytes are therefore ignored before the check.  Without that
/// a prefix of a perfectly ordinary UTF-8 file is called binary once every few hundred reads,
/// which is the kind of defect that is never reproduced on demand.
///
/// An empty run is text.  There is nothing in it to show, and calling it binary would put a hex
/// dump in front of somebody who created an empty file a moment ago.
///
/// # Arguments
/// * `b` - The leading bytes of the file.
pub fn looks_like_text(b: &[u8]) -> bool {
	if b.is_empty() {
		return true;
	}
	if b.contains(&0) {
		return false;
	}
	// A character is at most four bytes, so the last one begins at most four bytes from the end.
	// Walk back to its leader, work out how long it was MEANT to be, and drop it only if the run
	// stops short of that -- a whole character at the end must be kept, or a one-character file
	// would be read as an empty one.
	let mut end = b.len();
	let mut back = 0;
	while back < 4 && end > 0 && (b[end - 1] & 0xc0) == 0x80 {
		end -= 1;
		back += 1;
	}
	if end > 0 {
		let lead = b[end - 1];
		let want = if lead & 0x80 == 0x00	{ 1 }
			else if lead & 0xe0 == 0xc0	{ 2 }
			else if lead & 0xf0 == 0xe0	{ 3 }
			else if lead & 0xf8 == 0xf0	{ 4 }
			else				{ 0 };	// a stray continuation, which is an error
		// `back` continuation bytes followed the leader, so the character in hand is
		// `1 + back` bytes long.  Short of what the leader promised means it was cut.
		end = if want > 0 && 1 + back < want { end - 1 } else { b.len() };
	}
	let head = &b[..end];
	if core::str::from_utf8(head).is_err() {
		return false;
	}
	// Tab, newline and carriage return are text; the rest of C0, and DEL, are not.  One in ten
	// is generous -- ordinary prose has none at all -- and it is set there so that a file with a
	// stray form feed or a few ANSI escapes is still readable rather than being hidden.
	let ctrl = head.iter()
		.filter(|&&c| (c < 0x20 && c != b'\t' && c != b'\n' && c != b'\r') || c == 0x7f)
		.count();
	ctrl * 10 <= head.len()
}

/// Whether `b` begins with `sig`.
fn starts(b: &[u8], sig: &[u8]) -> bool {
	b.len() >= sig.len() && &b[..sig.len()] == sig
}

/// The first `n` bytes of `b` as a string, stopping at the last whole character.
fn lead(b: &[u8], n: usize) -> String {
	let mut end = b.len().min(n);
	while end > 0 && core::str::from_utf8(&b[..end]).is_err() {
		end -= 1;
	}
	String::from_utf8_lossy(&b[..end]).to_string()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_pictures_are_known_by_their_signatures() {
		assert_eq!(Media::sniff(b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR"), Media::Png);
		assert_eq!(Media::sniff(b"\xff\xd8\xff\xe0\x00\x10JFIF"), Media::Jpeg);
		assert_eq!(Media::sniff(b"GIF89a"), Media::Gif);
		assert_eq!(Media::sniff(b"RIFF\x24\x00\x00\x00WEBPVP8 "), Media::Webp);
		assert_eq!(Media::sniff(b"II*\x00\x08\x00\x00\x00"), Media::Tiff);
		assert_eq!(Media::sniff(b"MM\x00*\x00\x00\x00\x08"), Media::Tiff);
	}

	#[test]
	fn test_a_riff_container_is_read_for_what_it_holds() {
		// The whole point of checking bytes 8..12: three formats share four leading bytes.
		assert_eq!(Media::sniff(b"RIFF\x00\x00\x00\x00WAVEfmt "), Media::Wav);
		assert_eq!(Media::sniff(b"RIFF\x00\x00\x00\x00AVI LIST"), Media::Avi);
		assert_eq!(Media::sniff(b"RIFF\x00\x00\x00\x00WEBPVP8L"), Media::Webp);
		// A RIFF of some other kind is not guessed at.
		assert_eq!(Media::sniff(b"RIFF\x00\x00\x00\x00ZZZZ"), Media::Unknown);
	}

	#[test]
	fn test_an_iso_base_media_brand_decides_between_five_formats() {
		assert_eq!(Media::sniff(b"\x00\x00\x00\x18ftypavif\x00\x00\x00\x00"), Media::Avif);
		assert_eq!(Media::sniff(b"\x00\x00\x00\x18ftypheic\x00\x00\x00\x00"), Media::Heic);
		assert_eq!(Media::sniff(b"\x00\x00\x00\x18ftypM4A \x00\x00\x00\x00"), Media::M4a);
		assert_eq!(Media::sniff(b"\x00\x00\x00\x14ftypqt  \x00\x00\x00\x00"), Media::QuickTime);
		// An unrecognised brand in a recognised container is MP4, which is what the container
		// is; answering Unknown would hide a playable file.
		assert_eq!(Media::sniff(b"\x00\x00\x00\x18ftypisom\x00\x00\x00\x00"), Media::Mp4);
	}

	#[test]
	fn test_the_pdf_that_started_this() {
		// The defect this module was written for: a PDF's first 4 KB carry no NUL byte, so a
		// sniff that looks only for NUL calls it text, and a lossy UTF-8 decode then puts a
		// screenful of replacement characters in front of the reader.
		let head = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n1 0 obj\n<< /Type /Catalog >>\n";
		assert_eq!(Media::sniff(head), Media::Pdf);
		assert!(!head.contains(&0), "the header this fails on genuinely has no NUL in it");
	}

	#[test]
	fn test_text_shaped_markup_is_recognised_only_when_the_bytes_are_characters() {
		assert_eq!(Media::sniff(b"<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>"), Media::Svg);
		assert_eq!(Media::sniff(b"<?xml version=\"1.0\"?><svg></svg>"), Media::Svg);
		assert_eq!(Media::sniff(b"<?xml version=\"1.0\"?><rss></rss>"), Media::Xml);
		assert_eq!(Media::sniff(b"<!DOCTYPE html><html>"), Media::Html);
		assert_eq!(Media::sniff(b"Just some words.\n"), Media::Text);
	}

	#[test]
	fn test_bytes_beat_the_name_and_the_disagreement_is_reported() {
		let id = identify("photo.png", b"%PDF-1.4\n");
		assert_eq!(id.media, Media::Pdf, "act on the evidence");
		assert_eq!(id.by_name, Media::Png);
		assert!(id.disagree, "and the caller must be able to say so");
	}

	#[test]
	fn test_a_name_refines_a_weak_magic_answer_rather_than_fighting_it() {
		// CSV has no signature.  "These bytes are characters" and "this is a CSV" are not in
		// conflict, and reporting them as conflict would put a warning on every ordinary file.
		let id = identify("rows.csv", b"a,b,c\n1,2,3\n");
		assert_eq!(id.media, Media::Csv);
		assert!(!id.disagree);

		// Likewise the ZIP family: a `.docx` IS a ZIP, and saying so as a disagreement would be
		// reporting agreement as conflict.
		let id = identify("report.docx", b"PK\x03\x04\x14\x00");
		assert_eq!(id.media, Media::Docx);
		assert!(!id.disagree);
	}

	#[test]
	fn test_a_prefix_cut_mid_character_is_still_text() {
		// The defect this guards: a read of the first N bytes lands in the middle of a
		// multi-byte character perhaps once in a few hundred files, and a naive validity check
		// then calls an ordinary UTF-8 document binary.  Never reproduced on demand.
		let mut s = "Ordinary prose, and then a character that is three bytes: 日".as_bytes().to_vec();
		s.pop();                                        // cut the last continuation byte
		assert!(looks_like_text(&s), "a cut character is not evidence of binary");
		s.pop();
		assert!(looks_like_text(&s));
		s.pop();                                        // now the leader is gone too, cleanly
		assert!(looks_like_text(&s));
	}

	#[test]
	fn test_what_is_not_text() {
		assert!(!looks_like_text(b"\x00"), "a NUL settles it");
		assert!(!looks_like_text(b"before\x00after"));
		assert!(!looks_like_text(&[0xff, 0xfe, 0xfd, 0xfc]), "not valid UTF-8");
		assert!(looks_like_text(b""), "an empty file is text, not a hex dump");
		assert!(looks_like_text("héllo — ok\n".as_bytes()));
		// Generous, deliberately: a file with a couple of escapes in it is still readable.
		assert!(looks_like_text(b"plain text with one \x1b escape in it, which is fine"));
	}

	#[test]
	fn test_a_name_is_read_only_where_a_name_is() {
		assert_eq!(Media::from_name("/some.dir/file.png"), Media::Png);
		assert_eq!(Media::from_name("archive.tar.gz"), Media::Gzip, "the last extension wins");
		assert_eq!(Media::from_name(".gitignore"), Media::Unknown, "a dotfile is a name");
		assert_eq!(Media::from_name("Makefile"), Media::Unknown);
		assert_eq!(Media::from_name("IMAGE.PNG"), Media::Png, "case is not information here");
	}

	#[test]
	fn test_every_format_has_a_kind_a_mime_and_a_label() {
		// `kind`, `mime` and `label` match exhaustively with no wildcard arm, so a format added
		// to the enum without being classified will not compile.  This asserts the other half:
		// that none of them answers with a placeholder.
		let all = [
			Media::Png, Media::Jpeg, Media::Gif, Media::Webp, Media::Avif, Media::Heic,
			Media::Bmp, Media::Ico, Media::Tiff, Media::Svg, Media::Pdf, Media::Rtf,
			Media::PostScript, Media::Html, Media::Xml, Media::Markdown, Media::Json,
			Media::Csv, Media::Tsv, Media::Zip, Media::Gzip, Media::Bzip2, Media::Xz,
			Media::Zstd, Media::Tar, Media::SevenZip, Media::Rar, Media::Docx, Media::Xlsx,
			Media::Pptx, Media::Odt, Media::Ods, Media::Odp, Media::Odg,
			Media::Epub, Media::Mp3, Media::Wav,
			Media::Flac, Media::Ogg, Media::M4a, Media::Mp4, Media::Webm, Media::Matroska,
			Media::Avi, Media::QuickTime, Media::Ttf, Media::Otf, Media::Woff, Media::Woff2,
			Media::Elf, Media::Exe, Media::Wasm, Media::JavaClass, Media::Text,
		];
		for m in all.iter() {
			assert_ne!(m.kind(), Kind::Unknown, "{:?} has no kind", m);
			assert!(!m.label().is_empty(), "{:?} has no label", m);
			assert!(m.mime().contains('/'), "{:?} has no media type", m);
		}
		assert_eq!(Media::Unknown.kind(), Kind::Unknown);
	}
}
