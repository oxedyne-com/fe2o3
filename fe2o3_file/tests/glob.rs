//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_file::glob::{
    Glob,
    IgnoreFile,
};

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

    Ok(())
}
