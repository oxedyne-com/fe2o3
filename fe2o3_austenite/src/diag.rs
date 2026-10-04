//! Diagnostics for the evaluator path, replacing `lang::Refusals` at cut-over (U11). An evaluation
//! failure is recorded here with its span *and* returned as an `Err`, so an `Outcome` carries control flow
//! while the diagnostic list carries the position; `Engine::error` does both in one call.

use crate::eval::eval::std_path_bound;
use crate::eval::value::Type;
use crate::syntax::{
	Source,
	Span,
	SyntaxKind,
	SyntaxNode,
};

use oxedyne_fe2o3_core::prelude::*;

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
	Error,
	Warning,
}

impl Severity {
	pub fn name(self) -> &'static str {
		match self {
			Severity::Error		=> "error",
			Severity::Warning	=> "warning",
		}
	}
}

/// Why a diagnostic was raised, fixed where its refusal or error is raised and never read back from the
/// message's wording, which drifts. [`DiagnosticKind::as_str`] is the word a caller switches on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagnosticKind {
	MissingFile,		// a source, chapter, image or other file could not be found or read
	Encoding,			// a source or other text file that is there but is not valid UTF-8 text
	MissingFont,		// a named family that no supplied or embedded font declares
	Syntax,				// source that does not parse
	Type,				// a value of the wrong type
	UnknownVariable,	// a name with no binding in scope
	Package,			// a package import the host has not supplied
	Limit,				// a limit of the engine reached: layout that will not settle, a depth or a loop bound
	Unsupported,		// a construct passed over or refused, or one set with a stand-in for what it asked for
	Lint,				// a warning Typst itself raises about the source, which is set as written
	Internal,			// no pages, no content, or an error raised with no more specific tag
}

impl DiagnosticKind {
	/// The word the kind is carried as. A caller switches on it, so it never changes.
	pub fn as_str(&self) -> &'static str {
		match self {
			Self::MissingFile		=> "missing_file",
			Self::Encoding			=> "encoding",
			Self::MissingFont		=> "missing_font",
			Self::Syntax			=> "syntax",
			Self::Type				=> "type",
			Self::UnknownVariable	=> "unknown_variable",
			Self::Package			=> "package",
			Self::Limit				=> "limit",
			Self::Unsupported		=> "unsupported",
			Self::Lint				=> "lint",
			Self::Internal			=> "internal",
		}
	}

	/// Does a warning of this kind refuse a strict compile? It does where the document was not set as
	/// written: a construct passed over or set with a stand-in, or a file, its text, a package or a font
	/// family missing. A [`Lint`](Self::Lint) is Typst's own remark on a document it sets as written, so it
	/// stands beside the PDF.
	pub fn refuses_strict(&self) -> bool {
		matches!(self, Self::Unsupported | Self::MissingFile | Self::Encoding | Self::Package | Self::MissingFont)
	}

	/// The kind of a curated-path refusal, by the class it was refused under. Every class but a missing or
	/// unreadable file is a construct passed over or set with a stand-in.
	pub(crate) fn from_refusal_class(class: crate::lang::RefusalClass) -> Self {
		use crate::lang::RefusalClass;
		match class {
			RefusalClass::FixedPoint		=> Self::Unsupported,
			RefusalClass::Introspective		=> Self::Unsupported,
			RefusalClass::Unsupported		=> Self::Unsupported,
			RefusalClass::MissingFile		=> Self::MissingFile,
			RefusalClass::Unusable			=> Self::Unsupported,
			RefusalClass::Encoding			=> Self::Encoding,
		}
	}

	/// A hard error's kind, from the tags it was raised with anywhere in its chain. A file that is there
	/// but not UTF-8 text is told from one that cannot be read at all, since the remedy differs. An error
	/// raised as unimplemented is a construct passed over.
	pub fn from_error_tags(e: &Error<ErrTag>) -> Self {
		let tags = e.tags();
		if tags.contains(&ErrTag::Font) {
			Self::MissingFont
		} else if tags.contains(&ErrTag::UTF8) {
			Self::Encoding
		} else if tags.contains(&ErrTag::File) {
			Self::MissingFile
		} else if tags.contains(&ErrTag::LimitReached) {
			Self::Limit
		} else if tags.contains(&ErrTag::Unimplemented) || tags.contains(&ErrTag::NoImpl) {
			Self::Unsupported
		} else {
			Self::Internal
		}
	}
}

impl fmt::Display for DiagnosticKind {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "{}", self.as_str())
	}
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
	pub severity:	Severity,
	pub kind:		DiagnosticKind,
	pub span:		Span,
	pub message:	String,
	pub hints:		Vec<String>,
	pub trace:		Vec<(Span, String)>,	// call sites outward, innermost first
	pub need:		Option<String>,			// the package an import asked for that nobody supplied, as `@ns/name:version`
}

impl Diagnostic {
	pub fn error<S: Into<String>>(kind: DiagnosticKind, span: Span, message: S) -> Self {
		Self { severity: Severity::Error, kind, span, message: message.into(), hints: Vec::new(), trace: Vec::new(), need: None }
	}

	pub fn warning<S: Into<String>>(kind: DiagnosticKind, span: Span, message: S) -> Self {
		Self { severity: Severity::Warning, kind, span, message: message.into(), hints: Vec::new(), trace: Vec::new(), need: None }
	}

	pub fn with_hint<S: Into<String>>(mut self, hint: S) -> Self {
		self.hints.push(hint.into());
		self
	}

	/// Names the package an import asked for, so a host reads what to supply from the diagnostic, never from its
	/// message.
	pub fn needing<S: Into<String>>(mut self, spec: S) -> Self {
		self.need = Some(spec.into());
		self
	}

	pub fn is_error(&self) -> bool { self.severity == Severity::Error }

	/// The message up to its first colon: the name of what was passed over, before the particulars of
	/// where it was met.
	pub fn head(&self) -> &str {
		self.message.split(':').next().unwrap_or("").trim()
	}

	/// `path:line:col: severity: message`, then one `hint:` line each, against the sources the span
	/// indexes. A detached span or an unknown file renders without a position.
	pub fn render(&self, sources: &[Source]) -> String {
		let mut s = match sources.iter().find(|src| src.id == self.span.file) {
			Some(src) if !self.span.is_detached() => {
				let (l, c) = src.line_col(self.span.start);
				fmt!("{}:{}:{}: {}: {}", src.path.display(), l, c, self.severity.name(), self.message)
			}
			_ => fmt!("{}: {}", self.severity.name(), self.message),
		};
		for h in &self.hints {
			s.push_str("\n  hint: ");
			s.push_str(h);
		}
		s
	}
}

/// The lines of the `--diag-summary` report, one for each severity, kind and construct, as
/// `diag-summary <severity> <kind> <construct> <count>`, errors before warnings and each group in the
/// order of its kind's word and construct. The construct is given only for kind `unsupported`, as the
/// diagnostic's [`head`](Diagnostic::head) with each space written `_`; every other kind has `-`. No
/// message, path or position is carried, so the report names what was passed over and never the document
/// it was passed over in.
pub fn summary_lines(diags: &[Diagnostic]) -> Vec<String> {
	let mut counts: std::collections::BTreeMap<(Severity, &'static str, String), usize> =
		std::collections::BTreeMap::new();
	for d in diags {
		let construct = match d.kind {
			DiagnosticKind::Unsupported	=> {
				let head: String = d.head().chars()
					.map(|c| if c.is_whitespace() { '_' } else { c })
					.collect();
				if head.is_empty() { "-".to_string() } else { head }
			},
			_							=> "-".to_string(),
		};
		*counts.entry((d.severity, d.kind.as_str(), construct)).or_insert(0) += 1;
	}
	counts.into_iter()
		.map(|((sev, kind, construct), n)| fmt!("diag-summary {} {} {} {}", sev.name(), kind, construct, n))
		.collect()
}

/// The lines of the `--diag-summary` report that name each error, one per error diagnostic, as
/// `diag-error <kind> callee:<name> expected:<type> found:<type> file:<class>`. Every token is drawn from a
/// closed vocabulary, so the line says what failed in Typst's own terms and nothing of the document:
///
/// * `callee` is the call the error's span lies in, or failing that the name at the span, written as a
///   dotted path (`calc.pow`), when every segment is a name the standard library binds and the path is
///   lower-case letters, digits, `-` and `.`; otherwise `-`. A document's own function or variable is `-`.
/// * `expected` and `found` are the two types of a message that reads `expected <type>, found <type>` with
///   each a type in the registry ([`Type`]); otherwise both are `-`. A message that offers several types
///   is `-`.
/// * `file` is `main` for the compiled source (the first loaded), `sibling` for another file in its
///   directory, and `other` for anything else, a detached span included.
///
/// No message, name, path, position, field or source text of the document is carried.
pub fn error_lines(diags: &[Diagnostic], sources: &[Source]) -> Vec<String> {
	diags.iter()
		.filter(|d| d.is_error())
		.map(|d| {
			let (expected, found) = type_pair(&d.message).unwrap_or(("-", "-"));
			fmt!("diag-error {} callee:{} expected:{} found:{} file:{}",
				d.kind.as_str(), callee_of(d.span, sources), expected, found, file_class(d.span, sources))
		})
		.collect()
}

// The two registry types of `expected <type>, found <type>`, as their names, or `None` when the message is any
// other shape or either side is not exactly one registry type.
fn type_pair(message: &str) -> Option<(&'static str, &'static str)> {
	message.strip_prefix("expected ")
		.and_then(|m| m.split_once(", found "))
		.and_then(|(e, f)| Type::from_name(e).zip(Type::from_name(f)))
		.map(|(e, f)| (e.name(), f.name()))
}

// Where an error lies relative to the compiled source, which is the first one loaded.
fn file_class(span: Span, sources: &[Source]) -> &'static str {
	let at = match sources.iter().find(|s| s.id == span.file && !span.is_detached()) {
		Some(s)	=> s,
		None	=> return "other",
	};
	let main = match sources.first() {
		Some(m)	=> m,
		None	=> return "other",
	};
	if at.id == main.id {
		"main"
	} else if at.path.parent() == main.path.parent() {
		"sibling"
	} else {
		"other"
	}
}

// The standard-library name of the call an error's span lies in, else of the name at the span, else `-`.
fn callee_of(span: Span, sources: &[Source]) -> String {
	let src = match sources.iter().find(|s| s.id == span.file && !span.is_detached()) {
		Some(s)	=> s,
		None	=> return "-".to_string(),
	};
	// The nodes from the root down to the smallest one holding the span.
	let mut node = &src.root;
	let mut chain: Vec<&SyntaxNode> = vec![node];
	while let Some(c) = node.children().iter().find(|c| {
		let at = c.span();
		!at.is_detached() && at.start <= span.start && span.end <= at.end
	}) {
		chain.push(c);
		node = c;
	}
	let name = match chain.iter().rev().find(|n| matches!(n.kind(), SyntaxKind::FuncCall | SyntaxKind::MathCall)) {
		Some(call)	=> call.significant().next().and_then(path_of),
		None		=> chain.last().copied().and_then(path_of),
	};
	let name = match name {
		Some(n)	=> n,
		None	=> return "-".to_string(),
	};
	let segs: Vec<&str> = name.iter().map(|s| s.as_str()).collect();
	let word = segs.join(".");
	let plain = word.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.');
	if plain && !word.is_empty() && std_path_bound(&segs) { word } else { "-".to_string() }
}

// A name written as an identifier or a chain of field accesses on one, as its segments.
fn path_of(node: &SyntaxNode) -> Option<Vec<String>> {
	match node.kind() {
		SyntaxKind::Ident | SyntaxKind::MathIdent => Some(vec![node.text().to_string()]),
		SyntaxKind::FieldAccess | SyntaxKind::MathFieldAccess => {
			let target	= node.significant().next().and_then(path_of);
			let field	= node.significant().last()
				.filter(|f| matches!(f.kind(), SyntaxKind::Ident | SyntaxKind::MathIdent));
			match (target, field) {
				(Some(mut path), Some(f))	=> {
					path.push(f.text().to_string());
					Some(path)
				},
				_							=> None,
			}
		},
		_ => None,
	}
}

/// An error's own message, the last one pushed, without the chain of places it passed through: the
/// text a diagnostic shows for it.
pub fn message_of(e: &Error<ErrTag>) -> String {
	match e.msgs().into_iter().last() {
		Some(m)	=> m,
		None	=> fmt!("{}", e),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	// The eleven words and the strict set, which the Daimond gate's table maps: a caller switches on a word,
	// so neither moves without that table moving.
	const GATE: [(DiagnosticKind, &str, bool); 11] = [
		(DiagnosticKind::MissingFile,		"missing_file",		true),
		(DiagnosticKind::Encoding,			"encoding",			true),
		(DiagnosticKind::MissingFont,		"missing_font",		true),
		(DiagnosticKind::Syntax,			"syntax",			false),
		(DiagnosticKind::Type,				"type",				false),
		(DiagnosticKind::UnknownVariable,	"unknown_variable",	false),
		(DiagnosticKind::Package,			"package",			true),
		(DiagnosticKind::Limit,				"limit",			false),
		(DiagnosticKind::Unsupported,		"unsupported",		true),
		(DiagnosticKind::Lint,				"lint",				false),
		(DiagnosticKind::Internal,			"internal",			false),
	];

	// An exhaustive match, so a variant added to the enum fails here until the table above holds it.
	fn slot(k: DiagnosticKind) -> usize {
		match k {
			DiagnosticKind::MissingFile		=> 0,
			DiagnosticKind::Encoding		=> 1,
			DiagnosticKind::MissingFont		=> 2,
			DiagnosticKind::Syntax			=> 3,
			DiagnosticKind::Type			=> 4,
			DiagnosticKind::UnknownVariable	=> 5,
			DiagnosticKind::Package			=> 6,
			DiagnosticKind::Limit			=> 7,
			DiagnosticKind::Unsupported		=> 8,
			DiagnosticKind::Lint			=> 9,
			DiagnosticKind::Internal		=> 10,
		}
	}

	#[test]
	fn the_eleven_kind_words_and_the_strict_set_match_the_gates_table() {
		let mut seen = [false; 11];
		for (kind, word, strict) in GATE {
			seen[slot(kind)] = true;
			assert_eq!(kind.as_str(), word, "the word of {:?}", kind);
			assert_eq!(kind.to_string(), word, "the displayed word of {:?}", kind);
			assert_eq!(kind.refuses_strict(), strict, "does {:?} refuse a strict compile", kind);
		}
		assert!(seen.iter().all(|s| *s), "the gate's table leaves a kind out: {:?}", seen);
		assert_eq!(GATE.iter().filter(|g| g.2).count(), 5, "five kinds refuse a strict compile");
	}

	#[test]
	fn a_diagnostic_carries_the_kind_it_was_raised_with() {
		let at = Span::detached();
		let w = Diagnostic::warning(DiagnosticKind::Unsupported, at, "passed over");
		assert_eq!((w.severity, w.kind), (Severity::Warning, DiagnosticKind::Unsupported));
		let e = Diagnostic::error(DiagnosticKind::Syntax, at, "unclosed");
		assert_eq!((e.severity, e.kind), (Severity::Error, DiagnosticKind::Syntax));
		assert!(e.is_error() && !w.is_error());
	}

	#[test]
	fn an_error_kind_is_read_from_its_tags() {
		let kind = |e: Error<ErrTag>| DiagnosticKind::from_error_tags(&e);
		assert_eq!(kind(err!("not text"; Decode, UTF8, String)),	DiagnosticKind::Encoding);
		assert_eq!(kind(err!("no such file"; IO, File, Read)),		DiagnosticKind::MissingFile);
		assert_eq!(kind(err!("too deep"; Excessive, LimitReached)),	DiagnosticKind::Limit);
		assert_eq!(kind(err!("a stub"; Unimplemented)),				DiagnosticKind::Unsupported);
		assert_eq!(kind(err!("no tag of its own"; Input, Invalid)),	DiagnosticKind::Internal);
	}
}
