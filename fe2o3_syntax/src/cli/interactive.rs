//! Whether the process has an interactive terminal to ask on, and a yes/no question that
//! honours the answer: no tty means no prompt, ever, rather than a question nobody can see.

use oxedyne_fe2o3_core::prelude::*;

use std::io::{BufRead, IsTerminal, Write};

/// A yes/no answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer { Yes, No }

/// Is stdin a terminal a person could type an answer into?
pub fn stdin_is_tty() -> bool { std::io::stdin().is_terminal() }

/// Is stdout a terminal worth printing a prompt or colour to?
pub fn stdout_is_tty() -> bool { std::io::stdout().is_terminal() }

/// Can the terminal actually be used interactively: a tty, and `$TERM` not `"dumb"`? This is
/// the same test git applies before trusting `$VISUAL`.
pub fn terminal_capable(tty: bool, term: Option<&str>) -> bool {
	tty && term.map_or(true, |t| t != "dumb")
}

/// Asks `question` on `output` with a `[Y/n]`/`[y/N]` hint for `default`, and reads one line of
/// an answer from `input`.
///
/// Returns `Ok(None)` without writing anything when `is_tty` is false: there is nobody to
/// answer, so the caller decides what "could not ask" means for it -- refuse outright, or take
/// the default silently. An empty line (Enter alone) or a closed input takes the default; an
/// unrecognised word re-prompts up to `retries` times before falling back to the default too, so
/// a script feeding nonsense on a claimed tty cannot hang the caller forever.
pub fn ask_yes_no<R, W>(
	question: &str,
	default:  Answer,
	is_tty:   bool,
	retries:  u8,
	input:    &mut R,
	output:   &mut W,
) -> Outcome<Option<Answer>>
where
	R: BufRead,
	W: Write,
{
	if !is_tty {
		return Ok(None);
	}
	let hint = match default {
		Answer::Yes => "[Y/n]",
		Answer::No  => "[y/N]",
	};
	for _ in 0..=retries {
		res!(write!(output, "{} {} ", question, hint), IO, Write);
		res!(output.flush(), IO, Write);
		let mut line = String::new();
		let n = res!(input.read_line(&mut line), IO, Read);
		if n == 0 {
			return Ok(Some(default)); // input closed; nothing more will ever arrive
		}
		let word = line.trim();
		if word.is_empty() {
			return Ok(Some(default));
		}
		match parse_yes_no(word) {
			Some(answer) => return Ok(Some(answer)),
			None => res!(writeln!(output, "Please answer y or n."), IO, Write),
		}
	}
	Ok(Some(default))
}

fn parse_yes_no(word: &str) -> Option<Answer> {
	match word.to_ascii_lowercase().as_str() {
		"y" | "yes" => Some(Answer::Yes),
		"n" | "no"  => Some(Answer::No),
		_           => None,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	use std::io::Cursor;

	#[test]
	fn no_tty_never_prompts_and_returns_none_00() -> Outcome<()> {
		let mut input  = Cursor::new(b"y\n".to_vec());
		let mut output = Vec::new();
		let answer = res!(ask_yes_no("Proceed?", Answer::No, false, 2, &mut input, &mut output));
		req!(answer.is_none(), true);
		req!(output.len(), 0); // nothing printed: nobody could have read it
		Ok(())
	}

	#[test]
	fn tty_reads_y_as_yes_00() -> Outcome<()> {
		let mut input  = Cursor::new(b"y\n".to_vec());
		let mut output = Vec::new();
		let answer = res!(ask_yes_no("Proceed?", Answer::No, true, 2, &mut input, &mut output));
		req!(answer, Some(Answer::Yes));
		Ok(())
	}

	#[test]
	fn tty_reads_full_words_case_insensitively_01() -> Outcome<()> {
		let mut input  = Cursor::new(b"NO\n".to_vec());
		let mut output = Vec::new();
		let answer = res!(ask_yes_no("Proceed?", Answer::Yes, true, 2, &mut input, &mut output));
		req!(answer, Some(Answer::No));
		Ok(())
	}

	#[test]
	fn tty_empty_line_takes_the_default_02() -> Outcome<()> {
		let mut input  = Cursor::new(b"\n".to_vec());
		let mut output = Vec::new();
		let answer = res!(ask_yes_no("Proceed?", Answer::No, true, 2, &mut input, &mut output));
		req!(answer, Some(Answer::No));
		let printed = String::from_utf8_lossy(&output);
		req!(printed.contains("[y/N]"), true);
		Ok(())
	}

	#[test]
	fn tty_closed_input_takes_the_default_03() -> Outcome<()> {
		let mut input  = Cursor::new(Vec::new()); // EOF straight away
		let mut output = Vec::new();
		let answer = res!(ask_yes_no("Proceed?", Answer::Yes, true, 2, &mut input, &mut output));
		req!(answer, Some(Answer::Yes));
		Ok(())
	}

	#[test]
	fn tty_reprompts_on_nonsense_then_reads_the_next_line_04() -> Outcome<()> {
		let mut input  = Cursor::new(b"maybe\nyes\n".to_vec());
		let mut output = Vec::new();
		let answer = res!(ask_yes_no("Proceed?", Answer::No, true, 2, &mut input, &mut output));
		req!(answer, Some(Answer::Yes));
		let printed = String::from_utf8_lossy(&output);
		req!(printed.contains("Please answer y or n."), true);
		Ok(())
	}

	#[test]
	fn tty_exhausted_retries_on_nonsense_takes_the_default_05() -> Outcome<()> {
		let mut input  = Cursor::new(b"maybe\nnope-not-that-either\nstill-no\n".to_vec());
		let mut output = Vec::new();
		let answer = res!(ask_yes_no("Proceed?", Answer::No, true, 2, &mut input, &mut output));
		req!(answer, Some(Answer::No));
		Ok(())
	}

	#[test]
	fn terminal_capable_rejects_dumb_and_no_tty_06() -> Outcome<()> {
		req!(terminal_capable(true, None), true);
		req!(terminal_capable(true, Some("xterm-256color")), true);
		req!(terminal_capable(true, Some("dumb")), false);
		req!(terminal_capable(false, Some("xterm-256color")), false);
		Ok(())
	}
}
