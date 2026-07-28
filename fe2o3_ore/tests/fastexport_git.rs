//! Checks the fast-import stream parser against git itself.
//!
//! The unit tests in the module read streams written by hand, which proves the
//! parser self-consistent and nothing more. This test builds a real repository
//! with the git binary, has git write the stream, and asserts on what comes
//! back out: git is the oracle, and the fixtures were not authored by the same
//! hand as the parser.
//!
//! It is marked `#[ignore]` because it needs a `git` binary on the path and
//! writes to a temporary directory, neither of which a plain `cargo test`
//! should assume. Run it with:
//!
//! ```text
//! cargo test -p oxedyne_fe2o3_ore --test fastexport_git -- --ignored --nocapture
//! ```

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_ore::fastexport::{
	BlobRef,
	Event,
	FileChange,
	FileMode,
	ObjRef,
	Parser,
};

use std::{
	env,
	fs,
	path::{
		Path,
		PathBuf,
	},
	process::Command,
	time::{
		SystemTime,
		UNIX_EPOCH,
	},
};


/// The binary file's contents: NUL bytes, a bare line feed and a byte no text
/// encoding will leave alone.
const BINARY: &[u8] = b"\x00\x01\n\x02binary\x00\xff\n";

/// A fixed timestamp, so the identity lines are predictable.
const WHEN: &str = "1700000000 +1000";


/// Runs git in `dir`, returning its standard output.
///
/// The environment is stripped of the caller's git configuration so that a
/// developer's global settings, hooks or signing keys cannot change what the
/// stream says.
fn git(dir: &Path, args: &[&str])
	-> Outcome<Vec<u8>>
{
	let mut cmd = Command::new("git");
	cmd.current_dir(dir)
		.args(args)
		.env("GIT_CONFIG_GLOBAL", "/dev/null")
		.env("GIT_CONFIG_NOSYSTEM", "1")
		.env("GIT_AUTHOR_NAME", "Ada Lovelace")
		.env("GIT_AUTHOR_EMAIL", "ada@example.org")
		.env("GIT_COMMITTER_NAME", "Ada Lovelace")
		.env("GIT_COMMITTER_EMAIL", "ada@example.org")
		.env("GIT_AUTHOR_DATE", WHEN)
		.env("GIT_COMMITTER_DATE", WHEN);
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
fn scratch_dir()
	-> Outcome<PathBuf>
{
	let stamp = res!(SystemTime::now().duration_since(UNIX_EPOCH));
	let dir = env::temp_dir().join(fmt!(
		"fe2o3_ore_fastexport_{}_{}",
		std::process::id(),
		stamp.as_nanos(),
	));
	res!(fs::create_dir_all(&dir));
	Ok(dir)
}

/// Builds a repository holding a rename, a binary file, a merge and an
/// annotated tag, and returns the stream git writes for it.
fn build_repo_and_export(dir: &Path)
	-> Outcome<Vec<u8>>
{
	res!(git(dir, &["-c", "init.defaultBranch=main", "init", "--quiet", "."]));
	// A hooks path that does not exist keeps any hook out of the way.
	res!(git(dir, &["config", "core.hooksPath", "nohooks"]));
	res!(git(dir, &["config", "commit.gpgsign", "false"]));
	res!(git(dir, &["config", "tag.gpgsign", "false"]));
	res!(git(dir, &["config", "user.name", "Ada Lovelace"]));
	res!(git(dir, &["config", "user.email", "ada@example.org"]));

	res!(fs::write(dir.join("a.txt"), b"hello\n"));
	res!(fs::write(dir.join("bin.dat"), BINARY));
	res!(git(dir, &["add", "a.txt", "bin.dat"]));
	res!(git(dir, &["commit", "--quiet", "--no-verify", "-m", "first commit"]));

	res!(git(dir, &["mv", "a.txt", "renamed.txt"]));
	res!(git(dir, &["commit", "--quiet", "--no-verify", "-m", "rename a"]));

	res!(git(dir, &["checkout", "--quiet", "-b", "side", "HEAD~1"]));
	res!(fs::write(dir.join("side.txt"), b"side\n"));
	res!(git(dir, &["add", "side.txt"]));
	res!(git(dir, &["commit", "--quiet", "--no-verify", "-m", "side work"]));

	res!(git(dir, &["checkout", "--quiet", "main"]));
	res!(git(dir, &["merge", "--quiet", "--no-ff", "--no-verify", "side", "-m", "merge side"]));

	res!(git(dir, &["tag", "-a", "v1", "-m", "release one"]));

	git(dir, &["fast-export", "--all", "-M"])
}

/// Feeds a stream to the parser in awkward chunks, as a pipe would deliver it.
fn parse_in_chunks(stream: &[u8])
	-> Outcome<Vec<Event>>
{
	let mut parser = Parser::new();
	let mut events = Vec::new();
	for chunk in stream.chunks(13) {
		parser.feed(chunk);
		while let Some(event) = res!(parser.next_event()) {
			events.push(event);
		}
	}
	parser.end();
	while let Some(event) = res!(parser.next_event()) {
		events.push(event);
	}
	Ok(events)
}

/// A stream written by git, for a repository with a rename, a binary file, a
/// merge and an annotated tag, parses into the shape the repository had.
#[test]
#[ignore = "needs a git binary and a writable temporary directory"]
fn parses_a_stream_written_by_git() -> Outcome<()> {
	let dir = res!(scratch_dir());
	let outcome = run_against_git(&dir);
	// Clear up whatever happened, then report.
	let _ = fs::remove_dir_all(&dir);
	outcome
}

/// The body of the test, kept separate so the scratch directory is removed
/// however it ends.
fn run_against_git(dir: &Path)
	-> Outcome<()>
{
	let stream = res!(build_repo_and_export(dir));
	assert!(!stream.is_empty(), "git wrote no stream");

	let events = res!(parse_in_chunks(&stream));
	// Parsing the same bytes in one go must give the same events.
	let mut whole = Parser::new();
	whole.feed(&stream);
	whole.end();
	let mut at_once = Vec::new();
	while let Some(event) = res!(whole.next_event()) {
		at_once.push(event);
	}
	assert_eq!(events, at_once, "chunked parse differed from whole parse");

	let mut commits = Vec::new();
	let mut blobs = Vec::new();
	let mut tags = Vec::new();
	let mut resets = Vec::new();
	for event in &events {
		match event {
			Event::Commit(commit)			=> commits.push(commit),
			Event::Blob(blob)			=> blobs.push(blob),
			Event::Tag(tag)				=> tags.push(tag),
			Event::Reset { refname, .. }		=> resets.push(refname.clone()),
			Event::Progress(_)			=> {},
			Event::Checkpoint			=> {},
			Event::Alias { .. }			=> {},
			Event::Feature { .. }			=> {},
			Event::Opt(_)				=> {},
			Event::Done				=> {},
		}
	}

	// Four commits: the root, the rename, the side branch and the merge.
	assert_eq!(commits.len(), 4, "commits were {:?}",
		commits.iter().map(|c| String::from_utf8_lossy(&c.message)).collect::<Vec<_>>());

	let messages: Vec<String> = commits.iter()
		.map(|c| String::from_utf8_lossy(&c.message).trim_end().to_string())
		.collect();
	for want in ["first commit", "rename a", "side work", "merge side"] {
		assert!(messages.iter().any(|m| m == want), "no commit said {:?}: {:?}",
			want, messages);
	}

	// Every commit carries the identity git was told to use, parsed into
	// fields rather than left as a line.
	for commit in &commits {
		assert_eq!(commit.committer.email, b"ada@example.org".to_vec());
		assert_eq!(commit.committer.name, b"Ada Lovelace".to_vec());
		assert_eq!(commit.committer.when.secs, 1_700_000_000);
		assert_eq!(commit.committer.when.tz.mins, 600);
		assert!(commit.mark.is_some(), "commit has no mark");
	}

	// The rename survives as a rename, not as a delete and an add.
	let renames: Vec<&FileChange> = commits.iter()
		.flat_map(|c| c.changes.iter())
		.filter(|change| matches!(change, FileChange::Rename { .. }))
		.collect();
	assert_eq!(renames.len(), 1, "expected exactly one rename, got {:?}", renames);
	assert_eq!(renames[0], &FileChange::Rename {
		src:	b"a.txt".to_vec(),
		dst:	b"renamed.txt".to_vec(),
	});

	// The merge commit has a first parent and one other.
	let merges: Vec<&&oxedyne_fe2o3_ore::fastexport::Commit> = commits.iter()
		.filter(|c| !c.merges.is_empty())
		.collect();
	assert_eq!(merges.len(), 1, "expected exactly one merge commit");
	assert_eq!(merges[0].merges.len(), 1);
	assert!(merges[0].from.is_some(), "merge commit has no first parent");
	assert_eq!(String::from_utf8_lossy(&merges[0].message).trim_end(), "merge side");
	assert!(matches!(merges[0].merges[0], ObjRef::Mark(_)),
		"the second parent was not a mark");

	// The binary blob arrives byte for byte.
	assert!(blobs.iter().any(|b| b.data == BINARY),
		"no blob held the binary file; blobs were {:?}",
		blobs.iter().map(|b| b.data.len()).collect::<Vec<_>>());
	assert!(blobs.iter().any(|b| b.data == b"hello\n".to_vec()),
		"no blob held the text file");

	// Every file entry points at a blob the stream has already marked, and
	// every one is a normal file.
	let marked: Vec<u64> = blobs.iter().filter_map(|b| b.mark).collect();
	for commit in &commits {
		for change in &commit.changes {
			if let FileChange::Modify { mode, data, .. } = change {
				assert_eq!(*mode, FileMode::Normal);
				match data {
					BlobRef::Mark(n) => assert!(marked.contains(n),
						"file entry names mark :{}, which no blob minted", n),
					other => return Err(err!(
						"Expected a mark, got {:?}.", other; Test)),
				}
			}
		}
	}

	// The annotated tag arrives with its tagger and its message.
	assert_eq!(tags.len(), 1, "expected exactly one tag");
	assert_eq!(tags[0].name, "v1");
	assert!(String::from_utf8_lossy(&tags[0].message).starts_with("release one"),
		"tag message was {:?}", String::from_utf8_lossy(&tags[0].message));
	match &tags[0].tagger {
		Some(person) => assert_eq!(person.email, b"ada@example.org".to_vec()),
		None => return Err(err!("The tag has no tagger."; Test)),
	}

	// Both branches are named somewhere in the stream.
	let refnames: Vec<&str> = commits.iter()
		.map(|c| c.refname.as_str())
		.chain(resets.iter().map(|r| r.as_str()))
		.collect();
	for want in ["refs/heads/main", "refs/heads/side"] {
		assert!(refnames.iter().any(|r| *r == want),
			"no reference named {:?}: {:?}", want, refnames);
	}

	// Nothing was left unparsed.
	assert!(whole.is_exhausted(), "bytes were left over: {:?}",
		String::from_utf8_lossy(whole.remaining()));

	Ok(())
}
