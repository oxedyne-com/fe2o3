//! Tests for the terminal model.
//!
//! The cases marked as coming from the oracle were not invented here. Each byte sequence was
//! written to a file, that file was `cat`ed inside a detached `tmux` session of a known size, and
//! the resulting screen was read back with `capture-pane -p` and the cursor with
//! `display-message -p '#{cursor_x},#{cursor_y}'`. The expectations below are what tmux 3.6
//! produced. To regenerate one, run:
//!
//! ```text
//! printf '<the sequence>' > /tmp/seq
//! tmux -L t -f /dev/null new-session -d -x <cols> -y <rows> -s o 'cat /tmp/seq; sleep 30'
//! tmux -L t display-message -p -t o '#{cursor_x},#{cursor_y}'
//! tmux -L t capture-pane -p -t o
//! ```
//!
//! Two things tmux reports need translating. Its `cursor_x` is one past the final column when a
//! wrap is pending, which is `Cursor::reported_col` here. Its `capture-pane` re-encodes a run of
//! cells that a tab skipped over as a literal tab character, so the tab cases below compare the
//! cursor against tmux and the text against what the cells must hold.
//!
//! The same comparison was also run over recordings of real programmes rather than hand written
//! sequences. `script --log-out out.bin -c 'stty rows 24 cols 80; <programme>'` captures exactly
//! what a programme writes to a pseudoterminal; feeding that recording to this model and to tmux
//! and diffing the two screens covers far more of the grammar than any handwritten case can. Nine
//! recordings were checked in this way, among them 225 kB of `ls --color`, `top -b`, `man`,
//! `grep --color`, a carriage return progress bar, and `vim` and `less` stopped while their
//! alternate screens were still up. All nine agreed with tmux on every cell and on the cursor,
//! with the recording fed in chunks of varying size so that characters and sequences were cut at
//! every kind of boundary.

use oxedyne_fe2o3_tui::lib_tui::term::{
	cell::{
		runs,
		NamedColour,
		TermColour,
		Wide,
	},
	screen::Surface,
	width::{
		char_width,
		str_width,
		CharWidth,
	},
	Terminal,
};

use oxedyne_fe2o3_core::{
	prelude::*,
	test::test_it,
};


#[test]
fn main() -> Outcome<()> {

	log_set_level!("debug");

	let outcome = run_tests();

	log_finish_wait!();

	outcome
}

fn run_tests() -> Outcome<()> {

	let filter = "";

	res!(test_oracle(filter));
	res!(test_parser(filter));
	res!(test_screen(filter));
	res!(test_scrollback(filter));
	res!(test_resize(filter));
	res!(test_damage(filter));
	res!(test_style(filter));

	Ok(())
}

// ┌───────────────────────────────────────────────────────────────┐
// │ HELPERS                                                        │
// └───────────────────────────────────────────────────────────────┘

/// One case whose expectation came from a real terminal.
struct Case {
	/// What the case is called.
	name:	&'static str,
	/// Screen width.
	cols:	usize,
	/// Screen height.
	rows:	usize,
	/// The bytes fed to both terminals.
	byts:	&'static [u8],
	/// The rows tmux showed, with trailing blanks removed.
	lines:	&'static [&'static str],
	/// The cursor tmux reported, as column then row.
	cur:	(usize, usize),
}

/// Runs a case and compares the screen and the cursor with what the oracle gave.
fn run_case(case: &Case) -> Outcome<()> {
	let mut term = res!(Terminal::new(case.cols, case.rows));
	res!(term.feed(case.byts));
	res!(check_screen(case.name, &term, case.lines));
	res!(check_cursor(case.name, &term, case.cur));
	Ok(())
}

/// Compares every row of the screen with what was expected.
fn check_screen(name: &str, term: &Terminal, lines: &[&str]) -> Outcome<()> {
	let scr = term.screen();
	req!(scr.rows(), lines.len(), "({}: row count)", name);
	for r in 0..scr.rows() {
		let got = scr.row_text(r);
		if got != lines[r] {
			return Err(err!(
				"{}: row {} is {:?}, expected {:?}. Whole screen:\n{}",
				name, r, got, lines[r], scr.text();
				Test, Mismatch));
		}
	}
	Ok(())
}

/// Compares the cursor with what was expected, in the form tmux reports it.
fn check_cursor(name: &str, term: &Terminal, want: (usize, usize)) -> Outcome<()> {
	let cur = term.screen().cursor();
	let got = (cur.reported_col(), cur.row);
	if got != want {
		return Err(err!(
			"{}: cursor is {:?}, expected {:?}.", name, got, want;
			Test, Mismatch));
	}
	Ok(())
}

/// Turns an absent value into a test failure that names what was missing.
fn need<T>(v: Option<T>, what: &str) -> Outcome<T> {
	match v {
		Some(v)	=> Ok(v),
		None	=> Err(err!("{} was not there.", what; Test, Missing)),
	}
}

/// Builds a terminal and feeds it once.
fn fed(cols: usize, rows: usize, byts: &[u8]) -> Outcome<Terminal> {
	let mut term = res!(Terminal::new(cols, rows));
	res!(term.feed(byts));
	Ok(term)
}

// ┌───────────────────────────────────────────────────────────────┐
// │ THE ORACLE CASES                                               │
// └───────────────────────────────────────────────────────────────┘

/// Cases whose expectations were taken from tmux 3.6.
static CASES: &[Case] = &[
	// ── Cursor movement ────────────────────────────────────────
	Case {
		name:	"cursor absolute, back and down",
		cols:	20, rows: 6,
		byts:	b"\x1b[3;5Habc\x1b[3Dxy\x1b[2Bz",
		lines:	&["", "", "    xyc", "", "      z", ""],
		cur:	(7, 4),
	},
	Case {
		name:	"cursor home and clamped",
		cols:	20, rows: 6,
		byts:	b"ABCDE\x1b[Hz\x1b[10;99Hq",
		lines:	&["zBCDE", "", "", "", "", "                   q"],
		cur:	(20, 5),
	},
	// ── Erasing ────────────────────────────────────────────────
	Case {
		name:	"erase line to end",
		cols:	20, rows: 6,
		byts:	b"abcdefgh\x1b[1;4H\x1b[K",
		lines:	&["abc", "", "", "", "", ""],
		cur:	(3, 0),
	},
	Case {
		name:	"erase line to start",
		cols:	20, rows: 6,
		byts:	b"abcdefgh\x1b[1;4H\x1b[1K",
		lines:	&["    efgh", "", "", "", "", ""],
		cur:	(3, 0),
	},
	Case {
		name:	"erase whole line",
		cols:	20, rows: 6,
		byts:	b"abcdefgh\x1b[1;4H\x1b[2K",
		lines:	&["", "", "", "", "", ""],
		cur:	(3, 0),
	},
	Case {
		name:	"erase display to end",
		cols:	20, rows: 6,
		byts:	b"one\r\ntwo\r\nthree\x1b[2;2H\x1b[J",
		lines:	&["one", "t", "", "", "", ""],
		cur:	(1, 1),
	},
	Case {
		name:	"erase display to start",
		cols:	20, rows: 6,
		byts:	b"one\r\ntwo\r\nthree\x1b[2;2H\x1b[1J",
		lines:	&["", "  o", "three", "", "", ""],
		cur:	(1, 1),
	},
	Case {
		name:	"erase whole display",
		cols:	20, rows: 6,
		byts:	b"one\r\ntwo\r\nthree\x1b[2;2H\x1b[2J",
		lines:	&["", "", "", "", "", ""],
		cur:	(1, 1),
	},
	// ── Scrolling ──────────────────────────────────────────────
	Case {
		name:	"line feed at the foot scrolls",
		cols:	20, rows: 6,
		byts:	b"l1\r\nl2\r\nl3\r\nl4\r\nl5\r\nl6\r\nl7",
		lines:	&["l2", "l3", "l4", "l5", "l6", "l7"],
		cur:	(2, 5),
	},
	Case {
		name:	"scrolling region scrolls up",
		cols:	20, rows: 6,
		byts:	b"a\r\nb\r\nc\r\nd\r\ne\r\nf\x1b[2;4r\x1b[4;1HX\r\nY",
		lines:	&["a", "c", "X", "Y", "e", "f"],
		cur:	(1, 3),
	},
	Case {
		name:	"reverse index scrolls the region down",
		cols:	20, rows: 6,
		byts:	b"\x1b[2;4r\x1b[2;1HA\r\nB\r\nC\x1b[2;1H\x1bMZ",
		lines:	&["", "Z", "A", "B", "", ""],
		cur:	(1, 1),
	},
	Case {
		name:	"scroll up",
		cols:	20, rows: 6,
		byts:	b"a\r\nb\r\nc\r\nd\x1b[2S",
		lines:	&["c", "d", "", "", "", ""],
		cur:	(1, 3),
	},
	Case {
		name:	"scroll down",
		cols:	20, rows: 6,
		byts:	b"a\r\nb\r\nc\r\nd\x1b[1;1H\x1b[2T",
		lines:	&["", "", "a", "b", "c", "d"],
		cur:	(0, 0),
	},
	Case {
		name:	"setting the region homes the cursor",
		cols:	10, rows: 4,
		byts:	b"aaa\r\nbbb\r\nccc\x1b[2;3rX",
		lines:	&["Xaa", "bbb", "ccc", ""],
		cur:	(1, 0),
	},
	// ── Insertion and deletion ─────────────────────────────────
	Case {
		name:	"insert line",
		cols:	20, rows: 6,
		byts:	b"a\r\nb\r\nc\r\nd\x1b[2;1H\x1b[L",
		lines:	&["a", "", "b", "c", "d", ""],
		cur:	(0, 1),
	},
	Case {
		name:	"delete line",
		cols:	20, rows: 6,
		byts:	b"a\r\nb\r\nc\r\nd\x1b[2;1H\x1b[M",
		lines:	&["a", "c", "d", "", "", ""],
		cur:	(0, 1),
	},
	Case {
		name:	"insert characters",
		cols:	20, rows: 6,
		byts:	b"abcdef\x1b[1;3H\x1b[2@\x1b[1;1H\x1b[8G\x1b[1;1Hzz",
		lines:	&["zz  cdef", "", "", "", "", ""],
		cur:	(2, 0),
	},
	Case {
		name:	"delete characters",
		cols:	20, rows: 6,
		byts:	b"abcdef\x1b[1;3H\x1b[2P",
		lines:	&["abef", "", "", "", "", ""],
		cur:	(2, 0),
	},
	Case {
		name:	"erase characters",
		cols:	20, rows: 6,
		byts:	b"abcdef\x1b[1;3H\x1b[2X",
		lines:	&["ab  ef", "", "", "", "", ""],
		cur:	(2, 0),
	},
	Case {
		name:	"insert mode pushes the line right",
		cols:	10, rows: 4,
		byts:	b"abcdef\x1b[1;3H\x1b[4hXY",
		lines:	&["abXYcdef", "", "", ""],
		cur:	(4, 0),
	},
	// ── Wrapping and wide characters ───────────────────────────
	Case {
		name:	"narrow wrap",
		cols:	10, rows: 4,
		byts:	b"abcdefghijkl",
		lines:	&["abcdefghij", "kl", "", ""],
		cur:	(2, 1),
	},
	Case {
		name:	"wide characters mid line",
		cols:	10, rows: 4,
		byts:	"ab\u{4e2d}\u{6587}cd".as_bytes(),
		lines:	&["ab\u{4e2d}\u{6587}cd", "", "", ""],
		cur:	(8, 0),
	},
	Case {
		name:	"wide character with one cell left wraps whole",
		cols:	10, rows: 4,
		byts:	"abcdefghi\u{4e2d}\u{6587}".as_bytes(),
		lines:	&["abcdefghi", "\u{4e2d}\u{6587}", "", ""],
		cur:	(4, 1),
	},
	Case {
		name:	"wide character filling the final two cells",
		cols:	10, rows: 4,
		byts:	"abcdefgh\u{4e2d}Z".as_bytes(),
		lines:	&["abcdefgh\u{4e2d}", "Z", "", ""],
		cur:	(1, 1),
	},
	Case {
		name:	"writing over a wide character clears its other half",
		cols:	10, rows: 4,
		byts:	"ab\u{4e2d}\x08\x08Z".as_bytes(),
		lines:	&["abZ", "", "", ""],
		cur:	(3, 0),
	},
	Case {
		name:	"autowrap off pins the final column",
		cols:	10, rows: 4,
		byts:	b"\x1b[?7labcdefghijklmn",
		lines:	&["abcdefghin", "", "", ""],
		cur:	(9, 0),
	},
	// ── Sequences that must be swallowed ───────────────────────
	Case {
		name:	"unknown sequences are never printed",
		cols:	40, rows: 4,
		byts:	b"A\x1b[?2026hB\x1b[>4;2mC\x1b[1;2;3;4;5;6;7;8wD",
		lines:	&["ABCD", "", "", ""],
		cur:	(4, 0),
	},
	Case {
		name:	"an over long parameter list is dropped",
		cols:	10, rows: 4,
		byts:	b"A\x1b[1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;\
			1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;\
			1;1;1;1;1;1;1;1;1;1;1;3;3HB",
		lines:	&["AB", "", "", ""],
		cur:	(2, 0),
	},
	Case {
		name:	"an over large parameter is dropped",
		cols:	10, rows: 4,
		byts:	b"A\x1b[999999999999;1HB",
		lines:	&["AB", "", "", ""],
		cur:	(2, 0),
	},
	Case {
		name:	"operating system commands are swallowed",
		cols:	40, rows: 4,
		byts:	b"A\x1b]2;My Title\x1b\\B\x1b]0;Second\x07C",
		lines:	&["ABC", "", "", ""],
		cur:	(3, 0),
	},
	// ── The rest ───────────────────────────────────────────────
	Case {
		name:	"backward tabulation",
		cols:	40, rows: 4,
		byts:	b"A\x1b[?9999hB\x1b[99ZC\x1b[1!pD",
		lines:	&["CD", "", "", ""],
		cur:	(2, 0),
	},
	Case {
		name:	"save and restore the cursor",
		cols:	10, rows: 4,
		byts:	b"\x1b[2;3HA\x1b7\x1b[1;1HB\x1b8C",
		lines:	&["B", "  AC", "", ""],
		cur:	(4, 1),
	},
	Case {
		name:	"the alternate screen is left behind",
		cols:	10, rows: 4,
		byts:	b"main1\r\nmain2\x1b[?1049h\x1b[Halt\x1b[?1049l",
		lines:	&["main1", "main2", "", ""],
		cur:	(5, 1),
	},
	Case {
		name:	"the alternate screen starts blank",
		cols:	10, rows: 4,
		byts:	b"main1\r\nmain2\x1b[?1049h\x1b[Halt",
		lines:	&["alt", "", "", ""],
		cur:	(3, 0),
	},
	Case {
		name:	"repeat the last character",
		cols:	10, rows: 4,
		byts:	b"a\x1b[5b",
		lines:	&["aaaaaa", "", "", ""],
		cur:	(6, 0),
	},
	Case {
		name:	"the alignment pattern",
		cols:	10, rows: 4,
		byts:	b"\x1b#8",
		lines:	&["EEEEEEEEEE", "EEEEEEEEEE", "EEEEEEEEEE", "EEEEEEEEEE"],
		cur:	(0, 0),
	},
	Case {
		name:	"erase the scrollback leaves the screen",
		cols:	10, rows: 4,
		byts:	b"a\r\nb\r\nc\r\nd\r\ne\r\nf\x1b[3J",
		lines:	&["c", "d", "e", "f"],
		cur:	(1, 3),
	},
];

/// Every case whose expectation came from tmux.
fn test_oracle(filter: &'static str) -> Outcome<()> {

	res!(test_it(filter, &["Oracle cases", "all", "term", "oracle"], || {
		for case in CASES {
			res!(run_case(case));
		}
		test!("{} cases agreed with tmux.", CASES.len());
		Ok(())
	}));

	res!(test_it(filter, &["Oracle tab stops", "all", "term", "oracle", "tab"], || {
		// tmux re-encodes tab padding as a tab character in its capture, so only the cursor is
		// compared with it. The text is what the cells must hold given the stops it reported.
		let term = res!(fed(20, 6, b"a\tb\tc\td"));
		res!(check_cursor("default tab stops", &term, (20, 0)));
		req!(term.screen().row_text(0), fmt!("a       b       c  d"));

		let term = res!(fed(20, 6, b"\x1b[1;1H\x1b[3G\x1bH\x1b[1;1H\ty"));
		res!(check_cursor("a set tab stop", &term, (3, 0)));
		req!(term.screen().row_text(0), fmt!("  y"));
		Ok(())
	}));

	res!(test_it(filter, &["Oracle combining mark", "all", "term", "oracle", "combining"], || {
		// tmux reports a base and a combining mark as occupying one cell, and puts the cursor
		// after it. This model composes the pair where a composed character exists.
		let term = res!(fed(10, 4, "e\u{0301}x".as_bytes()));
		res!(check_cursor("combining mark", &term, (2, 0)));
		req!(term.screen().row_text(0), fmt!("\u{e9}x"));
		Ok(())
	}));

	res!(test_it(filter, &["Oracle scrollback", "all", "term", "oracle", "scrollback"], || {
		// tmux reported three lines of history for this flood, and none for the same flood on
		// the alternate screen.
		let term = res!(fed(10, 4, b"l1\r\nl2\r\nl3\r\nl4\r\nl5\r\nl6\r\nl7"));
		req!(term.screen().scrollback_len(), 3);
		req!(res!(need(term.screen().scrollback_text(0), "term.screen().scrollback_text(0)")), fmt!("l1"));
		req!(res!(need(term.screen().scrollback_text(2), "term.screen().scrollback_text(2)")), fmt!("l3"));

		let term = res!(fed(10, 4, b"\x1b[?1049hA\r\nB\r\nC\r\nD\r\nE\r\nF"));
		req!(term.screen().scrollback_len(), 0);
		res!(check_screen("alternate screen flood", &term, &["C", "D", "E", "F"]));

		// A region whose head is the head of the screen still fills the history.
		let term = res!(fed(10, 4, b"\x1b[1;3rl1\r\nl2\r\nl3\r\nl4"));
		req!(term.screen().scrollback_len(), 1);
		req!(res!(need(term.screen().scrollback_text(0), "term.screen().scrollback_text(0)")), fmt!("l1"));
		res!(check_screen("region at the head of the screen", &term, &["l2", "l3", "l4", ""]));
		res!(check_cursor("region at the head of the screen", &term, (2, 2)));

		// Erasing the display with a parameter of three empties the history.
		let term = res!(fed(10, 4, b"a\r\nb\r\nc\r\nd\r\ne\r\nf\x1b[3J"));
		req!(term.screen().scrollback_len(), 0);
		Ok(())
	}));

	res!(test_it(filter, &["Oracle cursor visibility", "all", "term", "oracle", "cursor"], || {
		let term = res!(fed(10, 4, b"\x1b[?25labc"));
		req!(term.screen().cursor().visible, false);
		res!(check_cursor("hidden cursor", &term, (3, 0)));
		let term = res!(fed(10, 4, b"\x1b[?25l\x1b[?25habc"));
		req!(term.screen().cursor().visible, true);
		Ok(())
	}));

	res!(test_it(filter, &["Oracle colon colour", "all", "term", "oracle", "sgr"], || {
		// tmux rewrote `38:2::10:20:30` as `38;2;10;20;30`, so it read the colon form with an
		// empty colour space identifier as a direct colour.
		let term = res!(fed(10, 4, b"\x1b[38:2::10:20:30mX"));
		let cell = res!(need(term.screen().cell(0, 0), "term.screen().cell(0, 0)"));
		req!(cell.pen.fore, TermColour::Rgb(10, 20, 30));

		let term = res!(fed(10, 4, b"\x1b[38;2;10;20;30mX"));
		let cell = res!(need(term.screen().cell(0, 0), "term.screen().cell(0, 0)"));
		req!(cell.pen.fore, TermColour::Rgb(10, 20, 30));

		let term = res!(fed(10, 4, b"\x1b[38:5:196mX"));
		let cell = res!(need(term.screen().cell(0, 0), "term.screen().cell(0, 0)"));
		req!(cell.pen.fore, TermColour::Indexed(196));
		Ok(())
	}));

	res!(test_it(filter, &["Oracle title", "all", "term", "oracle", "title"], || {
		let mut term = res!(fed(40, 4, b"A\x1b]2;My Title\x1b\\B\x1b]0;Second\x07C"));
		req!(res!(need(term.take_title(), "term.take_title()")), fmt!("Second"));
		req!(term.take_title(), Option::<String>::None);
		Ok(())
	}));

	Ok(())
}

// ┌───────────────────────────────────────────────────────────────┐
// │ THE PARSER                                                     │
// └───────────────────────────────────────────────────────────────┘

fn test_parser(filter: &'static str) -> Outcome<()> {

	res!(test_it(filter, &["Split character", "all", "term", "parse", "split"], || {
		// A three byte character cut after its first byte.
		let mut term = res!(Terminal::new(10, 2));
		res!(term.feed(b"ab\xe4"));
		req!(term.screen().row_text(0), fmt!("ab"));
		req!(term.is_partial(), true);
		res!(term.feed(b"\xb8\xadcd"));
		req!(term.screen().row_text(0), fmt!("ab\u{4e2d}cd"));
		req!(term.is_partial(), false);

		// Cut between every pair of bytes in turn.
		let whole = "a\u{4e2d}b\u{1f600}c".as_bytes();
		for at in 0..whole.len() {
			let mut term = res!(Terminal::new(20, 2));
			res!(term.feed(&whole[..at]));
			res!(term.feed(&whole[at..]));
			req!(term.screen().row_text(0), fmt!("a\u{4e2d}b\u{1f600}c"),
				"(split at byte {})", at);
		}
		Ok(())
	}));

	res!(test_it(filter, &["Malformed UTF-8", "all", "term", "parse", "utf8"], || {
		// A truncated character is one replacement, and the byte that truncated it still counts.
		let term = res!(fed(10, 2, b"a\xe4b"));
		req!(term.screen().row_text(0), fmt!("a\u{fffd}b"));
		// A stray continuation byte is one replacement.
		let term = res!(fed(10, 2, b"a\xb8b"));
		req!(term.screen().row_text(0), fmt!("a\u{fffd}b"));
		// An escape sequence arriving mid character is still obeyed.
		let term = res!(fed(10, 2, b"a\xe4\x1b[2Cb"));
		req!(term.screen().row_text(0), fmt!("a\u{fffd}  b"));
		Ok(())
	}));

	res!(test_it(filter, &["Split sequence", "all", "term", "parse", "split"], || {
		// A control sequence cut between every pair of bytes in turn.
		let whole = b"abc\x1b[2DX";
		for at in 0..whole.len() {
			let mut term = res!(Terminal::new(10, 2));
			res!(term.feed(&whole[..at]));
			res!(term.feed(&whole[at..]));
			req!(term.screen().row_text(0), fmt!("aXc"), "(split at byte {})", at);
		}
		// A sequence cut into single bytes.
		let mut term = res!(Terminal::new(10, 2));
		for b in b"\x1b[1;5H\x1b[31mZ" {
			res!(term.feed(&[*b]));
		}
		req!(term.screen().row_text(0), fmt!("    Z"));
		let cell = res!(need(term.screen().cell(4, 0), "term.screen().cell(4, 0)"));
		req!(cell.pen.fore, TermColour::Named(NamedColour::Red));
		Ok(())
	}));

	res!(test_it(filter, &["Runaway sequences", "all", "term", "parse", "runaway"], || {
		// An operating system command far longer than any real one. Its payload is dropped, the
		// terminator is still found, and what follows prints.
		let mut byts = Vec::new();
		byts.extend_from_slice(b"A\x1b]0;");
		for _ in 0..100_000 {
			byts.push(b'x');
		}
		byts.extend_from_slice(b"\x07B");
		let mut term = res!(Terminal::new(10, 2));
		res!(term.feed(&byts));
		req!(term.screen().row_text(0), fmt!("AB"));
		req!(term.take_title(), Option::<String>::None);

		// A control sequence with an absurd number of digits. Nothing is buffered and the
		// sequence is abandoned rather than acted on.
		let mut byts = Vec::new();
		byts.extend_from_slice(b"A\x1b[");
		for _ in 0..50_000 {
			byts.push(b'9');
		}
		byts.extend_from_slice(b"HB");
		let mut term = res!(Terminal::new(10, 2));
		res!(term.feed(&byts));
		req!(term.screen().row_text(0), fmt!("AB"));

		// An operating system command whose terminator never arrives recovers at the next line
		// break instead of swallowing the rest of the stream.
		let term = res!(fed(10, 3, b"A\x1b]0;no end here\nB"));
		req!(term.screen().row_text(0), fmt!("A"));
		req!(term.screen().row_text(1), fmt!(" B"));
		Ok(())
	}));

	res!(test_it(filter, &["Ignored sequences", "all", "term", "parse", "ignore"], || {
		// None of these may leave a mark on the screen.
		let noise: &[&[u8]] = &[
			b"\x1bP0;1|17/ab\x1b\\",		// A device control string.
			b"\x1b_G a=T,f=100 \x1b\\",		// An application programme command.
			b"\x1b^private\x1b\\",			// A privacy message.
			b"\x1bX start of string \x1b\\",	// A start of string.
			b"\x1b]52;c;aGVsbG8=\x07",		// A clipboard command.
			b"\x1b[?1000;1002;1006h",		// Mouse tracking.
			b"\x1b[>c",				// A secondary device attributes query.
			b"\x1b[3;4;5;6;7$p",			// A request with an intermediate.
			b"\x1b(B\x1b)0",			// Character set designation.
			b"\x1b[?12;25h\x1b[?12l",		// Cursor blink.
			b"\x1b[ q",				// A cursor style.
			b"\x1b[8;24;80t",			// A window operation.
		];
		for byts in noise {
			let mut term = res!(Terminal::new(20, 3));
			res!(term.feed(b"["));
			res!(term.feed(byts));
			res!(term.feed(b"]"));
			let got = term.screen().row_text(0);
			if got != "[]" {
				return Err(err!(
					"The sequence {:?} left {:?} on the screen.", byts, got;
					Test, Mismatch));
			}
		}
		Ok(())
	}));

	res!(test_it(filter, &["Reports", "all", "term", "parse", "report"], || {
		// The cursor position report counts from one.
		let mut term = res!(fed(20, 6, b"\x1b[3;5H\x1b[6n"));
		req!(term.take_replies(), b"\x1b[3;5R".to_vec());
		req!(term.take_replies(), Vec::<u8>::new());

		// The status report and the device attributes.
		let mut term = res!(fed(20, 6, b"\x1b[5n\x1b[c"));
		req!(term.take_replies(), b"\x1b[0n\x1b[?1;2c".to_vec());

		// A pending wrap is reported as the final column, not one past it.
		let mut term = res!(fed(10, 2, b"abcdefghij\x1b[6n"));
		req!(term.screen().cursor().reported_col(), 10);
		req!(term.take_replies(), b"\x1b[1;10R".to_vec());
		Ok(())
	}));

	res!(test_it(filter, &["Bells", "all", "term", "parse", "bell"], || {
		let mut term = res!(fed(10, 2, b"a\x07b\x07\x07c"));
		req!(term.screen().row_text(0), fmt!("abc"));
		req!(term.take_bells(), 3);
		req!(term.take_bells(), 0);
		Ok(())
	}));

	Ok(())
}

// ┌───────────────────────────────────────────────────────────────┐
// │ THE SCREEN                                                     │
// └───────────────────────────────────────────────────────────────┘

fn test_screen(filter: &'static str) -> Outcome<()> {

	res!(test_it(filter, &["Deferred wrap", "all", "term", "screen", "wrap"], || {
		// A line exactly as wide as the screen must not scroll until something follows it.
		let term = res!(fed(5, 3, b"abcde"));
		req!(term.screen().cursor().row, 0);
		req!(term.screen().cursor().wrap_pending, true);
		req!(term.screen().row_text(0), fmt!("abcde"));

		let term = res!(fed(5, 3, b"abcdef"));
		req!(term.screen().cursor().row, 1);
		req!(term.screen().row_text(1), fmt!("f"));

		// A carriage return cancels the pending wrap.
		let term = res!(fed(5, 3, b"abcde\rZ"));
		req!(term.screen().row_text(0), fmt!("Zbcde"));
		req!(term.screen().row_text(1), fmt!(""));

		// So does a backspace, which then leaves the cursor on the final column.
		let term = res!(fed(5, 3, b"abcde\x08Z"));
		req!(term.screen().row_text(0), fmt!("abcdZ"));
		Ok(())
	}));

	res!(test_it(filter, &["Bare line feed", "all", "term", "screen", "linefeed"], || {
		// Without newline mode a line feed keeps the column; a pseudoterminal usually turns
		// this into a carriage return and line feed before the model ever sees it, so the model
		// must not do it a second time.
		let term = res!(fed(10, 3, b"abc\ndef"));
		req!(term.screen().row_text(0), fmt!("abc"));
		req!(term.screen().row_text(1), fmt!("   def"));

		// With newline mode set it returns the carriage as well.
		let term = res!(fed(10, 3, b"\x1b[20habc\ndef"));
		req!(term.screen().row_text(1), fmt!("def"));
		Ok(())
	}));

	res!(test_it(filter, &["Origin mode", "all", "term", "screen", "origin"], || {
		// With origin mode set, row one is the head of the region and the cursor cannot leave it.
		let term = res!(fed(10, 6, b"\x1b[3;5r\x1b[?6h\x1b[1;1HX\x1b[9;1HY"));
		req!(term.screen().row_text(2), fmt!("X"));
		req!(term.screen().row_text(4), fmt!("Y"));
		req!(term.screen().row_text(0), fmt!(""));
		Ok(())
	}));

	res!(test_it(filter, &["Erase paints the background", "all", "term", "screen", "erase"], || {
		let term = res!(fed(10, 3, b"\x1b[41m\x1b[2J"));
		let cell = res!(need(term.screen().cell(3, 1), "term.screen().cell(3, 1)"));
		req!(cell.pen.back, TermColour::Named(NamedColour::Red));
		req!(cell.chr, ' ');
		// The foreground and the attributes are not carried into the erased cells.
		let term = res!(fed(10, 3, b"\x1b[1;31;42m\x1b[2J"));
		let cell = res!(need(term.screen().cell(3, 1), "term.screen().cell(3, 1)"));
		req!(cell.pen.back, TermColour::Named(NamedColour::Green));
		req!(cell.pen.fore, TermColour::Default);
		req!(cell.pen.attrs.bold(), false);
		Ok(())
	}));

	res!(test_it(filter, &["Wide character cells", "all", "term", "screen", "wide"], || {
		let term = res!(fed(10, 2, "a\u{4e2d}b".as_bytes()));
		req!(res!(need(term.screen().cell(0, 0), "term.screen().cell(0, 0)")).wide, Wide::No);
		req!(res!(need(term.screen().cell(1, 0), "term.screen().cell(1, 0)")).wide, Wide::Lead);
		req!(res!(need(term.screen().cell(1, 0), "term.screen().cell(1, 0)")).chr, '\u{4e2d}');
		req!(res!(need(term.screen().cell(2, 0), "term.screen().cell(2, 0)")).wide, Wide::Trail);
		req!(res!(need(term.screen().cell(3, 0), "term.screen().cell(3, 0)")).chr, 'b');

		// Writing over the left half clears the right half.
		let term = res!(fed(10, 2, "a\u{4e2d}b\x1b[1;2HZ".as_bytes()));
		req!(term.screen().row_text(0), fmt!("aZ b"));
		req!(res!(need(term.screen().cell(2, 0), "term.screen().cell(2, 0)")).wide, Wide::No);

		// Writing over the right half clears the left half.
		let term = res!(fed(10, 2, "a\u{4e2d}b\x1b[1;3HZ".as_bytes()));
		req!(term.screen().row_text(0), fmt!("a Zb"));
		req!(res!(need(term.screen().cell(1, 0), "term.screen().cell(1, 0)")).wide, Wide::No);
		Ok(())
	}));

	res!(test_it(filter, &["Character widths", "all", "term", "screen", "width"], || {
		req!(char_width('a'), CharWidth::Narrow);
		req!(char_width('\u{4e2d}'), CharWidth::Wide);
		req!(char_width('\u{ff21}'), CharWidth::Wide);
		req!(char_width('\u{1f600}'), CharWidth::Wide);
		req!(char_width('\u{0301}'), CharWidth::Zero);
		req!(char_width('\u{200b}'), CharWidth::Zero);
		req!(char_width('\u{fe0f}'), CharWidth::Zero);
		req!(char_width('\n'), CharWidth::Zero);
		req!(str_width("ab\u{4e2d}"), 4);
		Ok(())
	}));

	res!(test_it(filter, &["Cell size", "all", "term", "screen", "size"], || {
		// The argument for keeping combining marks out of the cell rests on this number. A
		// screen of two hundred columns with ten thousand lines of scrollback is two million
		// cells, so a cell that grew to hold a boxed string would cost tens of megabytes.
		let n = std::mem::size_of::<oxedyne_fe2o3_tui::lib_tui::term::Cell>();
		if n > 24 {
			return Err(err!("A cell has grown to {} bytes.", n; Test, Excessive));
		}
		test!("A cell is {} bytes.", n);
		Ok(())
	}));

	res!(test_it(filter, &["Alternate screen", "all", "term", "screen", "alt"], || {
		let term = res!(fed(10, 3, b"keep\x1b[?1049halt\x1b[?1049l"));
		req!(term.screen().surface(), Surface::Primary);
		req!(term.screen().row_text(0), fmt!("keep"));
		req!(term.screen().scrollback_len(), 0);

		// The alternate screen is blank when it comes forward a second time.
		let term = res!(fed(10, 3, b"\x1b[?1049halt\x1b[?1049l\x1b[?1049h"));
		req!(term.screen().surface(), Surface::Alternate);
		req!(term.screen().row_text(0), fmt!(""));

		// The older form switches without saving the cursor.
		let term = res!(fed(10, 3, b"abc\x1b[?47hX\x1b[?47l"));
		req!(term.screen().row_text(0), fmt!("abc"));
		Ok(())
	}));

	res!(test_it(filter, &["Reset", "all", "term", "screen", "reset"], || {
		let term = res!(fed(10, 3, b"\x1b[31mabc\x1b[2;4r\x1b[?7l\x1bcZ"));
		req!(term.screen().row_text(0), fmt!("Z"));
		req!(term.screen().region(), (0, 2));
		req!(term.screen().autowrap(), true);
		let cell = res!(need(term.screen().cell(0, 0), "term.screen().cell(0, 0)"));
		req!(cell.pen.fore, TermColour::Default);
		Ok(())
	}));

	Ok(())
}

// ┌───────────────────────────────────────────────────────────────┐
// │ SCROLLBACK                                                     │
// └───────────────────────────────────────────────────────────────┘

fn test_scrollback(filter: &'static str) -> Outcome<()> {

	res!(test_it(filter, &["Bounded scrollback", "all", "term", "scrollback", "bound"], || {
		// A thousand lines through a screen that keeps ten.
		let mut term = res!(Terminal::with_scrollback(20, 4, 10));
		for i in 1..=1000 {
			res!(term.feed(fmt!("l{}\r\n", i).as_bytes()));
		}
		req!(term.screen().scrollback_len(), 10);
		// The lines kept are the ten most recent to have left the screen.
		req!(res!(need(term.screen().scrollback_text(9), "term.screen().scrollback_text(9)")), fmt!("l997"));
		req!(res!(need(term.screen().scrollback_text(0), "term.screen().scrollback_text(0)")), fmt!("l988"));
		req!(term.screen().row_text(0), fmt!("l998"));
		req!(term.screen().row_text(2), fmt!("l1000"));
		Ok(())
	}));

	res!(test_it(filter, &["No scrollback", "all", "term", "scrollback", "none"], || {
		let mut term = res!(Terminal::with_scrollback(20, 4, 0));
		for i in 1..=100 {
			res!(term.feed(fmt!("l{}\r\n", i).as_bytes()));
		}
		req!(term.screen().scrollback_len(), 0);
		req!(term.screen().row_text(2), fmt!("l100"));
		// The count of lines that left is still reported, so a renderer holding its own history
		// knows what it missed.
		req!(term.damage().scrolled(), 97);
		Ok(())
	}));

	res!(test_it(filter, &["Scrollback trims blanks", "all", "term", "scrollback", "trim"], || {
		let mut term = res!(Terminal::with_scrollback(80, 2, 10));
		res!(term.feed(b"ab\r\ncd\r\nef"));
		let line = res!(need(term.screen().scrollback_line(0), "term.screen().scrollback_line(0)"));
		req!(line.len(), 2, "(a stored line keeps only what was written)");
		Ok(())
	}));

	res!(test_it(filter, &["Region scrollback", "all", "term", "scrollback", "region"], || {
		// A region that does not start at the head of the screen keeps nothing, because a line
		// that vanishes from the middle of the screen has no place in a history that is read
		// from the top down. tmux does store it; see the note in the module documentation.
		let term = res!(fed(10, 4, b"\x1b[2;4rl1\r\nl2\r\nl3\r\nl4\r\nl5"));
		req!(term.screen().scrollback_len(), 0);
		res!(check_screen("mid screen region", &term, &["l1", "l3", "l4", "l5"]));
		Ok(())
	}));

	Ok(())
}

// ┌───────────────────────────────────────────────────────────────┐
// │ RESIZE                                                         │
// └───────────────────────────────────────────────────────────────┘

fn test_resize(filter: &'static str) -> Outcome<()> {

	res!(test_it(filter, &["Narrower", "all", "term", "resize", "cols"], || {
		let mut term = res!(Terminal::new(10, 3));
		res!(term.feed(b"abcdefghij\r\nklm"));
		res!(term.resize(5, 3));
		req!(term.screen().cols(), 5);
		req!(term.screen().row_text(0), fmt!("abcde"));
		req!(term.screen().row_text(1), fmt!("klm"));
		Ok(())
	}));

	res!(test_it(filter, &["Wider", "all", "term", "resize", "cols"], || {
		let mut term = res!(Terminal::new(5, 3));
		res!(term.feed(b"abc"));
		res!(term.resize(10, 3));
		req!(term.screen().cols(), 10);
		req!(term.screen().row_text(0), fmt!("abc"));
		Ok(())
	}));

	res!(test_it(filter, &["Narrower cuts a wide character", "all", "term", "resize", "wide"], || {
		let mut term = res!(Terminal::new(10, 2));
		res!(term.feed("abcd\u{4e2d}".as_bytes()));
		res!(term.resize(5, 2));
		// The left half would be left without its right, so the whole character goes.
		req!(term.screen().row_text(0), fmt!("abcd"));
		Ok(())
	}));

	res!(test_it(filter, &["Shorter", "all", "term", "resize", "rows"], || {
		let mut term = res!(Terminal::new(10, 3));
		res!(term.feed(b"a\r\nb\r\nc"));
		req!(term.screen().cursor().row, 2);
		res!(term.resize(10, 2));
		// The cursor is on the last row, so the row lost is the topmost, and it is kept.
		req!(term.screen().row_text(0), fmt!("b"));
		req!(term.screen().row_text(1), fmt!("c"));
		req!(term.screen().cursor().row, 1);
		req!(term.screen().scrollback_len(), 1);
		req!(res!(need(term.screen().scrollback_text(0), "term.screen().scrollback_text(0)")), fmt!("a"));
		Ok(())
	}));

	res!(test_it(filter, &["Shorter below the cursor", "all", "term", "resize", "rows"], || {
		let mut term = res!(Terminal::new(10, 4));
		res!(term.feed(b"a\r\nb\r\nc\x1b[1;1H"));
		req!(term.screen().cursor().row, 0);
		res!(term.resize(10, 2));
		// There is room below the cursor, so nothing at the top is lost.
		req!(term.screen().row_text(0), fmt!("a"));
		req!(term.screen().row_text(1), fmt!("b"));
		req!(term.screen().scrollback_len(), 0);
		Ok(())
	}));

	res!(test_it(filter, &["Taller", "all", "term", "resize", "rows"], || {
		let mut term = res!(Terminal::new(10, 2));
		res!(term.feed(b"a\r\nb\r\nc"));
		req!(term.screen().scrollback_len(), 1);
		res!(term.resize(10, 3));
		// The line that had left comes back rather than a blank row appearing.
		req!(term.screen().row_text(0), fmt!("a"));
		req!(term.screen().row_text(1), fmt!("b"));
		req!(term.screen().row_text(2), fmt!("c"));
		req!(term.screen().scrollback_len(), 0);
		req!(term.screen().cursor().row, 2);
		Ok(())
	}));

	res!(test_it(filter, &["Taller with no history", "all", "term", "resize", "rows"], || {
		let mut term = res!(Terminal::new(10, 2));
		res!(term.feed(b"a\r\nb"));
		res!(term.resize(10, 4));
		req!(term.screen().row_text(0), fmt!("a"));
		req!(term.screen().row_text(1), fmt!("b"));
		req!(term.screen().row_text(3), fmt!(""));
		req!(term.screen().cursor().row, 1);
		Ok(())
	}));

	res!(test_it(filter, &["Resize resets the region", "all", "term", "resize", "region"], || {
		let mut term = res!(Terminal::new(10, 6));
		res!(term.feed(b"\x1b[2;4r"));
		req!(term.screen().region(), (1, 3));
		res!(term.resize(10, 4));
		req!(term.screen().region(), (0, 3));
		req!(term.damage().is_all(), true);
		Ok(())
	}));

	res!(test_it(filter, &["Resize refuses nothing", "all", "term", "resize", "zero"], || {
		let mut term = res!(Terminal::new(10, 4));
		req!(term.resize(0, 4).is_err(), true);
		req!(term.resize(10, 0).is_err(), true);
		req!(Terminal::new(0, 0).is_err(), true);
		Ok(())
	}));

	Ok(())
}

// ┌───────────────────────────────────────────────────────────────┐
// │ DAMAGE                                                         │
// └───────────────────────────────────────────────────────────────┘

fn test_damage(filter: &'static str) -> Outcome<()> {

	res!(test_it(filter, &["Rows", "all", "term", "damage", "rows"], || {
		let mut term = res!(Terminal::new(10, 4));
		term.clear_damage();
		req!(term.damage().any(), false);

		res!(term.feed(b"x"));
		req!(term.damage().dirty_rows(), vec![0]);
		req!(term.damage().cursor_moved(), true);
		term.clear_damage();

		res!(term.feed(b"\x1b[3;1Hy"));
		req!(term.damage().dirty_rows(), vec![2]);
		term.clear_damage();

		// Moving the cursor alone dirties no row.
		res!(term.feed(b"\x1b[1;1H"));
		req!(term.damage().dirty_rows(), Vec::<usize>::new());
		req!(term.damage().cursor_moved(), true);
		Ok(())
	}));

	res!(test_it(filter, &["Whole surface", "all", "term", "damage", "all"], || {
		let mut term = res!(Terminal::new(10, 4));
		term.clear_damage();
		res!(term.feed(b"\x1b[2J"));
		req!(term.damage().is_all(), true);
		req!(term.damage().dirty_rows(), vec![0, 1, 2, 3]);
		term.clear_damage();
		req!(term.damage().is_all(), false);

		// A scroll changes every row.
		res!(term.feed(b"a\r\nb\r\nc\r\nd\r\ne"));
		req!(term.damage().is_all(), true);
		req!(term.damage().scrolled(), 1);
		Ok(())
	}));

	res!(test_it(filter, &["A flood costs one repaint", "all", "term", "damage", "flood"], || {
		// Ten thousand lines between two draws leaves the renderer one screen to paint, not ten
		// thousand.
		let mut term = res!(Terminal::new(80, 24));
		term.clear_damage();
		for i in 0..10_000 {
			res!(term.feed(fmt!("line {}\r\n", i).as_bytes()));
		}
		req!(term.damage().dirty_rows().len(), 24);
		req!(term.damage().scrolled(), 9977);
		Ok(())
	}));

	Ok(())
}

// ┌───────────────────────────────────────────────────────────────┐
// │ STYLE                                                          │
// └───────────────────────────────────────────────────────────────┘

fn test_style(filter: &'static str) -> Outcome<()> {

	res!(test_it(filter, &["Attributes", "all", "term", "style", "attrs"], || {
		let term = res!(fed(20, 2, b"\x1b[1;2;3;4;5;7;9mX\x1b[0mY"));
		let cell = res!(need(term.screen().cell(0, 0), "term.screen().cell(0, 0)"));
		req!(cell.pen.attrs.bold(), true);
		req!(cell.pen.attrs.dim(), true);
		req!(cell.pen.attrs.italic(), true);
		req!(cell.pen.attrs.underline(), true);
		req!(cell.pen.attrs.blink(), true);
		req!(cell.pen.attrs.reverse(), true);
		req!(cell.pen.attrs.strike(), true);
		let cell = res!(need(term.screen().cell(1, 0), "term.screen().cell(1, 0)"));
		req!(cell.pen.attrs.is_empty(), true);

		// Each attribute has its own way off.
		let term = res!(fed(20, 2, b"\x1b[1;4;7m\x1b[22;24;27mX"));
		let cell = res!(need(term.screen().cell(0, 0), "term.screen().cell(0, 0)"));
		req!(cell.pen.attrs.is_empty(), true);

		// The modern underline spelling, on and off.
		let term = res!(fed(20, 2, b"\x1b[4:3mX\x1b[4:0mY"));
		req!(res!(need(term.screen().cell(0, 0), "term.screen().cell(0, 0)")).pen.attrs.underline(), true);
		req!(res!(need(term.screen().cell(1, 0), "term.screen().cell(1, 0)")).pen.attrs.underline(), false);
		Ok(())
	}));

	res!(test_it(filter, &["Colours", "all", "term", "style", "colour"], || {
		let term = res!(fed(20, 2, b"\x1b[31;42mA\x1b[91;102mB\x1b[39;49mC"));
		let a = res!(need(term.screen().cell(0, 0), "term.screen().cell(0, 0)"));
		req!(a.pen.fore, TermColour::Named(NamedColour::Red));
		req!(a.pen.back, TermColour::Named(NamedColour::Green));
		let b = res!(need(term.screen().cell(1, 0), "term.screen().cell(1, 0)"));
		req!(b.pen.fore, TermColour::Named(NamedColour::BrightRed));
		req!(b.pen.back, TermColour::Named(NamedColour::BrightGreen));
		let c = res!(need(term.screen().cell(2, 0), "term.screen().cell(2, 0)"));
		req!(c.pen.fore, TermColour::Default);
		req!(c.pen.back, TermColour::Default);

		// A palette entry and a direct colour, in one sequence with everything else.
		let term = res!(fed(20, 2, b"\x1b[1;38;5;196;48;2;1;2;3;4mZ"));
		let z = res!(need(term.screen().cell(0, 0), "term.screen().cell(0, 0)"));
		req!(z.pen.fore, TermColour::Indexed(196));
		req!(z.pen.back, TermColour::Rgb(1, 2, 3));
		req!(z.pen.attrs.bold(), true);
		req!(z.pen.attrs.underline(), true);

		// Reverse video is resolved for the renderer.
		let term = res!(fed(20, 2, b"\x1b[31;7mR"));
		let r = res!(need(term.screen().cell(0, 0), "term.screen().cell(0, 0)"));
		let (fore, back) = r.pen.resolved();
		req!(fore, TermColour::Default);
		req!(back, TermColour::Named(NamedColour::Red));
		Ok(())
	}));

	res!(test_it(filter, &["Runs", "all", "term", "style", "runs"], || {
		let term = res!(fed(20, 2, b"\x1b[31mab\x1b[0mcd"));
		let row = res!(need(term.screen().row(0), "term.screen().row(0)"));
		let list = res!(runs(row));
		req!(list.len(), 2);
		req!(list[0].col, 0);
		req!(list[0].text, fmt!("ab"));
		req!(list[0].pen.fore, TermColour::Named(NamedColour::Red));
		req!(list[1].col, 2);
		req!(list[1].text, fmt!("cd"));
		req!(list[1].pen.fore, TermColour::Default);

		// A wide character counts two cells but one character.
		let term = res!(fed(20, 2, "a\u{4e2d}b".as_bytes()));
		let row = res!(need(term.screen().row(0), "term.screen().row(0)"));
		let list = res!(runs(row));
		req!(list.len(), 1);
		req!(list[0].text, fmt!("a\u{4e2d}b"));
		req!(list[0].cells, 4);

		// A blank row in the default pen has nothing to draw.
		let term = res!(fed(20, 2, b""));
		let row = res!(need(term.screen().row(0), "term.screen().row(0)"));
		req!(res!(runs(row)).len(), 0);
		Ok(())
	}));

	res!(test_it(filter, &["Modes", "all", "term", "style", "modes"], || {
		let term = res!(fed(20, 2, b"\x1b[?1h\x1b[?1000h\x1b[?1006h\x1b[?2004h\x1b="));
		req!(term.modes().app_cursor, true);
		req!(term.modes().mouse_button, true);
		req!(term.modes().mouse_sgr, true);
		req!(term.modes().bracketed_paste, true);
		req!(term.modes().app_keypad, true);

		let term = res!(fed(20, 2, b"\x1b[?1h\x1b[?1l"));
		req!(term.modes().app_cursor, false);
		Ok(())
	}));

	Ok(())
}
