//! Diagnostics for the evaluator path, replacing `lang::Refusals` at cut-over (U11). An evaluation
//! failure is recorded here with its span *and* returned as an `Err`, so an `Outcome` carries control flow
//! while the diagnostic list carries the position; `Engine::error` does both in one call.

use crate::syntax::{
	Source,
	Span,
};

use oxedyne_fe2o3_core::prelude::*;

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
	pub severity:	Severity,
	pub span:		Span,
	pub message:	String,
	pub hints:		Vec<String>,
	pub trace:		Vec<(Span, String)>,	// call sites outward, innermost first
}

impl Diagnostic {
	pub fn error<S: Into<String>>(span: Span, message: S) -> Self {
		Self { severity: Severity::Error, span, message: message.into(), hints: Vec::new(), trace: Vec::new() }
	}

	pub fn warning<S: Into<String>>(span: Span, message: S) -> Self {
		Self { severity: Severity::Warning, span, message: message.into(), hints: Vec::new(), trace: Vec::new() }
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
