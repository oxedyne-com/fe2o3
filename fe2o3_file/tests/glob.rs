//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_file::glob::{
    Glob,
    IgnoreFile,
    EDITOR_DROPPINGS,
    SYNC_DROPPINGS,
};
use oxedyne_fe2o3_text::secret;

use oxedyne_fe2o3_core::{
    prelude::*,
    test::test_it,
};


fn glob(pattern: &str) -> Outcome<Glob> {
    Glob::new(pattern.as_bytes())
}

pub fn test_glob(filter: &'static str) -> Outcome<()> {

    res!(test_it(filter, &["Star stays within one component 000", "all", "glob"], || {
        let g = res!(glob("*.tmp"));
        assert!(g.matches(b"a.tmp", false));
        assert!(g.matches(b"deep/down/b.tmp", false), "unanchored, so any depth");
        assert!(!g.matches(b"a.tmpx", false));
        let g = res!(glob("src/*.rs"));
        assert!(g.matches(b"src/lib.rs", false));
        assert!(!g.matches(b"src/deep/lib.rs", false), "a star does not cross a slash");
        assert!(!g.matches(b"other/src/lib.rs", false), "an inner slash anchors");
        Ok(())
    }));

    res!(test_it(filter, &["Question mark and classes take one byte 000", "all", "glob"], || {
        let g = res!(glob("a?c"));
        assert!(g.matches(b"abc", false));
        assert!(g.matches(b"a\xffc", false), "one byte, not one character");
        assert!(!g.matches(b"ac", false));
        assert!(!g.matches(b"a/c", false), "never a slash");
        let g = res!(glob("v[0-9].txt"));
        assert!(g.matches(b"v7.txt", false));
        assert!(!g.matches(b"vx.txt", false));
        let g = res!(glob("v[!0-9].txt"));
        assert!(g.matches(b"vx.txt", false));
        assert!(!g.matches(b"v7.txt", false));
        let g = res!(glob("[]x]"));
        assert!(g.matches(b"]", false), "a bracket first in a class is literal");
        assert!(g.matches(b"x", false));
        Ok(())
    }));

    res!(test_it(filter, &["Escapes make wildcards literal 000", "all", "glob"], || {
        let g = res!(glob("a\\*b"));
        assert!(g.matches(b"a*b", false));
        assert!(!g.matches(b"axb", false));
        let g = res!(glob("\\!important"));
        assert!(!g.is_negated());
        assert!(g.matches(b"!important", false));
        let g = res!(glob("odd["));
        assert!(g.matches(b"odd[", false), "an unclosed class is a literal bracket");
        Ok(())
    }));

    res!(test_it(filter, &["Anchoring follows the slashes 000", "all", "glob"], || {
        let g = res!(glob("/target"));
        assert!(g.matches(b"target", false));
        assert!(!g.matches(b"sub/target", false), "a leading slash anchors to the root");
        let g = res!(glob("doc/notes.txt"));
        assert!(g.matches(b"doc/notes.txt", false));
        assert!(!g.matches(b"sub/doc/notes.txt", false));
        let g = res!(glob("notes.txt"));
        assert!(g.matches(b"sub/doc/notes.txt", false), "no slash, so any depth");
        Ok(())
    }));

    res!(test_it(filter, &["A trailing slash means directories only 000", "all", "glob"], || {
        let g = res!(glob("build/"));
        assert!(g.is_dir_only());
        assert!(g.matches(b"build", true));
        assert!(!g.matches(b"build", false), "a file of that name is not matched");
        assert!(g.matches(b"sub/build", true), "unanchored despite the trailing slash");
        Ok(())
    }));

    res!(test_it(filter, &["Double star spans directories 000", "all", "glob"], || {
        let g = res!(glob("**/foo"));
        assert!(g.matches(b"foo", false));
        assert!(g.matches(b"a/b/foo", false));
        let g = res!(glob("abc/**"));
        assert!(g.matches(b"abc/x", false));
        assert!(g.matches(b"abc/x/y", false));
        assert!(!g.matches(b"abc", true), "a trailing double star names the inside");
        let g = res!(glob("a/**/b"));
        assert!(g.matches(b"a/b", false), "zero directories between");
        assert!(g.matches(b"a/x/b", false));
        assert!(g.matches(b"a/x/y/b", false));
        assert!(!g.matches(b"a/xb", false));
        let g = res!(glob("a**b"));
        assert!(g.matches(b"axyb", false));
        assert!(!g.matches(b"a/b", false), "amid other bytes it is an ordinary star");
        Ok(())
    }));

    res!(test_it(filter, &["The last matching rule wins 000", "all", "glob", "ignore"], || {
        let f = IgnoreFile::parse(b"*.log\n!keep.log\n");
        assert_eq!(f.decides(b"debug.log", false), Some(true));
        assert_eq!(f.decides(b"keep.log", false), Some(false), "the negation is later");
        assert_eq!(f.decides(b"keep.txt", false), None, "no rule spoke");
        assert!(!f.ignores(b"keep.log", false));
        let f = IgnoreFile::parse(b"!keep.log\n*.log\n");
        assert!(f.ignores(b"keep.log", false), "order is everything");
        Ok(())
    }));

    res!(test_it(filter, &["An ignored directory swallows its contents 000", "all", "glob", "ignore"], || {
        let f = IgnoreFile::parse(b"target/\n!target/keep.txt\n");
        assert!(f.excludes(b"target", true));
        assert!(f.excludes(b"target/debris.o", false));
        assert!(f.excludes(b"target/keep.txt", false),
            "nothing inside an ignored directory can be re-included");
        assert!(!f.excludes(b"src/lib.rs", false));
        let f = IgnoreFile::parse(b"*.log\n!keep.log\n");
        assert!(!f.excludes(b"logs/keep.log", false),
            "the parent directory is not ignored, so the negation holds");
        Ok(())
    }));

    res!(test_it(filter, &["Comments, blanks and spaces read as git reads them 000", "all", "glob", "ignore"], || {
        let f = IgnoreFile::parse(b"# a comment\n\n\r\n*.tmp   \n\\#hash\nspaced\\ \n");
        assert!(f.ignores(b"a.tmp", false), "trailing spaces are trimmed");
        assert!(f.ignores(b"#hash", false), "an escaped hash is a pattern");
        assert!(f.ignores(b"spaced ", false), "an escaped trailing space is kept");
        assert!(!f.ignores(b"spaced", false));
        assert!(IgnoreFile::parse(b"# only a comment\n\n").is_empty());
        Ok(())
    }));

    res!(test_it(filter, &["Patterns and paths are bytes 000", "all", "glob", "ignore"], || {
        // A path that is not UTF-8 is matched byte for byte.
        let g = res!(glob("*.bin"));
        assert!(g.matches(b"\xff\xfe.bin", false));
        let g = res!(Glob::new(b"?\xff*"));
        assert!(g.matches(b"a\xff\xfe", false));
        assert!(!g.matches(b"a\xfe", false));
        // A pattern that is not UTF-8 compiles and matches likewise.
        let f = IgnoreFile::parse(b"\xff*/\n");
        assert!(f.excludes(b"\xff\xfe/inside.txt", false));
        assert!(!f.excludes(b"\xfe/inside.txt", false));
        Ok(())
    }));

    res!(test_it(filter, &["A pattern with nothing to say is refused 000", "all", "glob"], || {
        assert!(Glob::new(b"").is_err());
        assert!(Glob::new(b"!").is_err());
        assert!(Glob::new(b"/").is_err());
        Ok(())
    }));

    // The list lives upstream of this matcher, so the proof that every line of it is a rule, and
    // that the rules decide what the list's comment says, has to stand here.
    res!(test_it(filter, &["The secret paths are rules, and decide what they say 000", "all",
        "glob", "secret"], ||
    {
        for line in secret::SECRET_PATHS {
            res!(Glob::new(line.as_bytes()));
        }
        let f = IgnoreFile::parse(secret::SECRET_PATHS.join("\n").as_bytes());
        for path in [
            ".env", "app/.env.local", ".env.production",
            "server.pem", "cert/server.key", "store.p12", "store.pfx", "app.jks", "app.keystore",
            "id_rsa", ".ssh/id_ed25519", ".netrc", ".pgpass", ".htpasswd",
            "keys/anything.txt", "deep/tls/chain.txt",
        ] {
            assert!(f.excludes(path.as_bytes(), false), "{} is a secret by name", path);
        }
        for path in [
            ".env.example", ".env.sample", ".env.template", "environment.rs", "envoy.yaml",
            "id_ed25519.pub", "server.pub", "server.crt", "README.md", "src/keys.rs",
            "keys.txt", "tls.rs", "monkeys/banana.txt",
        ] {
            assert!(!f.excludes(path.as_bytes(), false), "{} is not", path);
        }
        assert!(f.excludes(b"keys", true), "the directory itself");
        assert!(!f.excludes(b"keys", false), "and not a file of that name");
        Ok(())
    }));

    res!(test_it(filter, &["Editor droppings are rules, and keep out real editor names 000", "all",
        "glob", "editor"], ||
    {
        for line in EDITOR_DROPPINGS {
            res!(Glob::new(line.as_bytes()));
        }
        let f = IgnoreFile::parse(EDITOR_DROPPINGS.join("\n").as_bytes());
        for path in [
            "ch3.typ.swp", ".ch3.typ.swp", "ch3.typ.swo", "ch3.typ.swx", "4913",
            "#ch3.typ#", ".#ch3.typ", "ch3.typ~", "notes.kate-swp",
        ] {
            assert!(f.excludes(path.as_bytes(), false), "{} is an editor dropping", path);
        }
        for path in [
            "ch3.typ", "src/main.rs", "swp.rs", "4913.rs", "sharpe.txt", "README~ish",
        ] {
            assert!(!f.excludes(path.as_bytes(), false), "{} is not", path);
        }
        // The repository's own rules still win: a `!` line re-includes a dropping by name.
        let mut lines: Vec<&str> = EDITOR_DROPPINGS.to_vec();
        lines.push("!ch3.typ~");
        let f = IgnoreFile::parse(lines.join("\n").as_bytes());
        assert!(!f.excludes(b"ch3.typ~", false), "a repo rule re-included it");
        assert!(f.excludes(b"other.typ~", false), "everything else is still kept out");
        Ok(())
    }));

    res!(test_it(filter, &["Sync droppings keep out transfer names, never a conflict copy 000", "all",
        "glob", "sync"], ||
    {
        for line in SYNC_DROPPINGS {
            res!(Glob::new(line.as_bytes()));
        }
        let f = IgnoreFile::parse(SYNC_DROPPINGS.join("\n").as_bytes());
        // Both spellings of the file Syncthing writes before it renames it into place, at any depth.
        for path in [
            ".syncthing.ch3.typ.tmp", "~syncthing~ch3.typ.tmp",
            "notes/.syncthing.ch3.typ.tmp", "notes/deep/~syncthing~ch3.typ.tmp",
        ] {
            assert!(f.excludes(path.as_bytes(), false), "{} is a transfer file", path);
        }
        for path in [
            // A conflict copy is the other machine's content, and is always kept.
            "ch3.sync-conflict-20261001-123456-ABCDEFG.typ",
            ".ch3.sync-conflict-20261001-123456-ABCDEFG.typ.swp",
            // Folder-root bookkeeping, not a transfer name.
            ".stfolder", ".stversions", ".stignore",
            // Near misses: the whole shape has to hold.
            "ch3.typ", "syncthing.tmp", "my.syncthing.ch3.tmp", ".syncthing.ch3.typ", "~syncthing~ch3.typ",
        ] {
            assert!(!f.excludes(path.as_bytes(), false), "{} is not", path);
        }
        // A repository's rule re-includes one by name, as for any dropping.
        let mut lines: Vec<&str> = SYNC_DROPPINGS.to_vec();
        lines.push("!.syncthing.*.tmp");
        let f = IgnoreFile::parse(lines.join("\n").as_bytes());
        assert!(!f.excludes(b".syncthing.ch3.typ.tmp", false), "a repo rule re-included it");
        assert!(f.excludes(b"~syncthing~ch3.typ.tmp", false), "the other spelling is still kept out");
        Ok(())
    }));

    res!(test_it(filter, &["The droppings lists name files and never a folder 000", "all",
        "glob", "editor", "sync", "files"], ||
    {
        let lists = [EDITOR_DROPPINGS, SYNC_DROPPINGS];
        for list in lists {
            let files = IgnoreFile::parse_files_only(list.join("\n").as_bytes());
            let anyhow = IgnoreFile::parse(list.join("\n").as_bytes());
            // A folder somebody keeps their work in, called as an editor might call a file.
            for dir in [
                "drafts~", ".#cache", "4913", "#auto#", "ch3.typ.swp", "kate.kate-swp",
                ".syncthing.d.tmp", "~syncthing~d.tmp", "notes/deep/drafts~",
            ] {
                let named = list.iter().any(|l| Glob::new(l.as_bytes())
                    .map_or(false, |g| g.matches(dir.as_bytes(), true)));
                if named {
                    // Control: the same lines compiled as an ordinary file do match it.
                    assert!(anyhow.ignores(dir.as_bytes(), true), "{} is matched as a folder by parse", dir);
                }
                assert!(!files.ignores(dir.as_bytes(), true), "{} is a folder and is not matched", dir);
                // And nothing beneath it is excluded for being in it.
                let inside = fmt!("{}/chapter.typ", dir);
                assert!(!files.excludes(inside.as_bytes(), false), "{} is not excluded", inside);
            }
        }
        // The same lines, asked about files, are as they were.
        let f = IgnoreFile::parse_files_only(EDITOR_DROPPINGS.join("\n").as_bytes());
        for path in ["ch3.typ.swp", "4913", "#ch3.typ#", ".#ch3.typ", "ch3.typ~", "drafts~/ch3.typ~"] {
            assert!(f.excludes(path.as_bytes(), false), "{} is still an editor dropping", path);
        }
        // And a repository's `!` line is read after them as before.
        let mut lines: Vec<&str> = EDITOR_DROPPINGS.to_vec();
        lines.push("!ch3.typ~");
        let f = IgnoreFile::parse_files_only(lines.join("\n").as_bytes());
        assert!(!f.excludes(b"ch3.typ~", false), "re-included by name");
        assert!(f.excludes(b"other.typ~", false), "the rest are still kept out");
        Ok(())
    }));

    res!(test_it(filter, &["A files-only rule refuses a folder, whatever the pattern says 000", "all",
        "glob", "files"], ||
    {
        let g = res!(glob("*.tmp"));
        assert!(!g.is_files_only(), "a pattern says nothing of it");
        assert!(g.matches(b"a.tmp", false) && g.matches(b"a.tmp", true), "unrestricted, it matches both");
        let g = g.files_only();
        assert!(g.is_files_only());
        assert!(g.matches(b"a.tmp", false), "a file");
        assert!(!g.matches(b"a.tmp", true), "not a folder of the same name");
        assert!(g.matches(b"sub/deep/a.tmp", false), "at depth");
        // A trailing slash asks for folders alone, and files-only for anything but, so both matches nothing.
        let g = res!(glob("build/")).files_only();
        assert!(!g.matches(b"build", true) && !g.matches(b"build", false), "contradictory, so empty");
        // A file's rules are read in order and the last decides, as ever.
        let f = IgnoreFile::parse_files_only(b"# comment\n*.tmp\n!keep.tmp\n");
        assert!(f.excludes(b"a.tmp", false));
        assert!(!f.excludes(b"keep.tmp", false), "re-included");
        assert!(!f.excludes(b"a.tmp", true), "a folder is not a file");
        assert!(!f.excludes(b"a.tmp/inner", false), "so nothing beneath it is excluded for its sake");
        Ok(())
    }));

    Ok(())
}
