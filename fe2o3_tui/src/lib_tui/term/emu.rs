//! The emulator: the parser and the screen, joined.
//!
//! [`Terminal`] is the type a caller holds. Bytes go in through [`Terminal::feed`] and a screen
//! comes out through [`Terminal::screen`]. Everything else on the type exists because a terminal is
//! not only a screen: it answers some questions the application asks of it, it reports a window
//! title, it rings a bell, and it holds the mode flags that tell an input layer how to encode a key
//! press.

use crate::lib_tui::term::{
	cell::{
		NamedColour,
		Pen,
		TermColour,
		ATTR_BLINK,
		ATTR_BOLD,
		ATTR_DIM,
		ATTR_HIDDEN,
		ATTR_ITALIC,
		ATTR_REVERSE,
		ATTR_STRIKE,
		ATTR_UNDERLINE,
	},
	parse::{
		Act,
		Csi,
		Esc,
		Osc,
		Params,
		Parser,
		C0,
	},
	screen::{
		Damage,
		Erase,
		Screen,
		Surface,
	},
};

use oxedyne_fe2o3_core::prelude::*;


/// The mode flags an input layer needs in order to encode what the user does.
///
/// None of these change the grid, which is why they live here rather than on the screen. They are
/// recorded because an application that asks for them and does not get them behaves badly: a mouse
/// click sent in the wrong encoding lands in the wrong place, and a paste sent without its
/// brackets is executed line by line.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Modes {
	/// `DECCKM`, which asks for cursor keys as `SS3` rather than `CSI`.
	pub app_cursor: bool,
	/// `DECKPAM`, which asks for the keypad in application mode.
	pub app_keypad: bool,
	/// `?1000`, reporting button presses and releases.
	pub mouse_button: bool,
	/// `?1002`, reporting motion while a button is down.
	pub mouse_drag: bool,
	/// `?1003`, reporting all motion.
	pub mouse_any: bool,
	/// `?1006`, the `SGR` mouse encoding.
	pub mouse_sgr: bool,
	/// `?2004`, bracketed paste.
	pub bracketed_paste: bool,
	/// `?1004`, focus in and out reporting.
	pub focus_events: bool,
	/// `LNM`, which makes a line feed also return the carriage.
	pub newline: bool,
}

/// A terminal: a byte stream in, a screen out.
#[derive(Clone, Debug)]
pub struct Terminal {
	/// The byte stream state machine.
	parser:	Parser,
	/// The grid and the cursor.
	screen:	Screen,
	/// The buffer the parser writes into, reused between feeds.
	acts:	Vec<Act>,
	/// The mode flags.
	modes:	Modes,
	/// The window title the stream last asked for.
	title:	String,
	/// Whether the title changed since a caller last took it.
	title_dirty: bool,
	/// How many bells have rung since a caller last took them.
	bells:	usize,
	/// Bytes the application asked for, to be written back to the pseudoterminal.
	replies: Vec<u8>,
	/// The last character printed, which `REP` repeats.
	last_print: Option<char>,
}

impl Terminal {

	/// A terminal of the given size with the default scrollback bound.
	///
	/// # Errors
	/// Fails if either dimension is zero.
	pub fn new(cols: usize, rows: usize) -> Outcome<Self> {
		Ok(Self::from_screen(res!(Screen::new(cols, rows))))
	}

	/// A terminal of the given size, keeping at most `scrollback` lines.
	///
	/// # Errors
	/// Fails if either dimension is zero.
	pub fn with_scrollback(cols: usize, rows: usize, scrollback: usize) -> Outcome<Self> {
		Ok(Self::from_screen(res!(Screen::with_scrollback(cols, rows, scrollback))))
	}

	/// Wraps a screen that has already been made.
	fn from_screen(screen: Screen) -> Self {
		Self {
			parser:		Parser::new(),
			screen,
			acts:		Vec::new(),
			modes:		Modes::default(),
			title:		String::new(),
			title_dirty:	false,
			bells:		0,
			replies:	Vec::new(),
			last_print:	None,
		}
	}

	// ┌─────────────────────────────┐
	// │ INSPECTION                  │
	// └─────────────────────────────┘

	/// The screen.
	pub fn screen(&self) -> &Screen {
		&self.screen
	}

	/// The screen, for a caller that wants to drive it directly.
	pub fn screen_mut(&mut self) -> &mut Screen {
		&mut self.screen
	}

	/// What has changed since a renderer last looked.
	pub fn damage(&self) -> &Damage {
		self.screen.damage()
	}

	/// Declares the screen drawn, so that damage accumulates afresh.
	pub fn clear_damage(&mut self) {
		self.screen.clear_damage();
	}

	/// The mode flags.
	pub fn modes(&self) -> &Modes {
		&self.modes
	}

	/// The window title the stream last asked for.
	pub fn title(&self) -> &str {
		&self.title
	}

	/// The window title, if it has changed since this was last called.
	pub fn take_title(&mut self) -> Option<String> {
		if self.title_dirty {
			self.title_dirty = false;
			Some(self.title.clone())
		} else {
			None
		}
	}

	/// How many bells have rung since this was last called.
	pub fn take_bells(&mut self) -> usize {
		let n = self.bells;
		self.bells = 0;
		n
	}

	/// The bytes the application has asked to be sent back to it, which the caller must write to
	/// the pseudoterminal.
	///
	/// An application that asks where the cursor is and never hears back will wait for the answer,
	/// so a caller that never drains this will eventually hang something.
	pub fn take_replies(&mut self) -> Vec<u8> {
		std::mem::take(&mut self.replies)
	}

	/// Whether a character or sequence is half collected, waiting on more bytes.
	pub fn is_partial(&self) -> bool {
		self.parser.is_partial()
	}

	// ┌─────────────────────────────┐
	// │ FEEDING                     │
	// └─────────────────────────────┘

	/// Consumes a slice of the byte stream.
	///
	/// The slice may end anywhere, including inside a character or a control sequence; what is
	/// incomplete is held over until the next call.
	pub fn feed(&mut self, bytes: &[u8]) -> Outcome<()> {
		let mut acts = std::mem::take(&mut self.acts);
		acts.clear();
		self.parser.advance(bytes, &mut acts);
		for act in &acts {
			self.act(act);
		}
		acts.clear();
		self.acts = acts;
		Ok(())
	}

	/// Changes the size of the screen.
	///
	/// # Errors
	/// Fails if either dimension is zero.
	pub fn resize(&mut self, cols: usize, rows: usize) -> Outcome<()> {
		res!(self.screen.resize(cols, rows));
		Ok(())
	}

	/// Carries out one thing the parser decided the stream was asking for.
	fn act(&mut self, act: &Act) {
		match act {
			Act::Print(c)	=> {
				self.screen.print(*c);
				self.last_print = Some(*c);
			}
			Act::Ctrl(c)	=> self.ctrl(*c),
			Act::Csi(seq)	=> self.csi(seq),
			Act::Esc(seq)	=> self.esc(seq),
			Act::Osc(seq)	=> self.osc(seq),
		}
	}

	/// Carries out a C0 control.
	fn ctrl(&mut self, c: C0) {
		match c {
			C0::Bell		=> self.bells += 1,
			C0::Backspace		=> self.screen.backspace(),
			C0::Tab			=> self.screen.tab(),
			C0::LineFeed		=> {
				if self.modes.newline {
					self.screen.next_line();
				} else {
					self.screen.line_feed();
				}
			}
			C0::CarriageReturn	=> self.screen.carriage_return(),
		}
	}

	// ┌─────────────────────────────┐
	// │ ESCAPE SEQUENCES            │
	// └─────────────────────────────┘

	/// Carries out an escape sequence, ignoring any it does not recognise.
	fn esc(&mut self, seq: &Esc) {
		match (seq.inter, seq.fin) {
			// Character set designation, which a UTF-8 terminal has no use for.
			(Some(b'(' | b')' | b'*' | b'+' | b'-' | b'.' | b'/'), _)	=> {}
			// DECALN, the alignment pattern.
			(Some(b'#'), b'8')	=> self.screen.fill_alignment(),
			(Some(_), _)		=> {}
			(None, b'7')		=> self.screen.save_cursor(),
			(None, b'8')		=> self.screen.restore_cursor(),
			(None, b'D')		=> self.screen.line_feed(),
			(None, b'E')		=> self.screen.next_line(),
			(None, b'M')		=> self.screen.reverse_index(),
			(None, b'H')		=> self.screen.set_tab(),
			(None, b'c')		=> self.hard_reset(),
			(None, b'=')		=> self.modes.app_keypad = true,
			(None, b'>')		=> self.modes.app_keypad = false,
			// ST on its own, and everything else, is consumed and dropped.
			(None, _)		=> {}
		}
	}

	/// Carries out an operating system command, ignoring any it does not recognise.
	///
	/// Only the title commands mean anything to a screen model. `OSC 0` sets both the window and
	/// the icon title, `OSC 1` the icon title alone and `OSC 2` the window title alone; the icon
	/// title has no place to go here, so `OSC 1` is consumed and dropped.
	fn osc(&mut self, seq: &Osc) {
		match seq.ident {
			Some(0) | Some(2)	=> {
				self.title = seq.text.clone();
				self.title_dirty = true;
			}
			_	=> {}
		}
	}

	/// Returns the terminal to its power on state.
	fn hard_reset(&mut self) {
		self.screen.reset();
		self.modes = Modes::default();
		self.last_print = None;
	}

	/// Returns the terminal to a known state without clearing the grid, as `DECSTR` asks.
	///
	/// The cursor does not move. Several accounts of `DECSTR` say that it homes the cursor, but
	/// xterm and tmux both leave it where it was, and an application that sends a soft reset in the
	/// middle of drawing would be badly served by a terminal that moved it.
	fn soft_reset(&mut self) {
		let cur = *self.screen.cursor();
		self.screen.set_pen(Pen::plain());
		self.screen.set_origin(false);
		self.screen.set_autowrap(true);
		self.screen.set_insert(false);
		self.screen.set_cursor_visible(true);
		let rows = self.screen.rows();
		self.screen.set_region(0, rows - 1);
		// Setting the region has homed the cursor, which is what the saved position becomes.
		self.screen.save_cursor();
		self.screen.move_to(cur.col, cur.row);
		self.modes = Modes::default();
	}

	// ┌─────────────────────────────┐
	// │ CONTROL SEQUENCES           │
	// └─────────────────────────────┘

	/// Carries out a control sequence, ignoring any it does not recognise.
	fn csi(&mut self, seq: &Csi) {
		let p = &seq.params;
		match (seq.private, seq.inter, seq.fin) {
			// ── Private sequences ──────────────────────────────
			(Some(b'?'), None, b'h')	=> self.dec_modes(p, true),
			(Some(b'?'), None, b'l')	=> self.dec_modes(p, false),
			(Some(b'?'), None, b'n')	=> self.dec_report(p),
			(Some(b'?'), None, b'J')	=> self.erase_display(p),
			(Some(b'?'), None, b'K')	=> self.erase_line(p),
			(Some(_), _, _)			=> {}
			// ── Intermediates ──────────────────────────────────
			(None, Some(b'!'), b'p')	=> self.soft_reset(),
			(None, Some(_), _)		=> {}
			// ── Cursor movement ────────────────────────────────
			(None, None, b'A')	=> self.screen.move_up(Self::count(p)),
			(None, None, b'B') | (None, None, b'e')	=> self.screen.move_down(Self::count(p)),
			(None, None, b'C') | (None, None, b'a')	=> self.screen.move_right(Self::count(p)),
			(None, None, b'D')	=> self.screen.move_left(Self::count(p)),
			(None, None, b'E')	=> {
				self.screen.move_down(Self::count(p));
				self.screen.move_to_col(0);
			}
			(None, None, b'F')	=> {
				self.screen.move_up(Self::count(p));
				self.screen.move_to_col(0);
			}
			(None, None, b'G') | (None, None, b'`')	=> {
				self.screen.move_to_col(Self::index(p, 0));
			}
			(None, None, b'd')	=> self.screen.move_to_row(Self::index(p, 0)),
			(None, None, b'H') | (None, None, b'f')	=> {
				self.screen.move_to(Self::index(p, 1), Self::index(p, 0));
			}
			(None, None, b'I')	=> self.screen.tab_forward(Self::count(p)),
			(None, None, b'Z')	=> self.screen.tab_back(Self::count(p)),
			// ── Erasing ────────────────────────────────────────
			(None, None, b'J')	=> self.erase_display(p),
			(None, None, b'K')	=> self.erase_line(p),
			(None, None, b'X')	=> self.screen.erase_chars(Self::count(p)),
			// ── Insertion and deletion ─────────────────────────
			(None, None, b'@')	=> self.screen.insert_chars(Self::count(p)),
			(None, None, b'P')	=> self.screen.delete_chars(Self::count(p)),
			(None, None, b'L')	=> self.screen.insert_lines(Self::count(p)),
			(None, None, b'M')	=> self.screen.delete_lines(Self::count(p)),
			// ── Scrolling ──────────────────────────────────────
			(None, None, b'S')	=> self.screen.scroll_up(Self::count(p)),
			(None, None, b'T')	=> self.screen.scroll_down(Self::count(p)),
			(None, None, b'r')	=> {
				let rows = self.screen.rows();
				let top = Self::index(p, 0);
				let bottom = match p.get(1) {
					Some(0) | None	=> rows - 1,
					Some(v)		=> (v as usize).saturating_sub(1),
				};
				self.screen.set_region(top, bottom);
			}
			// ── Attributes ─────────────────────────────────────
			(None, None, b'm')	=> self.sgr(p),
			// ── Modes ──────────────────────────────────────────
			(None, None, b'h')	=> self.ansi_modes(p, true),
			(None, None, b'l')	=> self.ansi_modes(p, false),
			// ── Reports ────────────────────────────────────────
			(None, None, b'n')	=> self.report(p),
			(None, None, b'c')	=> self.reply(b"\x1b[?1;2c"),
			// ── Tab stops ──────────────────────────────────────
			(None, None, b'g')	=> match p.get_or(0, 0) {
				0	=> self.screen.clear_tab(),
				3	=> self.screen.clear_all_tabs(),
				_	=> {}
			},
			// ── Repetition ─────────────────────────────────────
			(None, None, b'b')	=> {
				if let Some(c) = self.last_print {
					for _ in 0..Self::count(p) {
						self.screen.print(c);
					}
				}
			}
			// ── Cursor save and restore ────────────────────────
			(None, None, b's')	=> self.screen.save_cursor(),
			(None, None, b'u')	=> self.screen.restore_cursor(),
			// Everything else is consumed and dropped.
			(None, None, _)		=> {}
		}
	}

	/// A repeat count parameter, which is one when absent or zero.
	fn count(p: &Params) -> usize {
		p.get_or(0, 1) as usize
	}

	/// A one based position parameter, converted to a zero based index.
	fn index(p: &Params, i: usize) -> usize {
		(p.get_or(i, 1) as usize).saturating_sub(1)
	}

	/// Carries out an erase of the display.
	fn erase_display(&mut self, p: &Params) {
		match p.get_or(0, 0) {
			3	=> self.screen.erase_scrollback(),
			v	=> {
				if let Some(what) = Erase::from_param(v) {
					self.screen.erase_display(what);
				}
			}
		}
	}

	/// Carries out an erase of the line.
	fn erase_line(&mut self, p: &Params) {
		if let Some(what) = Erase::from_param(p.get_or(0, 0)) {
			self.screen.erase_line(what);
		}
	}

	// ┌─────────────────────────────┐
	// │ MODES                       │
	// └─────────────────────────────┘

	/// Sets or clears the private DEC modes.
	fn dec_modes(&mut self, p: &Params, on: bool) {
		for i in 0..p.len().max(1) {
			match p.get_or(i, 0) {
				1	=> self.modes.app_cursor = on,
				6	=> self.screen.set_origin(on),
				7	=> self.screen.set_autowrap(on),
				25	=> self.screen.set_cursor_visible(on),
				47 | 1047	=> self.surface(on, false),
				1049	=> self.surface(on, true),
				1000	=> self.modes.mouse_button = on,
				1002	=> self.modes.mouse_drag = on,
				1003	=> self.modes.mouse_any = on,
				1004	=> self.modes.focus_events = on,
				1006	=> self.modes.mouse_sgr = on,
				2004	=> self.modes.bracketed_paste = on,
				_	=> {}
			}
		}
	}

	/// Brings the alternate or the ordinary grid forward.
	fn surface(&mut self, alt: bool, save: bool) {
		let want = if alt { Surface::Alternate } else { Surface::Primary };
		self.screen.set_surface(want, save);
	}

	/// Sets or clears the ANSI modes.
	fn ansi_modes(&mut self, p: &Params, on: bool) {
		for i in 0..p.len().max(1) {
			match p.get_or(i, 0) {
				4	=> self.screen.set_insert(on),
				20	=> self.modes.newline = on,
				_	=> {}
			}
		}
	}

	// ┌─────────────────────────────┐
	// │ REPORTS                     │
	// └─────────────────────────────┘

	/// Answers a device status report.
	fn report(&mut self, p: &Params) {
		match p.get_or(0, 0) {
			5	=> self.reply(b"\x1b[0n"),
			6	=> {
				let (col, row) = self.cursor_1based();
				let s = fmt!("\x1b[{};{}R", row, col);
				self.reply(s.as_bytes());
			}
			_	=> {}
		}
	}

	/// Answers a private device status report.
	fn dec_report(&mut self, p: &Params) {
		match p.get_or(0, 0) {
			6	=> {
				let (col, row) = self.cursor_1based();
				let s = fmt!("\x1b[?{};{}R", row, col);
				self.reply(s.as_bytes());
			}
			_	=> {}
		}
	}

	/// The cursor position as a report gives it, counting from one and never past the final column.
	fn cursor_1based(&self) -> (usize, usize) {
		let cur = self.screen.cursor();
		let col = cur.reported_col().min(self.screen.cols() - 1) + 1;
		(col, cur.row + 1)
	}

	/// Queues bytes to be written back to the pseudoterminal.
	fn reply(&mut self, bytes: &[u8]) {
		self.replies.extend_from_slice(bytes);
	}

	// ┌─────────────────────────────┐
	// │ SELECT GRAPHIC RENDITION    │
	// └─────────────────────────────┘

	/// Applies a select graphic rendition sequence to the pen.
	fn sgr(&mut self, p: &Params) {
		let mut pen = *self.screen.pen();
		if p.is_empty() {
			self.screen.set_pen(Pen::plain());
			return;
		}
		let mut i = 0;
		while i < p.len() {
			let v = match p.get(i) {
				Some(v)	=> v,
				None	=> break,
			};
			match v {
				0	=> pen = Pen::plain(),
				1	=> pen.attrs.set(ATTR_BOLD),
				2	=> pen.attrs.set(ATTR_DIM),
				3	=> pen.attrs.set(ATTR_ITALIC),
				4	=> {
					// The modern form `4:0` turns the underline off; every other subvalue
					// picks a style this model does not distinguish.
					if p.is_sub(i + 1) && p.get(i + 1) == Some(0) {
						pen.attrs.clear(ATTR_UNDERLINE);
					} else {
						pen.attrs.set(ATTR_UNDERLINE);
					}
					while p.is_sub(i + 1) {
						i += 1;
					}
				}
				5 | 6	=> pen.attrs.set(ATTR_BLINK),
				7	=> pen.attrs.set(ATTR_REVERSE),
				8	=> pen.attrs.set(ATTR_HIDDEN),
				9	=> pen.attrs.set(ATTR_STRIKE),
				21 | 22	=> pen.attrs.clear(ATTR_BOLD | ATTR_DIM),
				23	=> pen.attrs.clear(ATTR_ITALIC),
				24	=> pen.attrs.clear(ATTR_UNDERLINE),
				25	=> pen.attrs.clear(ATTR_BLINK),
				27	=> pen.attrs.clear(ATTR_REVERSE),
				28	=> pen.attrs.clear(ATTR_HIDDEN),
				29	=> pen.attrs.clear(ATTR_STRIKE),
				30..=37	=> pen.fore = Self::named(v - 30),
				38	=> {
					let (col, next) = Self::extended(p, i);
					if let Some(c) = col {
						pen.fore = c;
					}
					i = next;
				}
				39	=> pen.fore = TermColour::Default,
				40..=47	=> pen.back = Self::named(v - 40),
				48	=> {
					let (col, next) = Self::extended(p, i);
					if let Some(c) = col {
						pen.back = c;
					}
					i = next;
				}
				49	=> pen.back = TermColour::Default,
				90..=97		=> pen.fore = bright(Self::named(v - 90)),
				100..=107	=> pen.back = bright(Self::named(v - 100)),
				_	=> {}
			}
			i += 1;
		}
		self.screen.set_pen(pen);
	}

	/// The named colour for an offset of zero to seven.
	fn named(i: u32) -> TermColour {
		match NamedColour::from_index(i as u8) {
			Some(c)	=> TermColour::Named(c),
			None	=> TermColour::Default,
		}
	}

	/// Reads an extended colour selection, returning the colour and the index of its last
	/// parameter.
	///
	/// Both spellings are accepted. The original uses semicolons throughout, as in `38;5;196` and
	/// `38;2;10;20;30`. The later one uses colons and may carry a colour space identifier that
	/// nobody fills in, as in `38:2::10:20:30`; the empty field is what distinguishes the two colon
	/// forms from each other.
	fn extended(p: &Params, at: usize) -> (Option<TermColour>, usize) {
		// The length of the colon run that starts here, counting the selector itself.
		let mut run = 1;
		while p.is_sub(at + run) {
			run += 1;
		}
		let colon = run > 1;
		let kind = match p.get(at + 1) {
			Some(k)	=> k,
			None	=> return (None, at),
		};
		match kind {
			5	=> {
				let idx = match p.get(at + 2) {
					Some(v)	=> v,
					None	=> return (None, at + 1),
				};
				let end = if colon { at + run - 1 } else { at + 2 };
				(Some(TermColour::Indexed(idx as u8)), end)
			}
			2	=> {
				// With a colour space identifier the run is six long, without it five.
				let base = if colon && run >= 6 { at + 3 } else { at + 2 };
				let r = p.get(base);
				let g = p.get(base + 1);
				let b = p.get(base + 2);
				let end = if colon { at + run - 1 } else { base + 2 };
				match (r, g, b) {
					(Some(r), Some(g), Some(b))	=> (
						Some(TermColour::Rgb(r as u8, g as u8, b as u8)),
						end,
					),
					_	=> (None, end),
				}
			}
			_	=> (None, if colon { at + run - 1 } else { at + 1 }),
		}
	}
}

/// The bright variant of a named colour, or the colour unchanged if it is not a named one.
fn bright(c: TermColour) -> TermColour {
	match c {
		TermColour::Named(n)	=> TermColour::Named(n.brighten()),
		other			=> other,
	}
}
