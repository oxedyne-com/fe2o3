//! A POSIX shell command line read into words and simple commands, without running anything.
//!
//! Only the quoting and the operators are read: a parameter, a command substitution, a glob or a
//! leading `~` is left as written and the word is marked as not literal, since the shell would
//! hand the program something else.  A caller that needs the word as the program would see it
//! treats a non-literal word as unknown.

use oxedyne_fe2o3_core::prelude::*;


/// A word of a command line, its quoting removed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Word {
	pub text:		String,
	pub literal:	bool, // no `$`, backquote, glob or leading `~` the shell would expand
}

impl Word {

	/// A word the shell hands over as written, such as an element of an argv.
	pub fn literal(text: &str) -> Self {
		Self { text: text.to_string(), literal: true }
	}
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Token {
	Word(Word),
	Sep,		// ends a simple command: `;`, `&`, `|`, a newline, a bracket or a comment
	ReadFrom,	// `<`: the next word is a file the command reads
	Redirect,	// any other redirection: the next word is not an operand
}

/// The tokens of a command line, quoted the way a POSIX shell quotes.
pub fn tokens(line: &str) -> Vec<Token> {
	let mut toks = Vec::new();
	let mut cur = String::new();
	let mut lit = true;
	let mut quoted = false;
	let mut cs = line.chars().peekable();
	let flush = |toks: &mut Vec<Token>, cur: &mut String, lit: &mut bool, quoted: &mut bool| {
		if *quoted || !cur.is_empty() {
			toks.push(Token::Word(Word { text: std::mem::take(cur), literal: *lit }));
		}
		*lit = true;
		*quoted = false;
	};
	while let Some(c) = cs.next() {
		match c {
			' ' | '\t' => flush(&mut toks, &mut cur, &mut lit, &mut quoted),
			'\n' | ';' | '|' | '(' | ')' => {
				flush(&mut toks, &mut cur, &mut lit, &mut quoted);
				toks.push(Token::Sep);
			}
			'&' if cs.peek() == Some(&'>') => {
				flush(&mut toks, &mut cur, &mut lit, &mut quoted);
				cs.next();
				if cs.peek() == Some(&'>') { cs.next(); }
				toks.push(Token::Redirect);
			}
			'&' => {
				flush(&mut toks, &mut cur, &mut lit, &mut quoted);
				toks.push(Token::Sep);
			}
			'<' | '>' => {
				// A file descriptor number before the operator belongs to it.
				if !quoted && !cur.is_empty() && cur.chars().all(|d| d.is_ascii_digit()) {
					cur.clear();
				}
				flush(&mut toks, &mut cur, &mut lit, &mut quoted);
				let mut plain = true;
				while let Some(&n) = cs.peek() {
					if n == c || n == '&' || (c == '>' && n == '|') {
						plain = false;
						cs.next();
					} else {
						break;
					}
				}
				// `<<`, `<<<` and `<&` take a delimiter, a string or a descriptor, not a file.
				toks.push(if c == '<' && plain { Token::ReadFrom } else { Token::Redirect });
			}
			'\'' => {
				quoted = true;
				for n in cs.by_ref() {
					if n == '\'' { break; }
					cur.push(n);
				}
			}
			'"' => {
				quoted = true;
				while let Some(n) = cs.next() {
					match n {
						'"'			=> break,
						'\\'		=> match cs.peek() {
							Some(&e) if matches!(e, '$' | '`' | '"' | '\\' | '\n') => {
								cs.next();
								cur.push(e);
							}
							_ => cur.push('\\'),
						},
						'$' | '`'	=> { lit = false; cur.push(n); }
						_			=> cur.push(n),
					}
				}
			}
			'\\' => {
				quoted = true;
				if let Some(n) = cs.next() {
					if n != '\n' { cur.push(n); }
				}
			}
			'#' if cur.is_empty() && !quoted => {
				for n in cs.by_ref() {
					if n == '\n' { break; }
				}
				toks.push(Token::Sep);
			}
			'~' if cur.is_empty() && !quoted	=> { lit = false; cur.push(c); }
			'$' | '`' | '*' | '?' | '['			=> { lit = false; cur.push(c); }
			_									=> cur.push(c),
		}
	}
	flush(&mut toks, &mut cur, &mut lit, &mut quoted);
	toks
}

/// A simple command: its words, and the files its `<` redirections read.  Other redirections
/// and their targets are dropped, as they are not operands.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Command {
	pub words:		Vec<Word>,
	pub read_from:	Vec<Word>,
}

/// The simple commands of a command line, in order.
pub fn commands(line: &str) -> Vec<Command> {
	let mut cmds = Vec::new();
	let mut cmd = Command::default();
	let mut toks = tokens(line).into_iter();
	while let Some(t) = toks.next() {
		match t {
			Token::Word(w)	=> cmd.words.push(w),
			Token::ReadFrom	=> if let Some(Token::Word(w)) = toks.next() { cmd.read_from.push(w) },
			Token::Redirect	=> { toks.next(); }
			Token::Sep		=> if !cmd.words.is_empty() || !cmd.read_from.is_empty() {
				cmds.push(std::mem::take(&mut cmd));
			},
		}
	}
	if !cmd.words.is_empty() || !cmd.read_from.is_empty() {
		cmds.push(cmd);
	}
	cmds
}

impl Command {

	pub fn of_argv(argv: &[String]) -> Self {
		Self { words: argv.iter().map(|a| Word::literal(a)).collect(), read_from: Vec::new() }
	}

	/// The program and its arguments, past any leading `NAME=value` assignments.
	pub fn args(&self) -> &[Word] {
		let n = self.words.iter()
			.take_while(|w| w.literal && w.text.split_once('=').map_or(false, |(k, _)|
				!k.is_empty() && k.chars().all(|c| c == '_' || c.is_ascii_alphanumeric())))
			.count();
		&self.words[n..]
	}

	/// The program's name without its folder, when the shell would run it as written.
	pub fn program(&self) -> Option<&str> {
		match self.args().first() {
			Some(w) if w.literal	=> w.text.rsplit('/').next(),
			_						=> None,
		}
	}

	/// Is the program a POSIX shell?
	pub fn is_shell(&self) -> bool {
		matches!(self.program(), Some("sh" | "bash" | "dash" | "zsh" | "ksh"))
	}

	/// The script a shell is given with `-c`, the first operand after its options.
	pub fn script(&self) -> Option<&Word> {
		if !self.is_shell() {
			return None;
		}
		let args = match self.args().split_first() {
			Some((_, rest))	=> rest,
			None			=> return None,
		};
		let c = args.iter().take_while(|w| w.text.starts_with('-'))
			.any(|w| !w.text.starts_with("--") && w.text.contains('c'));
		match c {
			true	=> args.iter().find(|w| !w.text.starts_with('-')),
			false	=> None,
		}
	}

	/// Is this a `cd`?  If so, `dir` moves to the folder it leaves the shell in, by [`join`],
	/// and to `None` when that cannot be known from the line.
	pub fn cd(&self, dir: &mut Option<String>) -> bool {
		if self.program() != Some("cd") {
			return false;
		}
		*dir = match self.args().get(1) {
			Some(w) if w.literal && w.text != "-"	=> join(dir, &w.text),
			_										=> None,
		};
		true
	}
}

/// Where `path` lands from the folder `dir`: an absolute path stands, `Some("")` is the folder
/// the line started in so a relative path stands as written, and `None` is a folder no longer
/// known, from which nothing relative can be placed.
pub fn join(dir: &Option<String>, path: &str) -> Option<String> {
	if path.starts_with('/') {
		return Some(path.to_string());
	}
	match dir {
		Some(d) if d.is_empty()	=> Some(path.to_string()),
		Some(d)					=> Some(fmt!("{}/{}", d.trim_end_matches('/'), path)),
		None					=> None,
	}
}

/// A word a POSIX shell reads back as exactly `s`.
pub fn quote(s: &str) -> String {
	if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "_./,:@%+=-".contains(c)) {
		return s.to_string();
	}
	fmt!("'{}'", s.replace('\'', "'\\''"))
}
