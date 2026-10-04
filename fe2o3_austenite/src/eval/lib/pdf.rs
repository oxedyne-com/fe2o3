// The `pdf` module of Typst 0.15: `pdf.attach` and `pdf.artifact`. Austenite writes neither an embedded file nor a
// tagged structure. An attachment is therefore passed over with a warning that refuses a strict compile, as every
// construct set otherwise than Typst sets it does; an artifact is its body, since marking a body as a page
// furnishing changes the tags of a PDF and not what it shows.

use crate::diag::DiagnosticKind;
use crate::eval::args::Args;
use crate::eval::content::Content;
use crate::eval::func::{
	Func,
	NativeFunc,
};
use crate::eval::import::{
	read_file,
	resolve_path,
};
use crate::eval::lib::foundations::{
	define_nth,
	finish,
	mismatch,
	need,
};
use crate::eval::scope::Scope;
use crate::eval::value::{
	Module,
	Value,
};
use crate::eval::Engine;
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

use std::sync::Arc;

native_fns! {
	pub enum PdfFn {
		Attach		=> "attach",
		Artifact	=> "artifact",
	}
}

// What an attachment may be to the document, and what a body may be to the page.
const RELATIONSHIPS:	[&str; 4] = ["source", "data", "alternative", "supplement"];
const KINDS:			[&str; 12] = [
	"header", "footer", "watermark", "page-number", "line-number", "redaction", "bates", "page",
	"pagination-other", "layout", "background", "other",
];

pub fn define(scope: &mut Scope) {
	let mut s = Scope::new();
	for (n, f) in PdfFn::ALL.iter().enumerate() {
		define_nth(&mut s, n, f.name(), Value::Func(Func::Native(NativeFunc::Pdf(*f))));
	}
	scope.define("pdf", Value::Module(Arc::new(Module::new("pdf", s))));
}

// The words of `expected "a", "b", or "c"` for a closed set of strings, with `none` among them where it is allowed.
fn one_of(set: &[&str], none: bool) -> String {
	let mut items: Vec<String> = set.iter().map(|s| fmt!("\"{}\"", s)).collect();
	if none {
		items.push("none".to_string());
	}
	match items.split_last() {
		Some((last, rest))	=> fmt!("expected {}, or {}", rest.join(", "), last),
		None				=> String::new(),
	}
}

// Where Typst places an error in a named argument: at its value.
fn value_span(args: &Args, name: &str) -> Span {
	args.items.iter().find(|a| a.name.as_deref() == Some(name)).map(|a| a.value_span).unwrap_or(args.span)
}

pub fn call(f: PdfFn, engine: &mut Engine, mut args: Args) -> Outcome<Value> {
	let span = args.span;
	match f {
		PdfFn::Attach => {
			// A path that names no file fails at the argument naming it.
			let at = args.items.iter().find(|a| a.name.is_none()).map(|a| a.span).unwrap_or(span);
			let (at_rel, at_mime, at_desc) = (
				value_span(&args, "relationship"), value_span(&args, "mime-type"), value_span(&args, "description"));
			let path = match res!(need(engine, &mut args, "path")).symbol_as_str() {
				Value::Str(s)	=> s,
				other			=> return Err(mismatch(engine, span, "string", &other)),
			};
			// The data is the second positional, and never a named argument.
			let data		= res!(args.eat::<Value>());
			let relationship = res!(args.named::<Value>("relationship"));
			let mime		= res!(args.named::<Value>("mime-type"));
			let description	= res!(args.named::<Value>("description"));
			res!(finish(engine, args));
			match relationship {
				None | Some(Value::None)						=> (),
				Some(Value::Str(ref s)) if RELATIONSHIPS.contains(&s.as_str())	=> (),
				Some(_)											=> {
					return Err(engine.error(DiagnosticKind::Type, at_rel, one_of(&RELATIONSHIPS, true)));
				},
			}
			for (v, here) in [(&mime, at_mime), (&description, at_desc)] {
				match v {
					None | Some(Value::None) | Some(Value::Str(_))	=> (),
					Some(other)	=> return Err(mismatch(engine, here, "string or none", other)),
				}
			}
			match data {
				Some(Value::Bytes(_))	=> (),
				// Typst reads the file the path names when no data is given, so a path that names none fails.
				None | Some(Value::Auto)	=> {
					let file = res!(resolve_path(engine, &path, at.file, at));
					let _ = res!(read_file(engine, &file, at));
				},
				Some(other)	=> return Err(mismatch(engine, span, "bytes or auto", &other)),
			}
			engine.warn(DiagnosticKind::Unsupported, span, "pdf attachments are not embedded");
			Ok(Value::Content(Content::empty()))
		},
		PdfFn::Artifact => {
			let at_kind	= value_span(&args, "kind");
			let body	= res!(need(engine, &mut args, "body"));
			let kind	= res!(args.named::<Value>("kind"));
			res!(finish(engine, args));
			match kind {
				None						=> (),
				Some(Value::Str(ref s)) if KINDS.contains(&s.as_str())	=> (),
				Some(Value::Str(_))			=> return Err(engine.error(DiagnosticKind::Type, at_kind, one_of(&KINDS, false))),
				Some(other)					=> {
					let msg = fmt!("{}, found {}", one_of(&KINDS, false), other.ty().long_name());
					return Err(engine.error(DiagnosticKind::Type, at_kind, msg));
				},
			}
			match body {
				Value::Content(c)	=> Ok(Value::Content(c)),
				Value::Str(s)		=> Ok(Value::Content(Content::text(&s))),
				other				=> Err(mismatch(engine, span, "content", &other)),
			}
		},
	}
}
