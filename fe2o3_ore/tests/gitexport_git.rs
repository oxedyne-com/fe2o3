//! Checks the fast-import stream emitter against git itself.
//!
//! The unit tests in the module read the emitted stream back with the parser
//! next door, which proves the two halves agree and nothing more: both were
//! written by the same hand and could be wrong together. This test pipes what
//! the emitter produces into a real `git fast-import`, checks the result out,
//! and compares it byte for byte and mode for mode with what was asked for. Git
//! is the oracle.
//!
//! It is marked `#[ignore]` because it needs a `git` binary on the path and
//! writes to a temporary directory, neither of which a plain `cargo test`
//! should assume. Run it with:
//!
//! ```text
//! cargo test -p oxedyne_fe2o3_ore --test gitexport_git -- --ignored --nocapture
//! ```

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_ore::fastexport::{
	BlobRef,
	FileMode,
	ObjRef,
	Person,
	TzOffset,
	When,
};
use oxedyne_fe2o3_ore::gitexport::{
	changes,
	Change,
	Commit,
	Entry,
	Stream,
	Tree,
};

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{
	Path,
	PathBuf,
};
use std::process::{
	Command,
	Stdio,
};
use std::time::{
	SystemTime,
	UNIX_EPOCH,
};


/// A payload no text encoding leaves alone: NUL bytes, a bare line feed and a
/// byte that is not valid UTF-8.
const BINARY: &[u8] = b"\x00\x01\n\x02binary\x00\xff\n";

/// A fixed moment, so a run is the same run every time.
const WHEN: When = When { secs: 1_700_000_000, tz: TzOffset { mins: 600, neg: false } };


/// Runs git in `dir`, returning its standard output.
///
/// The caller's git configuration, hooks and signing keys are all fenced off, so
/// that nothing on the developer's machine can change what git does here.
fn git(dir: &Path, args: &[&str])
	-> Outcome<Vec<u8>>
{
	let mut cmd = Command::new("git");
	cmd.current_dir(dir)
		.args(args)
		.env("GIT_CONFIG_GLOBAL", "/dev/null")
		.env("GIT_CONFIG_NOSYSTEM", "1");
	let out = res!(cmd.output());
	if !out.status.success() {
		return Err(err!(
			"git {} failed: {}", args.join(" "),
			String::from_utf8_lossy(&out.stderr);
		Test, IO));
	}
	Ok(out.stdout)
}

/// Returns a directory of its own for this test run.
fn scratch_dir(what: &str)
	-> Outcome<PathBuf>
{
	let stamp = res!(SystemTime::now().duration_since(UNIX_EPOCH));
	let dir = env::temp_dir().join(fmt!(
		"fe2o3_ore_gitexport_{}_{}_{}", what, std::process::id(), stamp.as_nanos(),
	));
	res!(fs::create_dir_all(&dir));
	Ok(dir)
}

/// Builds a bare repository from a stream, and returns where it is.
///
/// Git is run with `--done`, so a stream cut short is refused rather than half
/// applied: what comes back is either the whole history or an error.
fn build(dir: &Path, stream: &[u8])
	-> Outcome<PathBuf>
{
	let repo = dir.join("mirror.git");
	res!(fs::create_dir_all(&repo));
	res!(git(&repo, &["init", "--bare", "--quiet", "."]));
	let mut child = res!(Command::new("git")
		.current_dir(&repo)
		.args(["fast-import", "--quiet", "--done"])
		.env("GIT_CONFIG_GLOBAL", "/dev/null")
		.env("GIT_CONFIG_NOSYSTEM", "1")
		.stdin(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn());
	match child.stdin.take() {
		Some(mut pipe) => res!(pipe.write_all(stream)),
		None => return Err(err!("git fast-import gave no standard input."; Test, IO)),
	}
	let out = res!(child.wait_with_output());
	if !out.status.success() {
		return Err(err!(
			"git fast-import refused the stream: {}",
			String::from_utf8_lossy(&out.stderr);
		Test, IO));
	}
	Ok(repo)
}

/// Checks a reference of a bare repository out into a working tree, and returns
/// every file it holds, by the bytes of its path.
fn checkout(dir: &Path, repo: &Path, refname: &str, what: &str)
	-> Outcome<BTreeMap<Vec<u8>, Entry>>
{
	let work = dir.join(what);
	res!(fs::create_dir_all(&work));
	res!(git(&work, &["clone", "--quiet", "--no-checkout",
		&fmt!("{}", repo.display()), "."]));
	res!(git(&work, &["checkout", "--quiet", "--force", refname]));
	let mut out = BTreeMap::new();
	res!(collect(&work, &[], &mut out));
	Ok(out)
}

/// Reads one directory of a checkout, recursing, with names as bytes.
fn collect(dir: &Path, prefix: &[u8], out: &mut BTreeMap<Vec<u8>, Entry>)
	-> Outcome<()>
{
	for entry in res!(fs::read_dir(dir)) {
		let entry = res!(entry);
		let name = entry.file_name();
		let name = name.as_bytes();
		if name == b".git" {
			continue;
		}
		let mut rel = prefix.to_vec();
		if !rel.is_empty() {
			rel.push(b'/');
		}
		rel.extend_from_slice(name);
		let kind = res!(entry.file_type());
		if kind.is_dir() {
			res!(collect(&entry.path(), &rel, out));
		} else if kind.is_symlink() {
			let target = res!(fs::read_link(entry.path()));
			out.insert(rel, Entry {
				mode:	FileMode::Symlink,
				data:	target.as_os_str().as_bytes().to_vec(),
			});
		} else if kind.is_file() {
			let meta = res!(entry.metadata());
			out.insert(rel, Entry {
				mode: if meta.permissions().mode() & 0o100 != 0 {
					FileMode::Executable
				} else {
					FileMode::Normal
				},
				data: res!(fs::read(entry.path())),
			});
		}
	}
	Ok(())
}

/// Returns an identity to write commits under.
fn ada() -> Person {
	Person {
		name:	b"Ada Lovelace".to_vec(),
		email:	b"ada@example.org".to_vec(),
		when:	WHEN,
	}
}

/// Returns a tree entry.
fn entry(mode: FileMode, data: &[u8]) -> Entry {
	Entry { mode, data: data.to_vec() }
}

/// Says which paths differ, and how, so a failure names the file.
fn same(got: &BTreeMap<Vec<u8>, Entry>, want: &Tree, what: &str)
	-> Outcome<()>
{
	let mine: Vec<String> = got.keys()
		.map(|p| String::from_utf8_lossy(p).into_owned()).collect();
	let yours: Vec<String> = want.keys()
		.map(|p| String::from_utf8_lossy(p).into_owned()).collect();
	if mine != yours {
		return Err(err!(
			"{}: git checked out {:?} where {:?} was asked for.", what, mine, yours;
		Test, Mismatch));
	}
	for (path, wanted) in want {
		let shown = String::from_utf8_lossy(path).into_owned();
		let held = match got.get(path) {
			Some(h) => h,
			None => return Err(err!("{}: git has no {:?}.", what, shown; Test, Missing)),
		};
		if held.mode != wanted.mode {
			return Err(err!(
				"{}: git checked {:?} out as {} where {} was asked for.",
				what, shown, held.mode, wanted.mode;
			Test, Mismatch));
		}
		if held.data != wanted.data {
			return Err(err!(
				"{}: git checked {:?} out as {} bytes where {} were asked for.",
				what, shown, held.data.len(), wanted.data.len();
			Test, Mismatch));
		}
	}
	Ok(())
}


/// A tree emitted is the tree git checks out, byte for byte and mode for mode.
///
/// The tree holds everything the emitter has to survive: a binary file, an
/// executable one, a path holding a space, a path holding bytes that are not
/// UTF-8, a path holding a line feed, and a deep directory.
#[test]
#[ignore = "needs a git binary and a writable temporary directory"]
fn a_tree_emitted_is_the_tree_git_holds() -> Outcome<()> {
	let dir = res!(scratch_dir("tree"));
	let mut want = Tree::new();
	want.insert(b"plain.txt".to_vec(), entry(FileMode::Normal, b"hello\n"));
	want.insert(b"bin.dat".to_vec(), entry(FileMode::Normal, BINARY));
	want.insert(b"build.sh".to_vec(), entry(FileMode::Executable, b"#!/bin/sh\necho\n"));
	want.insert(b"a space.txt".to_vec(), entry(FileMode::Normal, b"spaced\n"));
	want.insert(b"line\nfeed.txt".to_vec(), entry(FileMode::Normal, b"odd\n"));
	want.insert(vec![0xff, 0xfe, b'.', b'b'], entry(FileMode::Normal, b"not utf8\n"));
	want.insert(b"deep/down/here.txt".to_vec(), entry(FileMode::Normal, b"deep\n"));

	let mut stream = Stream::new();
	res!(stream.commit(&Commit {
		refname:	fmt!("refs/heads/main"),
		mark:		Some(1),
		author:		None,
		committer:	ada(),
		message:	b"everything at once".to_vec(),
		from:		None,
		merges:		Vec::new(),
		changes:	res!(changes(&Tree::new(), &want)),
	}));
	stream.done();

	let repo = res!(build(&dir, stream.bytes()));
	let got = res!(checkout(&dir, &repo, "main", "check"));
	let outcome = same(&got, &want, "the one commit");
	let _ = fs::remove_dir_all(&dir);
	outcome
}

/// A history emitted is a history git holds: each commit's tree is what was
/// asked for, a merge has two parents in the order given, and a lightweight tag
/// names the commit it was pointed at.
#[test]
#[ignore = "needs a git binary and a writable temporary directory"]
fn a_history_emitted_is_the_history_git_holds() -> Outcome<()> {
	let dir = res!(scratch_dir("history"));

	// The root: two files, one of them a script.
	let mut base = Tree::new();
	base.insert(b"f.txt".to_vec(), entry(FileMode::Normal, b"alpha\nbeta\n"));
	base.insert(b"run.sh".to_vec(), entry(FileMode::Normal, b"#!/bin/sh\n"));

	// One line of work renames a file and makes the script runnable.
	let mut trunk = Tree::new();
	trunk.insert(b"kept.txt".to_vec(), entry(FileMode::Normal, b"alpha\nbeta\n"));
	trunk.insert(b"run.sh".to_vec(), entry(FileMode::Executable, b"#!/bin/sh\n"));

	// Another, from the same root, adds a file of its own.
	let mut side = base.clone();
	side.insert(b"s.txt".to_vec(), entry(FileMode::Normal, b"only here\n"));

	// And the two are joined, with the script's bit taken away again.
	let mut merged = Tree::new();
	merged.insert(b"kept.txt".to_vec(), entry(FileMode::Normal, b"alpha\nresolved\n"));
	merged.insert(b"run.sh".to_vec(), entry(FileMode::Normal, b"#!/bin/sh\n"));
	merged.insert(b"s.txt".to_vec(), entry(FileMode::Normal, b"only here\n"));

	let mut stream = Stream::new();
	let commit = |mark, from: Option<u64>, merges: Vec<u64>, msg: &str, ch| Commit {
		refname:	fmt!("refs/heads/main"),
		mark:		Some(mark),
		author:		None,
		committer:	ada(),
		message:	msg.as_bytes().to_vec(),
		from:		from.map(ObjRef::Mark),
		merges:		merges.into_iter().map(ObjRef::Mark).collect(),
		changes:	ch,
	};
	res!(stream.commit(&commit(1, None, vec![], "base",
		res!(changes(&Tree::new(), &base)))));
	res!(stream.commit(&commit(2, Some(1), vec![], "trunk",
		res!(changes(&base, &trunk)))));
	res!(stream.commit(&commit(3, Some(1), vec![], "side",
		res!(changes(&base, &side)))));
	// A merge's changes are applied over its first parent's tree, which is what
	// the difference here is taken against.
	res!(stream.commit(&commit(4, Some(2), vec![3], "merge",
		res!(changes(&trunk, &merged)))));
	res!(stream.reset("refs/tags/v1", Some(&ObjRef::Mark(2))));
	stream.done();

	let repo = res!(build(&dir, stream.bytes()));
	let outcome = (|| -> Outcome<()> {
		res!(same(&res!(checkout(&dir, &repo, "main", "at_head")), &merged, "the merge"));
		res!(same(&res!(checkout(&dir, &repo, "v1", "at_tag")), &trunk, "the tag"));

		// Two parents, in the order the stream gave them.
		let out = res!(git(&repo, &["rev-list", "--parents", "-n", "1", "main"]));
		let text = String::from_utf8_lossy(&out).into_owned();
		let ids: Vec<&str> = text.trim().split(' ').collect();
		assert_eq!(ids.len(), 3, "the head has two parents: {}", text.trim());
		let first = res!(git(&repo, &["rev-parse", "main^1"]));
		assert_eq!(ids[1], String::from_utf8_lossy(&first).trim(),
			"and the first of them is the one named by from");

		// Four commits, and no more.
		let out = res!(git(&repo, &["rev-list", "--all", "--count"]));
		assert_eq!(String::from_utf8_lossy(&out).trim(), "4",
			"four commits were written and four are held");

		// The tag points at a commit and mints no object of its own.
		let kind = res!(git(&repo, &["cat-file", "-t", "refs/tags/v1"]));
		assert_eq!(String::from_utf8_lossy(&kind).trim(), "commit",
			"a lightweight tag names the commit directly");
		Ok(())
	})();
	let _ = fs::remove_dir_all(&dir);
	outcome
}

/// The same content gives the same tree object however the commit around it
/// differs, which is what makes a mirror checkable against the repository it
/// mirrors.
///
/// This is the round-trip bar stated as an assertion: git's blob and tree names
/// are hashes of content alone, so an exported tree can be compared with the
/// original's by name and not merely by walking it. Commit names cannot, and the
/// second half says so.
#[test]
#[ignore = "needs a git binary and a writable temporary directory"]
fn a_tree_object_is_a_function_of_its_content_alone() -> Outcome<()> {
	let dir = res!(scratch_dir("hashes"));
	let mut want = Tree::new();
	want.insert(b"f.txt".to_vec(), entry(FileMode::Normal, b"content\n"));
	want.insert(b"sub/g.sh".to_vec(), entry(FileMode::Executable, b"#!/bin/sh\n"));
	let ch = res!(changes(&Tree::new(), &want));

	let mut trees = Vec::new();
	let mut commits = Vec::new();
	// Two repositories over the same tree, differing in everything a commit
	// object holds besides that tree.
	for (n, (msg, secs)) in [("one message", 1_000i64), ("another entirely", 2_000)]
		.into_iter().enumerate()
	{
		let mut stream = Stream::new();
		res!(stream.commit(&Commit {
			refname:	fmt!("refs/heads/main"),
			mark:		Some(1),
			author:		None,
			committer:	Person {
				name:	fmt!("Writer {}", n).into_bytes(),
				email:	fmt!("w{}@example.org", n).into_bytes(),
				when:	When { secs, tz: TzOffset::new(0) },
			},
			message:	msg.as_bytes().to_vec(),
			from:		None,
			merges:		Vec::new(),
			changes:	ch.clone(),
		}));
		stream.done();
		let sub = dir.join(fmt!("r{}", n));
		res!(fs::create_dir_all(&sub));
		let repo = res!(build(&sub, stream.bytes()));
		let tree = res!(git(&repo, &["rev-parse", "main^{tree}"]));
		let commit = res!(git(&repo, &["rev-parse", "main"]));
		trees.push(String::from_utf8_lossy(&tree).trim().to_owned());
		commits.push(String::from_utf8_lossy(&commit).trim().to_owned());
	}
	let outcome = (|| -> Outcome<()> {
		assert_eq!(trees[0], trees[1],
			"the same content is the same tree object: {:?}", trees);
		assert_ne!(commits[0], commits[1],
			"and a different message and date is a different commit: {:?}", commits);
		Ok(())
	})();
	let _ = fs::remove_dir_all(&dir);
	outcome
}

/// Git takes a stream naming a path as both a file and a directory and drops
/// the file without a word, so the emitter refuses it first.
///
/// The half of this test that matters is the second: git's own behaviour is
/// measured rather than assumed, so the refusal is answering something real.
#[test]
#[ignore = "needs a git binary and a writable temporary directory"]
fn git_would_drop_a_file_the_emitter_refuses_to_write() -> Outcome<()> {
	let dir = res!(scratch_dir("clash"));

	// What the emitter does.
	let mut want = Tree::new();
	want.insert(b"a".to_vec(), entry(FileMode::Normal, b"a file\n"));
	want.insert(b"a/b".to_vec(), entry(FileMode::Normal, b"and a directory\n"));
	assert!(changes(&Tree::new(), &want).is_err(),
		"a path that is both a file and a directory is refused");

	// What git does, given the same thing said by hand.
	let mut stream = Stream::new();
	res!(stream.commit(&Commit {
		refname:	fmt!("refs/heads/main"),
		mark:		Some(1),
		author:		None,
		committer:	ada(),
		message:	b"both at once".to_vec(),
		from:		None,
		merges:		Vec::new(),
		changes:	vec![
			Change::Modify {
				mode:	FileMode::Normal,
				data:	BlobRef::Inline(b"a file\n".to_vec()),
				path:	b"a".to_vec(),
			},
			Change::Modify {
				mode:	FileMode::Normal,
				data:	BlobRef::Inline(b"and a directory\n".to_vec()),
				path:	b"a/b".to_vec(),
			},
		],
	}));
	stream.done();
	let repo = res!(build(&dir, stream.bytes()));
	let got = res!(checkout(&dir, &repo, "main", "check"));
	let outcome = (|| -> Outcome<()> {
		let paths: Vec<String> = got.keys()
			.map(|p| String::from_utf8_lossy(p).into_owned()).collect();
		assert_eq!(paths, vec![fmt!("a/b")],
			"git kept the directory and dropped the file, saying nothing: {:?}", paths);
		Ok(())
	})();
	let _ = fs::remove_dir_all(&dir);
	outcome
}

/// A symbolic link is a mode and a blob whose bytes are the target, and git
/// checks one out as a link.
#[test]
#[ignore = "needs a git binary and a writable temporary directory"]
fn a_symlink_is_checked_out_as_a_link() -> Outcome<()> {
	let dir = res!(scratch_dir("symlink"));
	let mut want = Tree::new();
	want.insert(b"real.txt".to_vec(), entry(FileMode::Normal, b"the target\n"));
	want.insert(b"link".to_vec(), entry(FileMode::Symlink, b"real.txt"));

	let mut stream = Stream::new();
	res!(stream.commit(&Commit {
		refname:	fmt!("refs/heads/main"),
		mark:		Some(1),
		author:		None,
		committer:	ada(),
		message:	b"a link".to_vec(),
		from:		None,
		merges:		Vec::new(),
		changes:	res!(changes(&Tree::new(), &want)),
	}));
	stream.done();
	let repo = res!(build(&dir, stream.bytes()));
	let got = res!(checkout(&dir, &repo, "main", "check"));
	let outcome = same(&got, &want, "the link");
	let _ = fs::remove_dir_all(&dir);
	outcome
}

/// A stream cut short is refused rather than half applied, which is what `done`
/// and git's own `--done` are for.
#[test]
#[ignore = "needs a git binary and a writable temporary directory"]
fn a_stream_that_never_says_done_is_refused() -> Outcome<()> {
	let dir = res!(scratch_dir("cut"));
	let mut want = Tree::new();
	want.insert(b"f.txt".to_vec(), entry(FileMode::Normal, b"x\n"));
	let mut stream = Stream::new();
	res!(stream.commit(&Commit {
		refname:	fmt!("refs/heads/main"),
		mark:		Some(1),
		author:		None,
		committer:	ada(),
		message:	b"unfinished".to_vec(),
		from:		None,
		merges:		Vec::new(),
		changes:	res!(changes(&Tree::new(), &want)),
	}));
	// No `done`, as a producer that died part way through would leave it.
	let outcome = match build(&dir, stream.bytes()) {
		Ok(_)	=> Err(err!("git accepted a stream that never said done."; Test)),
		Err(_)	=> Ok(()),
	};
	let _ = fs::remove_dir_all(&dir);
	outcome
}
