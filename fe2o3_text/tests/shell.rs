//! `shell`: a command line read into words and simple commands the way a POSIX shell quotes it.
//!
//! Moved here from the Daimond engine's spend notice (8 Nov 2026), which names the file a
//! command read, through `sh -c` and `cd`.

use oxedyne_fe2o3_text::shell::{
	commands,
	join,
	quote,
	tokens,
	Command,
	Token,
	Word,
};


fn texts(cmd: &Command) -> Vec<&str> {
	cmd.words.iter().map(|w| w.text.as_str()).collect()
}

#[test]
fn quoting_is_removed_as_the_shell_removes_it() {
	let cmds = commands(r#"grep -n 'a b' "c \"d\" \x" e\ f '' g"#);
	assert_eq!(1, cmds.len());
	assert_eq!(vec!["grep", "-n", "a b", "c \"d\" \\x", "e f", "", "g"], texts(&cmds[0]));
	assert!(cmds[0].words.iter().all(|w| w.literal));
}

#[test]
fn an_expanded_word_is_not_literal() {
	for (line, lit) in [
		("cat $HOME/a",		false),
		("cat \"$x\"",		false),
		("cat `pwd`",		false),
		("cat *.rs",		false),
		("cat a?.rs",		false),
		("cat ~/a",			false),
		("cat a~b",			true),
		("cat '$x'",		true),
		("cat '~/a'",		true),
	] {
		let cmds = commands(line);
		assert_eq!(lit, cmds[0].words[1].literal, "{}", line);
	}
}

#[test]
fn operators_end_a_simple_command() {
	let cmds = commands("cd web && cat a; ls | wc -l & (echo x)\nnl b # cat c\ntac d");
	let progs: Vec<Option<&str>> = cmds.iter().map(|c| c.program()).collect();
	assert_eq!(vec![Some("cd"), Some("cat"), Some("ls"), Some("wc"), Some("echo"), Some("nl"),
		Some("tac")], progs);
	assert_eq!(vec!["nl", "b"], texts(&cmds[5]));
}

#[test]
fn redirections_are_not_operands() {
	let cmds = commands("sort < in.txt > out.txt 2>/dev/null 2>&1 &>> log");
	assert_eq!(1, cmds.len());
	assert_eq!(vec!["sort"], texts(&cmds[0]));
	assert_eq!(vec![Word::literal("in.txt")], cmds[0].read_from);

	// A here-document, a here-string and a descriptor duplication read no file.
	for line in ["cat << EOF", "cat <<< text", "cat <&3", "cat >| f"] {
		let cmds = commands(line);
		assert_eq!(vec!["cat"], texts(&cmds[0]), "{}", line);
		assert!(cmds[0].read_from.is_empty(), "{}", line);
	}
	assert_eq!(vec![Token::Word(Word::literal("a")), Token::Redirect, Token::Word(Word::literal("b"))],
		tokens("a 2>b"));
	// A quoted number is a word, not a descriptor.
	assert_eq!(vec!["echo", "2"], texts(&commands("echo '2'>f")[0]));
}

#[test]
fn assignments_before_the_program_are_skipped() {
	let cmds = commands("LANG=C A_1=x grep -n x=y f");
	assert_eq!(Some("grep"), cmds[0].program());
	assert_eq!(4, cmds[0].args().len());
	assert_eq!(Some("cat"), commands("/usr/bin/cat f")[0].program());
	assert_eq!(None, commands("$EDITOR f")[0].program());
}

#[test]
fn a_shell_hands_over_its_dash_c_script() {
	let cmd = Command::of_argv(&["bash".to_string(), "-lc".to_string(), "cd a && cat b".to_string()]);
	assert!(cmd.is_shell());
	assert_eq!(Some("cd a && cat b"), cmd.script().map(|w| w.text.as_str()));
	assert_eq!(None, commands("sh run.sh")[0].script());
	assert_eq!(None, commands("sh --norc run.sh")[0].script());
	assert_eq!(None, commands("python3 -c 'print(1)'")[0].script());
	assert!(!commands("sh -c \"cat $f\"")[0].script().map_or(true, |w| w.literal));
}

#[test]
fn cd_moves_the_folder_or_loses_it() {
	let mut dir = Some(String::new());
	assert!(!commands("cat a")[0].cd(&mut dir));
	assert!(commands("cd web")[0].cd(&mut dir));
	assert_eq!(Some("web".to_string()), dir);
	assert!(commands("cd apps/")[0].cd(&mut dir));
	assert_eq!(Some("web/apps/".to_string()), dir);
	assert_eq!(Some("web/apps/x.rs".to_string()), join(&dir, "x.rs"));
	assert!(commands("cd /etc")[0].cd(&mut dir));
	assert_eq!(Some("/etc".to_string()), dir);
	for lost in ["cd", "cd -", "cd $HOME", "cd ~"] {
		let mut dir = Some("/etc".to_string());
		assert!(commands(lost)[0].cd(&mut dir));
		assert_eq!(None, dir, "{}", lost);
		assert_eq!(None, join(&dir, "x"));
		assert_eq!(Some("/x".to_string()), join(&dir, "/x"));
	}
}

#[test]
fn quote_reads_back_as_written() {
	for s in ["plain/path-1.rs", "my notes.md", "it's", "", "$x", "a\nb"] {
		let q = quote(s);
		let cmds = commands(&format!("cat {}", q));
		assert_eq!(s, cmds[0].words[1].text, "{} quoted as {}", s, q);
		assert!(cmds[0].words[1].literal, "{}", q);
	}
	assert_eq!("plain/path-1.rs", quote("plain/path-1.rs"));
	assert_eq!("'it'\\''s'", quote("it's"));
}
