//! The settings of `austenite watch` and `austenite build`.
//!
//! One file, `austenite.jdat`, sits beside the document root, the directory that holds `assets/`, and is
//! found by walking up from the source. It is a jdat map. Every key is optional, so an absent key takes its
//! default and a file written before a key existed still reads. The precedence is the built-in default, then
//! the file, then each `--set key=value` of the command line, in order, for the one run.
//!
//! A key that is not in [`KEYS`], a value of the wrong type and a value outside its range are each refused
//! by the key's full dotted name before anything compiles. So is a setting that asks for what is not built
//! yet ([`Settings::check_built`]), rather than being ignored.

use crate::emit::pdf::PdfOptions;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::pdf::ColourOut;
use oxedyne_fe2o3_jdat::prelude::*;

use std::collections::BTreeMap;
use std::path::{
	Path,
	PathBuf,
};

pub const FILE: &str = "austenite.jdat";

// What a key's value may be.
#[derive(Clone, Copy, Debug)]
enum Kind {
	Bool,
	Num(u64, u64),
	Text,
	List,
	Pick(&'static [&'static str]),
}

impl Kind {
	fn say(&self) -> String {
		match self {
			Self::Bool			=> "true or false".to_string(),
			Self::Num(lo, hi)	=> fmt!("a whole number from {} to {}", lo, hi),
			Self::Text			=> "text".to_string(),
			Self::List			=> "a list of text".to_string(),
			Self::Pick(ws)		=> fmt!("one of {}", ws.iter().map(|w| fmt!("\"{}\"", w)).collect::<Vec<_>>().join(", ")),
		}
	}
}

const SPACES:	&[&str] = &["native", "rgb", "cmyk", "grey"];
const INTENTS:	&[&str] = &["perceptual", "relative", "saturation", "absolute"];
const BLACKS:	&[&str] = &["k", "rich"];
const VERSIONS:	&[&str] = &["1.4", "1.5", "1.6", "1.7"];
const DIAGS:	&[&str] = &["errors", "summary"];

// Every key, by its dotted path.
const KEYS: &[(&str, Kind)] = &[
	("root",				Kind::Text),
	("fonts",				Kind::List),
	("document",			Kind::Text),
	("output",				Kind::Text),
	("strict",				Kind::Bool),
	("diagnostics",			Kind::Pick(DIAGS)),
	("watch.poll_ms",		Kind::Num(10, 60_000)),
	("watch.warm",			Kind::Bool),
	("figs.dir",			Kind::Text),
	("figs.render",			Kind::Bool),
	("colour.space",		Kind::Pick(SPACES)),
	("colour.rgb_profile",	Kind::Text),
	("colour.grey_profile",	Kind::Text),
	("colour.cmyk_profile",	Kind::Text),
	("colour.intent",		Kind::Pick(INTENTS)),
	("colour.black_point",	Kind::Bool),
	("colour.black",		Kind::Pick(BLACKS)),
	("metadata.document",	Kind::Bool),
	("metadata.engine",		Kind::Bool),
	("pdf.version",			Kind::Pick(VERSIONS)),
	("pdf.compress",		Kind::Bool),
	("pdf.outline",			Kind::Bool),
	("view.open",			Kind::Bool),
	("view.app",			Kind::Text),
];

#[derive(Clone, Debug, FromDatMap)]
pub struct Watch {
	#[optional] pub poll_ms:	u64,
	#[optional] pub warm:		bool,
}

impl Default for Watch {
	fn default() -> Self { Self { poll_ms: 200, warm: true } }
}

#[derive(Clone, Debug, FromDatMap)]
pub struct Figs {
	#[optional] pub dir:		String,
	#[optional] pub render:		bool,
}

impl Default for Figs {
	fn default() -> Self { Self { dir: "figs".to_string(), render: true } }
}

#[derive(Clone, Debug, FromDatMap)]
pub struct Colour {
	#[optional] pub space:			String,
	#[optional] pub rgb_profile:	String,
	#[optional] pub grey_profile:	String,
	#[optional] pub cmyk_profile:	String,
	#[optional] pub intent:			String,
	#[optional] pub black_point:	bool,
	#[optional] pub black:			String,
}

impl Default for Colour {
	fn default() -> Self {
		Self {
			space:			"native".to_string(),
			rgb_profile:	"srgb".to_string(),
			grey_profile:	"sgray".to_string(),
			cmyk_profile:	"fogra39".to_string(),
			intent:			"perceptual".to_string(),
			black_point:	true,
			black:			"k".to_string(),
		}
	}
}

#[derive(Clone, Debug, Default, FromDatMap)]
pub struct Metadata {
	#[optional] pub document:	bool,
	#[optional] pub engine:		bool,
}

#[derive(Clone, Debug, FromDatMap)]
pub struct Pdf {
	#[optional] pub version:	String,
	#[optional] pub compress:	bool,
	#[optional] pub outline:	bool,
}

impl Default for Pdf {
	fn default() -> Self { Self { version: "1.7".to_string(), compress: true, outline: true } }
}

#[derive(Clone, Debug, FromDatMap)]
pub struct View {
	#[optional] pub open:	bool,
	#[optional] pub app:	String,
}

impl Default for View {
	fn default() -> Self { Self { open: true, app: "auto".to_string() } }
}

/// The settings of a run. `root` and `document` are empty when unset.
#[derive(Clone, Debug, FromDatMap)]
pub struct Settings {
	#[optional] pub root:			String,
	#[optional] pub fonts:			Vec<String>,
	#[optional] pub document:		String,
	#[optional] pub output:			String,
	#[optional] pub strict:			bool,
	#[optional] pub diagnostics:	String,
	#[skip] pub watch:		Watch,
	#[skip] pub figs:		Figs,
	#[skip] pub colour:		Colour,
	#[skip] pub metadata:	Metadata,
	#[skip] pub pdf:		Pdf,
	#[skip] pub view:		View,
}

impl Default for Settings {
	fn default() -> Self {
		Self {
			root:			String::new(),
			fonts:			vec!["assets/fonts".to_string()],
			document:		String::new(),
			output:			"{stem}.pdf".to_string(),
			strict:			false,
			diagnostics:	"errors".to_string(),
			watch:			Watch::default(),
			figs:			Figs::default(),
			colour:			Colour::default(),
			metadata:		Metadata::default(),
			pdf:			Pdf::default(),
			view:			View::default(),
		}
	}
}

// The entries of a map of either kind.
fn pairs(d: &Dat) -> Option<Vec<(&Dat, &Dat)>> {
	match d {
		Dat::Map(m)		=> Some(m.iter().collect()),
		Dat::OrdMap(m)	=> Some(m.iter().map(|(k, v)| (k.dat(), v)).collect()),
		_				=> None,
	}
}

fn key_of(path: &str) -> Option<Kind> {
	KEYS.iter().find(|(p, _)| *p == path).map(|(_, k)| *k)
}

// Does `path` name a group, a map of keys, and not a key?
fn is_group(path: &str) -> bool {
	let dotted = fmt!("{}.", path);
	KEYS.iter().any(|(p, _)| p.starts_with(&dotted))
}

// Does the value suit the kind, and lie in its range?
fn suits(path: &str, kind: Kind, v: &Dat) -> Outcome<()> {
	let ok = match kind {
		Kind::Bool		=> v.get_bool().is_some(),
		Kind::Num(lo, hi)	=> v.get_u64().map(|n| n >= lo && n <= hi).unwrap_or(false),
		Kind::Text		=> v.get_string().is_some(),
		Kind::List		=> v.get_string_list().is_some(),
		Kind::Pick(ws)	=> v.get_string().map(|s| ws.contains(&s.as_str())).unwrap_or(false),
	};
	if ok { Ok(()) } else {
		Err(err!("The setting '{}' must be {}, not {}.", path, kind.say(), shown(v); Input, Invalid))
	}
}

// A value for an error message, short.
fn shown(v: &Dat) -> String {
	match v {
		Dat::Str(s)	=> fmt!("\"{}\"", s),
		Dat::Bool(b)	=> fmt!("{}", b),
		other		=> fmt!("{:?}", other),
	}
}

// Every leaf of the map, as a dotted path, refusing a key that is unknown and a value that does not suit.
fn leaves<'a>(prefix: &str, d: &'a Dat, out: &mut Vec<(String, &'a Dat)>) -> Outcome<()> {
	let entries = match pairs(d) {
		Some(e)	=> e,
		None	=> return Err(err!(
			"The settings must be a map of keys, and '{}' is not one.", if prefix.is_empty() { "the file" } else { prefix };
			Input, Invalid)),
	};
	for (k, v) in entries {
		let name = match k {
			Dat::Str(s)	=> s.clone(),
			other		=> return Err(err!("A settings key must be text, not {:?}.", other; Input, Invalid)),
		};
		let path = if prefix.is_empty() { name } else { fmt!("{}.{}", prefix, name) };
		if is_group(&path) {
			res!(leaves(&path, v, out));
			continue;
		}
		match key_of(&path) {
			Some(kind)	=> {
				res!(suits(&path, kind, v));
				out.push((path, v));
			},
			None		=> return Err(err!(
				"The setting '{}' is not one Austenite has. The keys are: {}.",
				path, KEYS.iter().map(|(p, _)| *p).collect::<Vec<_>>().join(", "); Input, Invalid)),
		}
	}
	Ok(())
}

// The sub-map at `name` of the top map, as a plain map.
fn group(top: &Dat, name: &str) -> DaticleMap {
	let mut m = DaticleMap::new();
	if let Some(entries) = pairs(top) {
		for (k, v) in entries {
			if k == &Dat::Str(name.to_string()) {
				if let Some(inner) = pairs(v) {
					for (ik, iv) in inner {
						m.insert(ik.clone(), iv.clone());
					}
				}
			}
		}
	}
	m
}

impl Settings {
	/// Reads the settings from a decoded map, refusing what [`KEYS`] does not allow.
	pub fn from_dat(top: &Dat) -> Outcome<Self> {
		let mut seen = Vec::new();
		res!(leaves("", top, &mut seen));
		let mut flat = DaticleMap::new();
		if let Some(entries) = pairs(top) {
			for (k, v) in entries {
				flat.insert(k.clone(), v.clone());
			}
		}
		let mut s = res!(Settings::from_datmap(flat));
		s.watch		= res!(Watch::from_datmap(group(top, "watch")));
		s.figs		= res!(Figs::from_datmap(group(top, "figs")));
		s.colour	= res!(Colour::from_datmap(group(top, "colour")));
		s.metadata	= res!(Metadata::from_datmap(group(top, "metadata")));
		s.pdf		= res!(Pdf::from_datmap(group(top, "pdf")));
		s.view		= res!(View::from_datmap(group(top, "view")));
		Ok(s)
	}

	/// The settings of one run: the defaults, then the file `file` when there is one, then each `--set`.
	pub fn resolve(file: Option<&Path>, sets: &[String]) -> Outcome<Self> {
		let mut top = Dat::Map(BTreeMap::new());
		if let Some(f) = file {
			let text = res!(std::fs::read_to_string(f));
			if !text.trim().is_empty() {
				top = match Dat::decode_string(text) {
					Ok(d)	=> d,
					Err(e)	=> return Err(err!(e, "The settings file {} cannot be read.", f.display(); Input, Invalid)),
				};
			}
			let mut seen = Vec::new();
			if let Err(e) = leaves("", &top, &mut seen) {
				return Err(err!(e, "In the settings file {}.", f.display(); Input, Invalid));
			}
		}
		for set in sets {
			let (path, val) = res!(parse_set(set));
			res!(top.map_put_dotted(&path, val));
		}
		Self::from_dat(&top)
	}

	/// Refuses a setting that asks for what is not built, by the key's name. `doc_dir` is the document's own
	/// directory, where the figure sources would be.
	pub fn check_built(&self, doc_dir: &Path) -> Outcome<()> {
		let c = &self.colour;
		if c.space == "cmyk" || c.space == "grey" {
			return Err(err!("The setting 'colour.space' = \"{}\" is not built yet. Use \"native\" or \"rgb\".", c.space; Input, Invalid));
		}
		let defaults = Colour::default();
		for (key, got, want) in [
			("colour.rgb_profile",	&c.rgb_profile,		&defaults.rgb_profile),
			("colour.grey_profile",	&c.grey_profile,	&defaults.grey_profile),
			("colour.cmyk_profile",	&c.cmyk_profile,	&defaults.cmyk_profile),
			("colour.intent",		&c.intent,			&defaults.intent),
			("colour.black",		&c.black,			&defaults.black),
		] {
			if got != want {
				return Err(err!("The setting '{}' = \"{}\" is not built yet; only \"{}\" can be used.", key, got, want; Input, Invalid));
			}
		}
		if !c.black_point {
			return Err(err!("The setting 'colour.black_point' = false is not built yet; only true can be used."; Input, Invalid));
		}
		if self.figs.render && doc_dir.join(&self.figs.dir).is_dir() {
			return Err(err!(
				"The setting 'figs.render' is on and the figures directory '{}' exists beside the document, but \
				figure rendering is not built yet. Set figs.render=false to go on.", self.figs.dir; Input, Invalid));
		}
		Ok(())
	}

	/// How the PDF is written under these settings.
	pub fn pdf_options(&self) -> Outcome<PdfOptions> {
		let minor = match self.pdf.version.strip_prefix("1.").and_then(|m| m.parse::<u8>().ok()) {
			Some(m)	=> m,
			None	=> return Err(err!("The setting 'pdf.version' = \"{}\" is not 1.4 to 1.7.", self.pdf.version; Input, Invalid)),
		};
		Ok(PdfOptions {
			minor,
			compress:		self.pdf.compress,
			outline:		self.pdf.outline,
			doc_info:		self.metadata.document,
			engine_info:	self.metadata.engine,
			colour:			if self.colour.space == "rgb" { ColourOut::Rgb } else { ColourOut::Native },
		})
	}

	/// The finished PDF's path for `main`: `output` with `{stem}` filled in, beside the source unless absolute.
	pub fn output_path(&self, main: &Path) -> PathBuf {
		let stem = main.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
		let name = self.output.replace("{stem}", &stem);
		let p = PathBuf::from(name);
		if p.is_absolute() { p } else {
			main.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or_else(|| Path::new(".")).join(p)
		}
	}

	/// The command that opens a viewer, none when `view.open` is off or no viewer is installed.
	pub fn viewer(&self) -> Option<String> {
		if !self.view.open {
			return None;
		}
		if self.view.app != "auto" {
			return Some(self.view.app.clone());
		}
		["papers", "evince", "xdg-open"].iter().find(|c| on_path(c)).map(|c| c.to_string())
	}
}

fn on_path(cmd: &str) -> bool {
	match std::env::var_os("PATH") {
		Some(path)	=> std::env::split_paths(&path).any(|d| d.join(cmd).is_file()),
		None		=> false,
	}
}

// One `--set key=value`: the key, and the value typed by what the key takes. A list is comma-separated.
fn parse_set(arg: &str) -> Outcome<(String, Dat)> {
	let (key, text) = match arg.split_once('=') {
		Some(kv)	=> kv,
		None		=> return Err(err!("--set takes key=value, and '{}' has no '='.", arg; Input, Invalid)),
	};
	let kind = match key_of(key) {
		Some(k)	=> k,
		None	=> return Err(err!(
			"--set names the setting '{}', which is not one Austenite has. The keys are: {}.",
			key, KEYS.iter().map(|(p, _)| *p).collect::<Vec<_>>().join(", "); Input, Invalid)),
	};
	let val = match kind {
		Kind::Bool		=> match text {
			"true"	=> Dat::Bool(true),
			"false"	=> Dat::Bool(false),
			_		=> return Err(err!("The setting '{}' must be {}, not \"{}\".", key, kind.say(), text; Input, Invalid)),
		},
		Kind::Num(..)	=> match text.parse::<u64>() {
			Ok(n)	=> Dat::U64(n),
			Err(_)	=> return Err(err!("The setting '{}' must be {}, not \"{}\".", key, kind.say(), text; Input, Invalid)),
		},
		Kind::List		=> Dat::List(text.split(',').filter(|t| !t.is_empty()).map(|t| Dat::Str(t.trim().to_string())).collect()),
		Kind::Text | Kind::Pick(_)	=> Dat::Str(text.to_string()),
	};
	res!(suits(key, kind, &val));
	Ok((key.to_string(), val))
}

/// The settings file for a source: the first `austenite.jdat` in its directory or above it.
pub fn find(start: &Path) -> Option<PathBuf> {
	let dir = if start.is_dir() { start.to_path_buf() } else {
		start.parent().filter(|d| !d.as_os_str().is_empty()).map(|d| d.to_path_buf()).unwrap_or_else(|| PathBuf::from("."))
	};
	let mut at = std::fs::canonicalize(&dir).ok();
	while let Some(d) = at {
		let f = d.join(FILE);
		if f.is_file() {
			return Some(f);
		}
		at = d.parent().map(|p| p.to_path_buf());
	}
	None
}

// The quoted paths that follow `import` and `include` in a source, as a file name each.
fn referenced(text: &str) -> Vec<String> {
	let mut out = Vec::new();
	for word in ["import", "include"] {
		let mut from = 0;
		while let Some(i) = text[from..].find(word) {
			let after = from + i + word.len();
			from = after;
			let rest = text[after..].trim_start();
			if let Some(q) = rest.strip_prefix('"') {
				if let Some(end) = q.find('"') {
					out.push(q[..end].to_string());
				}
			}
		}
	}
	out
}

/// The `.typ` in `dir` that no sibling imports or includes. When there are several, they are listed and
/// refused; when there is none, that is refused too.
pub fn select_document(dir: &Path) -> Outcome<PathBuf> {
	let mut typs = Vec::new();
	for e in res!(std::fs::read_dir(dir)) {
		let p = res!(e).path();
		if p.extension().map(|x| x == "typ").unwrap_or(false) && p.is_file() {
			typs.push(p);
		}
	}
	typs.sort();
	let mut used = std::collections::BTreeSet::new();
	for p in &typs {
		let text = std::fs::read_to_string(p).unwrap_or_default();
		for r in referenced(&text) {
			if let Ok(c) = std::fs::canonicalize(dir.join(&r)) {
				used.insert(c);
			}
		}
	}
	let roots: Vec<PathBuf> = typs.iter()
		.filter(|p| std::fs::canonicalize(p).map(|c| !used.contains(&c)).unwrap_or(true))
		.cloned()
		.collect();
	match roots.len() {
		0	=> Err(err!("No document in {}: every .typ there is imported by another. Name one.", dir.display(); Input, Missing)),
		1	=> Ok(roots[0].clone()),
		_	=> Err(err!(
			"More than one document in {} is imported by no other: {}. Name one.",
			dir.display(), roots.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).collect::<Vec<_>>().join(", ");
			Input, Invalid)),
	}
}
