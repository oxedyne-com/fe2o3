//! Diagnostics for the evaluator path, replacing `lang::Refusals` at cut-over (U11). An evaluation
//! failure is recorded here with its span *and* returned as an `Err`, so an `Outcome` carries control flow
//! while the diagnostic list carries the position; `Engine::error` does both in one call.

use crate::syntax::{
	Source,
	Span,
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
			Self::Internal			=> "internal",
		}
	}

	/// Does a warning of this kind refuse a strict compile? It does where the document was not set as
	/// written: a construct passed over or set with a stand-in, or a file, its text, a package or a font
	/// family missing.
	pub fn refuses_strict(&self) -> bool {
		matches!(self, Self::Unsupported | Self::MissingFile | Self::Encoding | Self::Package | Self::MissingFont)
	}

	/// A hard error's kind, from the tags it was raised with anywhere in its chain. A file that is there
	/// but not UTF-8 text is told from one that cannot be read at all, since the remedy differs. An error
	/// raised as unimplemented is a construct passed over.
	pub fn from_error_tags(e: &Error<ErrTag>) -> Self {
		let tags = e.tags();
		if tags.contains(&ErrTag::UTF8) {
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
}

impl Diagnostic {
	pub fn error<S: Into<String>>(kind: DiagnosticKind, span: Span, message: S) -> Self {
		Self { severity: Severity::Error, kind, span, message: message.into(), hints: Vec::new(), trace: Vec::new() }
	}

	pub fn warning<S: Into<String>>(kind: DiagnosticKind, span: Span, message: S) -> Self {
		Self { severity: Severity::Warning, kind, span, message: message.into(), hints: Vec::new(), trace: Vec::new() }
	}

	pub fn with_hint<S: Into<String>>(mut self, hint: S) -> Self {
		self.hints.push(hint.into());
		self
	}

	pub fn is_error(&self) -> bool { self.severity == Severity::Error }

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

	// The ten words and the strict set, pinned against the Daimond gate's table (`compile.rs`, the
	// `DiagnosticKind` block): a caller switches on a word, so neither moves without that table moving.
	const GATE: [(DiagnosticKind, &str, bool); 10] = [
		(DiagnosticKind::MissingFile,		"missing_file",		true),
		(DiagnosticKind::Encoding,			"encoding",			true),
		(DiagnosticKind::MissingFont,		"missing_font",		true),
		(DiagnosticKind::Syntax,			"syntax",			false),
		(DiagnosticKind::Type,				"type",				false),
		(DiagnosticKind::UnknownVariable,	"unknown_variable",	false),
		(DiagnosticKind::Package,			"package",			true),
		(DiagnosticKind::Limit,				"limit",			false),
		(DiagnosticKind::Unsupported,		"unsupported",		true),
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
			DiagnosticKind::Internal		=> 9,
		}
	}

	#[test]
	fn the_ten_kind_words_and_the_strict_set_match_the_gates_table() {
		let mut seen = [false; 10];
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
