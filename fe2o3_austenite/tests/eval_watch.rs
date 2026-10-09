//! `austenite --eval --watch` rebuilds when a file the evaluation read changes, and for no other cause.
//!
//! The watched set is the evaluator's own record of what it asked the vfs for ([`Evaluated::files_read`]),
//! so these cases hold it to the files a synthetic project reads: its main, an import, an include, an image,
//! a data file, a package's files and the fonts under `--font-path`, and to nothing beside them. The
//! rebuild cases run the built binary over such a project, edit a file, and count the status lines it
//! prints: one for the edit of a file it read, none for the edit of one it did not.

use oxedyne_fe2o3_austenite::compile;
use oxedyne_fe2o3_austenite::emit::sinks::PdfSink;
use oxedyne_fe2o3_austenite::eval::package::{
	self,
	PackageSpec,
};
use oxedyne_fe2o3_austenite::flow::text::FontStore;

use std::io::BufRead;
use std::path::{
	Path,
	PathBuf,
};
use std::process::{
	Child,
	Command,
	Stdio,
};
use std::sync::mpsc::{
	self,
	Receiver,
};
use std::time::Duration;

const SVG: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"10\" height=\"10\"><rect width=\"10\" height=\"10\" fill=\"red\"/></svg>";

// A project beneath the target's temporary directory, written fresh for each case: a main that imports a
// library, includes a part, shows an image and reads a data file, an unrelated file beside them, and a font
// directory holding a file no parser accepts (the watch keys on what was found, not on what parsed).
fn project(name: &str) -> PathBuf {
	let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("eval_watch").join(name);
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(dir.join("fonts")).expect("the project's directories");
	let dir = std::fs::canonicalize(&dir).expect("the project's canonical path");
	write(&dir.join("main.typ"), concat!(
		"#import \"lib.typ\": word\n",
		"#include \"part.typ\"\n",
		"#let d = json(\"data.json\")\n",
		"#image(\"pic.svg\", width: 1cm)\n",
		"#word #d.n\n",
	));
	write(&dir.join("lib.typ"), "#let word = [Imported.]\n");
	write(&dir.join("part.typ"), "Included.\n");
	write(&dir.join("data.json"), "{\"n\": 1}\n");
	write(&dir.join("pic.svg"), SVG);
	write(&dir.join("unrelated.typ"), "Nothing reads this.\n");
	write(&dir.join("fonts/fake.ttf"), "not a font");
	dir
}

fn write(path: &Path, text: &str) {
	std::fs::write(path, text).expect("a project file");
}

// A package at `<dir>/local/<name>/0.1.0`, whose entrypoint `lib.typ` defines `greet`.
fn package_dir(dir: &Path, name: &str) -> PathBuf {
	let pkg = dir.join("packages");
	let own = pkg.join("local").join(name).join("0.1.0");
	std::fs::create_dir_all(&own).expect("the package's directory");
	write(&own.join("typst.toml"), &format!("[package]\nname = \"{}\"\nversion = \"0.1.0\"\nentrypoint = \"lib.typ\"\n", name));
	write(&own.join("lib.typ"), "#let greet = [Packaged.]\n");
	pkg
}

// What the evaluator read compiling `main` under `root`, with each of `fonts` a font directory.
fn read(main: &Path, root: &Path, fonts: &[&Path]) -> Vec<PathBuf> {
	let mut store = FontStore::default();
	for f in fonts {
		store.add_dir(f.to_path_buf());
	}
	let mut sink = PdfSink::new().expect("a PDF sink");
	let done = compile::assemble_eval(main, root, store, &mut sink).expect("the main is readable");
	done.files_read()
}

#[test]
fn the_watch_set_is_the_files_the_evaluation_read() {
	let dir = project("set");
	let pkgs = package_dir(&dir, "wdemo");
	package::add_dir(pkgs.clone()).expect("the package directory is added");
	write(&dir.join("main.typ"), concat!(
		"#import \"lib.typ\": word\n",
		"#import \"@local/wdemo:0.1.0\": greet\n",
		"#include \"part.typ\"\n",
		"#let d = json(\"data.json\")\n",
		"#image(\"pic.svg\", width: 1cm)\n",
		"#word #greet #d.n\n",
	));
	let found = read(&dir.join("main.typ"), &dir, &[&dir.join("fonts")]);
	let own = pkgs.join("local/wdemo/0.1.0");
	let mut want = vec![
		dir.join("main.typ"),
		dir.join("lib.typ"),
		dir.join("part.typ"),
		dir.join("data.json"),
		dir.join("pic.svg"),
		own.join("typst.toml"),
		own.join("lib.typ"),
		dir.join("fonts"),
		dir.join("fonts/fake.ttf"),
	];
	want.sort();
	assert_eq!(found, want, "the files read, the package's on disc beside the project's, and nothing else");
	assert!(!found.contains(&dir.join("unrelated.typ")), "a file nothing reads is not watched");
}

#[test]
fn a_file_asked_for_and_not_there_is_watched_so_that_its_arrival_is_seen() {
	let dir = project("missing");
	write(&dir.join("main.typ"), "#import \"later.typ\": x\n#x\n");
	let found = read(&dir.join("main.typ"), &dir, &[]);
	assert!(found.contains(&dir.join("main.typ")), "the main is watched: {:?}", found);
	assert!(found.contains(&dir.join("later.typ")), "the import that failed is watched: {:?}", found);
}

#[test]
fn a_package_supplied_in_memory_has_no_file_to_watch() {
	let dir = project("memory");
	let spec = PackageSpec { namespace: "local".to_string(), name: "memdemo".to_string(), version: (0, 1, 0) };
	package::supply(&spec, vec![
		("typst.toml".to_string(), b"[package]\nname = \"memdemo\"\nversion = \"0.1.0\"\nentrypoint = \"lib.typ\"\n".to_vec()),
		("lib.typ".to_string(), b"#let greet = [Held.]\n".to_vec()),
	]).expect("the package is supplied");
	write(&dir.join("main.typ"), "#import \"@local/memdemo:0.1.0\": greet\n#greet\n");
	let found = read(&dir.join("main.typ"), &dir, &[]);
	assert_eq!(found, vec![dir.join("main.typ")], "only the main stands on disc");
	let _ = package::withdraw(&spec);
}

// A running `austenite --eval --watch`, its standard output and error read line by line on channels. It is
// killed when it goes out of scope, whatever the case did.
struct Watch {
	child:	Child,
	out:	Receiver<String>,
	err:	Receiver<String>,
}

impl Watch {
	fn start(dir: &Path, extra: &[&str], envs: &[(&str, &Path)]) -> Self {
		Self::run(dir, extra, "main.typ", envs)
	}

	fn run(dir: &Path, extra: &[&str], main: &str, envs: &[(&str, &Path)]) -> Self {
		// The project was written an instant ago. A file saved within a tick of a build's start is built
		// again, once, since its stamp cannot be told from one made during the build; no real start is so
		// close behind a save, so the case waits it out and the quiet it asserts is the watch's own.
		std::thread::sleep(Duration::from_millis(150));
		let mut cmd = Command::new(env!("CARGO_BIN_EXE_austenite"));
		cmd.current_dir(dir)
			.args(["--eval", "--watch"])
			.args(extra)
			.args([main, "out"])
			.stdout(Stdio::piped())
			.stderr(Stdio::piped());
		for (k, v) in envs {
			cmd.env(k, v);
		}
		let mut child = cmd.spawn().expect("the built austenite binary");
		let out = Self::lines(child.stdout.take().expect("a piped standard output"));
		let err = Self::lines(child.stderr.take().expect("a piped standard error"));
		Self { child, out, err }
	}

	fn lines<R: std::io::Read + Send + 'static>(from: R) -> Receiver<String> {
		let (tx, rx) = mpsc::channel();
		std::thread::spawn(move || {
			for line in std::io::BufReader::new(from).lines().map_while(Result::ok) {
				if tx.send(line).is_err() {
					return;
				}
			}
		});
		rx
	}

	// The next status line of a rebuild that worked, within `wait`.
	fn status(&self, wait: Duration) -> Option<String> {
		let end = std::time::Instant::now() + wait;
		loop {
			let left = end.checked_duration_since(std::time::Instant::now())?;
			match self.out.recv_timeout(left) {
				Ok(l) if l.contains(" page(s), ")	=> return Some(l),
				Ok(_)								=> continue,
				Err(_)								=> return None,
			}
		}
	}

	// Is there no status line for `wait`?
	fn quiet(&self, wait: Duration) -> bool {
		self.status(wait).is_none()
	}

	// The next line of standard error that holds `needle`, within `wait`.
	fn complaint(&self, needle: &str, wait: Duration) -> Option<String> {
		let end = std::time::Instant::now() + wait;
		loop {
			let left = end.checked_duration_since(std::time::Instant::now())?;
			match self.err.recv_timeout(left) {
				Ok(l) if l.contains(needle)	=> return Some(l),
				Ok(_)						=> continue,
				Err(_)						=> return None,
			}
		}
	}
}

impl Drop for Watch {
	fn drop(&mut self) {
		let _ = self.child.kill();
		let _ = self.child.wait();
	}
}

const FIRST:	Duration = Duration::from_secs(40);	// the first build
const REBUILD:	Duration = Duration::from_secs(30);
const SETTLE:	Duration = Duration::from_millis(2500);	// six polls

fn pdf(dir: &Path) -> Vec<u8> {
	std::fs::read(dir.join("out/document.pdf")).expect("the PDF the watch wrote")
}

#[test]
fn a_change_to_an_imported_file_rebuilds_once() {
	let dir = project("import");
	let w = Watch::start(&dir, &[], &[]);
	assert!(w.status(FIRST).is_some(), "the first build prints a status line");
	assert!(w.quiet(SETTLE), "nothing rebuilds while nothing changes");
	write(&dir.join("lib.typ"), "#let word = [Imported again, and longer.]\n");
	assert!(w.status(REBUILD).is_some(), "an edit of the imported file rebuilds");
	assert!(w.quiet(SETTLE), "and rebuilds once");
}

#[test]
fn a_change_to_a_file_nothing_reads_rebuilds_nothing() {
	let dir = project("unrelated");
	let w = Watch::start(&dir, &[], &[]);
	assert!(w.status(FIRST).is_some(), "the first build prints a status line");
	write(&dir.join("unrelated.typ"), "Still read by nothing.\n");
	write(&dir.join("notes.txt"), "a new file beside the main\n");
	assert!(w.quiet(SETTLE), "an edit of a file the evaluation did not read does not rebuild");
	write(&dir.join("part.typ"), "Included, and edited.\n");
	assert!(w.status(REBUILD).is_some(), "an edit of an included file still does");
}

#[test]
fn a_change_to_an_image_or_a_data_file_rebuilds() {
	let dir = project("assets");
	let w = Watch::start(&dir, &[], &[]);
	assert!(w.status(FIRST).is_some(), "the first build prints a status line");
	write(&dir.join("pic.svg"), &SVG.replace("red", "blue"));
	assert!(w.status(REBUILD).is_some(), "an edit of the image rebuilds");
	assert!(w.quiet(SETTLE), "once");
	write(&dir.join("data.json"), "{\"n\": 2}\n");
	assert!(w.status(REBUILD).is_some(), "an edit of the data file rebuilds");
}

#[test]
fn a_font_under_the_font_path_is_watched() {
	let dir = project("fonts");
	let w = Watch::start(&dir, &["--font-path", "fonts"], &[]);
	assert!(w.status(FIRST).is_some(), "the first build prints a status line");
	write(&dir.join("fonts/fake.ttf"), "still not a font, but changed");
	assert!(w.status(REBUILD).is_some(), "an edit of a font file rebuilds");
	assert!(w.quiet(SETTLE), "once");
	write(&dir.join("fonts/another.otf"), "a font added");
	assert!(w.status(REBUILD).is_some(), "a font added to the directory rebuilds");
}

#[test]
fn a_change_to_a_package_file_rebuilds() {
	let dir = project("package");
	let pkgs = package_dir(&dir, "wwatch");
	write(&dir.join("main.typ"), "#import \"@local/wwatch:0.1.0\": greet\n#greet\n");
	let w = Watch::start(&dir, &[], &[("TYPST_PACKAGE_CACHE_PATH", &pkgs)]);
	assert!(w.status(FIRST).is_some(), "the first build prints a status line");
	write(&pkgs.join("local/wwatch/0.1.0/lib.typ"), "#let greet = [Packaged, then edited.]\n");
	assert!(w.status(REBUILD).is_some(), "an edit of a package's file rebuilds");
}

#[test]
fn a_file_first_read_after_an_edit_is_watched_from_then_on() {
	let dir = project("grows");
	write(&dir.join("extra.typ"), "#let more = [Extra.]\n");
	let w = Watch::start(&dir, &[], &[]);
	assert!(w.status(FIRST).is_some(), "the first build prints a status line");
	write(&dir.join("extra.typ"), "#let more = [Extra, before anything imports it.]\n");
	assert!(w.quiet(SETTLE), "extra.typ is not read yet, so an edit of it rebuilds nothing");
	write(&dir.join("main.typ"), "#import \"extra.typ\": more\n#more\n");
	assert!(w.status(REBUILD).is_some(), "the edit of the main rebuilds");
	assert!(w.quiet(SETTLE), "once");
	write(&dir.join("extra.typ"), "#let more = [Extra, now imported.]\n");
	assert!(w.status(REBUILD).is_some(), "the file the rebuild read for the first time is now watched");
}

#[test]
fn a_failed_rebuild_keeps_the_last_good_pdf_says_why_and_is_mended_by_the_file_it_stopped_at() {
	let dir = project("failure");
	let w = Watch::start(&dir, &[], &[]);
	assert!(w.status(FIRST).is_some(), "the first build prints a status line");
	let good = pdf(&dir);
	write(&dir.join("lib.typ"), "#let word = (\n");
	assert!(w.complaint("error", REBUILD).is_some(), "a rebuild that fails says why");
	assert!(w.quiet(SETTLE), "and prints no status line for it");
	assert_eq!(pdf(&dir), good, "the last good PDF is as it was");
	write(&dir.join("lib.typ"), "#let word = [Mended.]\n");
	assert!(w.status(REBUILD).is_some(), "mending the file the failed build stopped at rebuilds");
	assert!(w.quiet(SETTLE), "once");
}

#[test]
fn strict_and_the_root_are_honoured_in_the_watch() {
	let dir = project("strict");
	std::fs::create_dir_all(dir.join("sub")).expect("a subdirectory");
	write(&dir.join("sub/main.typ"), "#import \"/lib.typ\": word\n#word\n");
	// Under the root the main sits in `sub` and reaches `lib.typ` above it by a root-absolute path.
	let w = Watch::run(&dir, &["--strict", "--root", "."], "sub/main.typ", &[]);
	assert!(w.status(FIRST).is_some(), "a main that reaches a root-absolute path builds under --root");
	write(&dir.join("lib.typ"), "#set text(font: \"No Such Family Anywhere\")\n#let word = [Strict.]\n");
	assert!(w.complaint("strict:", REBUILD).is_some(),
		"under --strict a warning that refuses the compile fails the rebuild, and says so");
	assert!(w.quiet(SETTLE), "with no status line");
}

#[test]
fn the_alias_writes_where_it_is_told_and_reads_no_settings_file() {
	let dir = project("alias");
	std::fs::create_dir_all(dir.join("sub")).expect("a subdirectory");
	write(&dir.join("sub/main.typ"), "#import \"/lib.typ\": word\n#word\n");
	// A settings file `austenite watch` would obey: another place for the PDF, and its colour space.
	write(&dir.join("austenite.jdat"), "{\"output\": \"elsewhere.pdf\", \"colour\": {\"space\": \"cmyk\"}}");
	let w = Watch::run(&dir, &["--root", "."], "sub/main.typ", &[]);
	assert!(w.status(FIRST).is_some(), "the first build prints a status line");
	let bytes = pdf(&dir);
	assert!(bytes.starts_with(b"%PDF-1.7"), "the PDF is in the output directory named from the working directory");
	assert!(!bytes.windows(10).any(|w| w == b"DeviceCMYK"), "the settings file is not read");
	assert!(!dir.join("sub/out").exists() && !dir.join("sub/elsewhere.pdf").exists() && !dir.join("elsewhere.pdf").exists(),
		"nothing is written beside the source or where the file says");
}
