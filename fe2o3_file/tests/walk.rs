//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_file::{
    glob::IgnoreFile,
    walk::{
        Disk,
        Listed,
        Rules,
        Step,
        Walk,
        WalkEnd,
        WalkSource,
        EXCLUDES_NAME,
        git_config_excludes,
        git_global_excludes,
    },
};

use oxedyne_fe2o3_core::{
    prelude::*,
    test::test_it,
};

use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
};


// A tree held in memory: every file path with its content.  Directories are implied by the paths.
struct Mem {
    files:  BTreeMap<Vec<u8>, Vec<u8>>,
    lists:  usize,          // listings asked for
    reads:  Vec<String>,    // ignore files asked for, in order
}

impl Mem {
    fn new(files: &[(&str, &str)]) -> Self {
        Self {
            files:  files.iter().map(|(p, c)| (p.as_bytes().to_vec(), c.as_bytes().to_vec())).collect(),
            lists:  0,
            reads:  Vec::new(),
        }
    }
}

impl WalkSource for Mem {

    fn list(&mut self, dir: &[u8]) -> Outcome<Vec<Listed>> {
        self.lists += 1;
        let mut prefix = dir.to_vec();
        if !prefix.is_empty() {
            prefix.push(b'/');
        }
        let mut out: Vec<Listed> = Vec::new();
        for path in self.files.keys() {
            if !path.starts_with(&prefix) {
                continue;
            }
            let rest = &path[prefix.len()..];
            let (name, is_dir) = match rest.iter().position(|b| *b == b'/') {
                Some(i) => (rest[..i].to_vec(), true),
                None    => (rest.to_vec(), false),
            };
            if !out.iter().any(|e| e.name == name) {
                out.push(Listed::new(name, is_dir));
            }
        }
        Ok(out)
    }

    fn read(&mut self, path: &[u8]) -> Outcome<Option<Vec<u8>>> {
        self.reads.push(String::from_utf8_lossy(path).into_owned());
        Ok(self.files.get(path).cloned())
    }
}

fn walk_all(walk: Walk, src: &mut Mem) -> Outcome<(Vec<String>, WalkEnd)> {
    let mut found = Vec::new();
    let end = res!(walk.run(src, |e| {
        if !e.is_dir {
            found.push(String::from_utf8_lossy(&e.path).into_owned());
        }
        Ok(true)
    }));
    Ok((found, end))
}

pub fn test_walk(filter: &'static str) -> Outcome<()> {

    res!(test_it(filter, &["Defaults keep stores and dependencies out 000", "all", "walk"], || {
        let mut src = Mem::new(&[
            (".git/HEAD",                       ""),
            (".ore/log",                        ""),
            (".claude/worktrees/a/src/x.rs",    ""),
            (".stversions/old.rs",              ""),
            ("web/node_modules/pkg/index.js",   ""),
            ("target/debug/out",                ""),
            ("src/lib.rs",                      ""),
            ("target.txt",                      ""),
        ]);
        let (found, end) = res!(walk_all(Walk::new(1000), &mut src));
        assert_eq!(found, vec!["target.txt", "src/lib.rs"], "a directory's files before its subdirectories");
        assert_eq!(end, WalkEnd::Complete);
        assert_eq!(src.lists, 3, "root, src and web: no ignored directory is ever listed");
        // A repository can take one back.
        let mut src = Mem::new(&[
            (".gitignore",                  "!node_modules/\n"),
            ("node_modules/pkg/index.js",   ""),
        ]);
        let (found, _) = res!(walk_all(Walk::new(1000), &mut src));
        assert_eq!(found, vec![".gitignore", "node_modules/pkg/index.js"]);
        // Or walk everything.
        let mut src = Mem::new(&[(".git/HEAD", "")]);
        let (found, _) = res!(walk_all(Walk::new(1000).with_defaults(IgnoreFile::default()), &mut src));
        assert_eq!(found, vec![".git/HEAD"]);
        Ok(())
    }));

    res!(test_it(filter, &["Nested ignore files speak for their own directory 000", "all", "walk"], || {
        let mut src = Mem::new(&[
            (".gitignore",          "*.log\n/build/\n"),
            ("a.log",               ""),
            ("build/out",           ""),
            ("sub/.gitignore",      "/local.txt\n!keep.log\n"),
            ("sub/keep.log",        ""),
            ("sub/drop.log",        ""),
            ("sub/local.txt",       ""),
            ("sub/build/out",       ""),
            ("sub/deep/local.txt",  ""),
            ("other/local.txt",     ""),
            ("other/keep.log",      ""),
        ]);
        let (found, _) = res!(walk_all(Walk::new(1000), &mut src));
        assert_eq!(found, vec![
            ".gitignore",
            "other/local.txt",      // sub's rules do not reach a sibling
            "sub/.gitignore",
            "sub/keep.log",         // the deeper file's negation outranks the root's *.log
            "sub/build/out",        // /build/ is anchored to the root
            "sub/deep/local.txt",   // /local.txt is anchored to sub
        ]);
        Ok(())
    }));

    res!(test_it(filter, &["Negation, dir-only and the ore file 000", "all", "walk"], || {
        let mut src = Mem::new(&[
            (".gitignore",      "gen/\nnotes\n*.tmp\n"),
            (".oreignore",      "!x.tmp\n"),
            ("gen/a.rs",        ""),
            ("x/gen",           ""),        // a file named like a dir-only pattern
            ("notes/a.md",      ""),
            ("x.tmp",           ""),
            ("y.tmp",           ""),
        ]);
        let (found, _) = res!(walk_all(Walk::new(1000), &mut src));
        assert_eq!(found, vec![".gitignore", ".oreignore", "x.tmp", "x/gen"]);
        // Nothing inside an ignored directory can be re-included, as in git.
        let mut src = Mem::new(&[
            (".gitignore",  "dist/\n!dist/keep.js\n"),
            ("dist/keep.js", ""),
        ]);
        let (found, _) = res!(walk_all(Walk::new(1000), &mut src));
        assert_eq!(found, vec![".gitignore"]);
        Ok(())
    }));

    res!(test_it(filter, &["The budget is reported when it runs out 000", "all", "walk"], || {
        let mut src = Mem::new(&[
            ("a/1", ""), ("a/2", ""), ("b/1", ""), ("c/1", ""), (".git/x", ""),
        ]);
        let end = res!(Walk::new(4).run(&mut src, |_| Ok(true)));
        match end {
            WalkEnd::Budget(rep) => {
                // The root's four entries spend the budget, so the walk stops listing a.
                assert_eq!(rep.budget, 4);
                assert_eq!(rep.kept, 3);
                assert_eq!(rep.ignored, 1, ".git");
                assert_eq!(rep.unlisted, 2, "b and c were kept but never listed");
                assert_eq!(rep.at, b"a".to_vec());
            }
            other => return Err(err!("Expected a budget stop, got {:?}.", other; Test)),
        }
        // A budget the tree fits in completes.
        let end = res!(Walk::new(8).run(&mut src, |_| Ok(true)));
        assert_eq!(end, WalkEnd::Complete);
        // A visitor can halt.
        let end = res!(Walk::new(100).run(&mut src, |_| Ok(false)));
        assert_eq!(end, WalkEnd::Halted);
        Ok(())
    }));

    res!(test_it(filter, &["Driven by hand, out of order is an error 000", "all", "walk"], || {
        let mut walk = Walk::new(10);
        assert_eq!(res!(walk.next()), Step::List(Vec::new()));
        assert!(walk.next().is_err(), "the listing is owed");
        assert!(walk.read(None).is_err(), "no file was asked for");
        res!(walk.listed(vec![Listed::new(".gitignore", false), Listed::new("a", false)]));
        assert_eq!(res!(walk.next()), Step::Read(b".gitignore".to_vec()));
        res!(walk.read(Some(b"a\n".to_vec())));
        match res!(walk.next()) {
            Step::Entry(e) => assert_eq!(e.path, b".gitignore".to_vec()),
            other => return Err(err!("Expected an entry, got {:?}.", other; Test)),
        }
        assert_eq!(res!(walk.next()), Step::Done);
        assert_eq!(res!(walk.next()), Step::Done);
        Ok(())
    }));

    res!(test_it(filter, &["Disk source walks a real tree 000", "all", "walk"], || {
        let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("fe2o3_file_walk");
        let _ = fs::remove_dir_all(&root);
        for (rel, content) in [
            (".gitignore",          "*.o\n"),
            ("src/main.rs",         ""),
            ("src/main.o",          ""),
            ("src/.oreignore",      "secret\n"),
            ("src/secret",          ""),
            (".ore/ops/1",          ""),
            ("node_modules/x/y.js", ""),
        ] {
            let path = root.join(rel);
            if let Some(dir) = path.parent() {
                res!(fs::create_dir_all(dir));
            }
            res!(fs::write(&path, content));
        }
        let mut found = Vec::new();
        let end = res!(Walk::new(1000).run(&mut Disk::new(&root), |e| {
            found.push((String::from_utf8_lossy(&e.path).into_owned(), e.is_dir));
            Ok(true)
        }));
        assert_eq!(end, WalkEnd::Complete);
        assert_eq!(found, vec![
            (".gitignore".to_string(),      false),
            ("src".to_string(),             true),
            ("src/.oreignore".to_string(),  false),
            ("src/main.rs".to_string(),     false),
        ]);
        assert!(Walk::new(10).run(&mut Disk::new(root.join("absent")), |_| Ok(true)).is_err(),
            "a missing root is an error, not an empty walk");
        let _ = fs::remove_dir_all(&root);
        Ok(())
    }));

    res!(test_it(filter, &["A tree with no .git reads only the ignore files it lists 000", "all", "walk"], || {
        let mut src = Mem::new(&[
            (".gitignore",      "*.log\n"),
            ("a.log",           ""),
            ("sub/.oreignore",  "x\n"),
            ("sub/x",           ""),
            ("sub/y",           ""),
        ]);
        let (found, _) = res!(walk_all(Walk::new(1000), &mut src));
        assert_eq!(found, vec![".gitignore", "sub/.oreignore", "sub/y"]);
        assert_eq!(src.reads, vec![".gitignore", "sub/.oreignore"], "no new reads for an existing caller");
        Ok(())
    }));

    res!(test_it(filter, &["Beneath reads every directory above the start, lists none 000", "all", "walk"], || {
        let tree: &[(&str, &str)] = &[
            (".git/HEAD",               ""),
            (".git/info/exclude",       "*.bak\n"),
            (".gitignore",              "/drafts/\n"),
            ("docs/.gitignore",         "*.pdf\n"),
            ("docs/book/.gitignore",    "!keep.pdf\n"),
            ("docs/book/a.md",          ""),
            ("docs/book/a.pdf",         ""),
            ("docs/book/keep.pdf",      ""),
            ("docs/book/x.bak",         ""),
            ("docs/other/x.md",         ""),
            ("drafts/sub/x.md",         ""),
        ];
        let mut src = Mem::new(tree);
        let walk = Walk::new(100).beneath(b"/docs/book/");
        let (found, end) = res!(walk_all(walk, &mut src));
        assert_eq!(found, vec!["docs/book/.gitignore", "docs/book/a.md", "docs/book/keep.pdf"],
            "paths stay relative to the root; the root's info/exclude and docs' *.pdf are heard");
        assert_eq!(end, WalkEnd::Complete);
        assert_eq!(src.lists, 1, "only the start is listed");
        assert_eq!(src.reads, vec![
            ".git", ".git/info/exclude", ".gitignore", ".oreignore",
            "docs/.git", "docs/.git/info/exclude", "docs/.gitignore", "docs/.oreignore",
            "docs/book/.gitignore",
        ]);
        // An ignored start, or one beneath an ignored directory, yields nothing.
        for start in ["drafts", "drafts/sub"] {
            let mut src = Mem::new(tree);
            let mut walk = Walk::new(100).beneath(start);
            loop {
                match res!(walk.next()) {
                    Step::Read(p)   => res!(walk.read(res!(src.read(&p)))),
                    Step::Done      => break,
                    other           => return Err(err!("Expected only reads, got {:?}.", other; Test)),
                }
            }
            assert!(walk.start_ignored(), "{} is ignored", start);
            assert_eq!(src.lists, 0);
            assert!(!src.reads.iter().any(|r| r.starts_with("drafts")), "nothing read inside drafts");
        }
        let mut walk = Walk::new(100).beneath("docs");
        assert!(!walk.start_ignored());
        assert_eq!(res!(walk.next()), Step::Read(b".git".to_vec()));
        // An empty start is the root.
        let mut walk = Walk::new(100).beneath("/");
        assert_eq!(res!(walk.next()), Step::List(Vec::new()));
        Ok(())
    }));

    res!(test_it(filter, &["Info/exclude and global excludes rank where git puts them 000", "all", "walk"], || {
        // Expected from git ls-files -co --exclude-standard, with core.excludesFile = "*.tmp".
        let tree: &[(&str, &str)] = &[
            (".git/HEAD",           ""),
            (".git/info/exclude",   "local.txt\n*.bak\n!keep.bak\n!keep.tmp\n"),
            (".gitignore",          "*.log\n/build/\nsecret/\n"),
            ("a.log",               ""),
            ("build/out",           ""),
            ("global.tmp",          ""),
            ("keep.bak",            ""),
            ("keep.tmp",            ""),
            ("local.txt",           ""),
            ("nestqa/.gitignore",   "secret.txt\n"),
            ("nestqa/ok.txt",       ""),
            ("nestqa/secret.txt",   ""),
            ("other/b.log",         ""),
            ("readme.md",           ""),
            ("secret/key",          ""),
            ("sub/.gitignore",      "!local.txt\n!*.log\n"),
            ("sub/a.log",           ""),
            ("sub/deep/local.txt",  ""),
            ("sub/global.tmp",      ""),
            ("sub/local.txt",       ""),
            ("sub/x.bak",           ""),
            ("x.bak",               ""),
        ];
        let git = [
            ".gitignore",
            "keep.bak",
            "keep.tmp",
            "nestqa/.gitignore",
            "nestqa/ok.txt",
            "readme.md",
            "sub/.gitignore",
            "sub/a.log",
            "sub/deep/local.txt",
            "sub/local.txt",
        ];
        let mut src = Mem::new(tree);
        let mut walk = Walk::new(1000).with_excludes(b"*.tmp\n");
        let mut found = Vec::new();
        loop {
            match res!(walk.next()) {
                Step::List(d)   => res!(walk.listed(res!(src.list(&d)))),
                Step::Read(p)   => res!(walk.read(res!(src.read(&p)))),
                Step::Entry(e)  => if !e.is_dir { found.push(String::from_utf8_lossy(&e.path).into_owned()) },
                other           => { assert_eq!(other, Step::Done); break; }
            }
        }
        found.sort();
        assert_eq!(found, git.to_vec(), "the walk agrees with git");
        assert_eq!(src.reads[0], ".git/info/exclude", "read before the root's .gitignore");
        // The rules agree with the walk on every path, directories included.
        let rules: Rules = walk.rules();
        for (path, _) in tree {
            assert_eq!(rules.ignored(path.as_bytes(), false), !git.contains(path), "{}", path);
        }
        for (dir, ign) in [("build", true), ("secret", true), (".git", true), ("sub", false), ("sub/deep", false)] {
            assert_eq!(rules.ignored(dir.as_bytes(), true), ign, "{}", dir);
        }
        let why = |p: &str| rules.why(p.as_bytes(), false).map(|w| String::from_utf8_lossy(&w).into_owned());
        assert_eq!(why("a.log"),                Some(".gitignore".to_string()));
        assert_eq!(why("x.bak"),                Some(".git/info/exclude".to_string()));
        assert_eq!(why("sub/x.bak"),            Some(".git/info/exclude".to_string()));
        assert_eq!(why("nestqa/secret.txt"),    Some("nestqa/.gitignore".to_string()));
        assert_eq!(why("global.tmp"),           Some(EXCLUDES_NAME.to_string()));
        assert_eq!(why("secret/key"),           Some(".gitignore".to_string()));
        assert_eq!(why(".git/HEAD"),            Some(String::new()), "the built-in defaults");
        assert_eq!(why("sub/a.log"),            None);
        assert_eq!(rules.top_ignored(b"secret/a/b", false), Some(b"secret".to_vec()));
        assert_eq!(rules.top_ignored(b"x.bak", false), Some(b"x.bak".to_vec()));
        assert_eq!(rules.top_ignored(b"sub/a.log", false), None);
        Ok(())
    }));

    res!(test_it(filter, &["Each layer outranks the one below 000", "all", "walk"], || {
        // A nested `!` over the root; info/exclude over excludes; excludes over the defaults.
        let tree: &[(&str, &str)] = &[
            (".git/info/exclude",       "!node_modules/\n!x.tmp\n"),
            (".gitignore",              "*.gen\n"),
            ("a/.gitignore",            "!*.gen\n"),
            ("a/b.gen",                 ""),
            ("c.gen",                   ""),
            ("node_modules/m.js",       ""),
            ("target/t",                ""),
            ("x.tmp",                   ""),
            ("y.tmp",                   ""),
        ];
        let mut src = Mem::new(tree);
        let walk = Walk::new(1000).with_excludes("!target/\n*.tmp\n");
        let (mut found, _) = res!(walk_all(walk, &mut src));
        found.sort();
        assert_eq!(found, vec![".gitignore", "a/.gitignore", "a/b.gen", "node_modules/m.js", "target/t", "x.tmp"]);
        Ok(())
    }));

    res!(test_it(filter, &["A nested repository's info/exclude speaks for it alone 000", "all", "walk"], || {
        let mut src = Mem::new(&[
            ("c.dat",                   ""),
            ("inner/.git/info/exclude", "*.dat\n"),
            ("inner/.gitignore",        "!keep.dat\n"),
            ("inner/a.dat",             ""),
            ("inner/keep.dat",          ""),
            ("inner/b.txt",             ""),
        ]);
        let walk = Walk::new(1000);
        let (mut found, _) = res!(walk_all(walk, &mut src));
        found.sort();
        assert_eq!(found, vec!["c.dat", "inner/.gitignore", "inner/b.txt", "inner/keep.dat"]);
        assert_eq!(src.reads, vec!["inner/.git/info/exclude", "inner/.gitignore"]);
        Ok(())
    }));

    res!(test_it(filter, &["Rules absorb another start's files once 000", "all", "walk"], || {
        let tree: &[(&str, &str)] = &[
            (".gitignore",      "*.log\n"),
            ("a/.gitignore",    "x\n"),
            ("a/x",             ""),
            ("b/.gitignore",    "y\n"),
            ("b/y",             ""),
            ("b/z.log",         ""),
        ];
        let mut walk_a = Walk::new(100).beneath("a");
        let mut walk_b = Walk::new(100).beneath("b");
        let mut src = Mem::new(tree);
        for walk in [&mut walk_a, &mut walk_b] {
            loop {
                match res!(walk.next()) {
                    Step::List(d)   => res!(walk.listed(res!(src.list(&d)))),
                    Step::Read(p)   => res!(walk.read(res!(src.read(&p)))),
                    Step::Entry(_)  => (),
                    _               => break,
                }
            }
        }
        let mut rules = walk_a.rules();
        assert!(!rules.ignored(b"b/y", false), "a's walk never heard b/.gitignore");
        rules.absorb(walk_b.rules());
        rules.absorb(walk_b.rules());
        assert!(rules.ignored(b"a/x", false));
        assert!(rules.ignored(b"b/y", false));
        assert!(rules.ignored(b"b/z.log", false));
        assert_eq!(rules.why(b"b/z.log", false), Some(b".gitignore".to_vec()));
        assert!(!Rules::default().ignored(b"anything", false));
        Ok(())
    }));

    res!(test_it(filter, &["Git config names the global excludes 000", "all", "walk"], || {
        assert_eq!(git_config_excludes(b"[core]\n\texcludesFile = ~/.gitignore_global\n"),
            Some(b"~/.gitignore_global".to_vec()));
        assert_eq!(git_config_excludes(b"[Core]\nExcludesfile = \"/a b/ign\" # note\n"),
            Some(b"/a b/ign".to_vec()));
        assert_eq!(git_config_excludes(b"[core \"x\"]\nexcludesFile = /no\n[user]\nexcludesFile = /no\n"), None);
        assert_eq!(git_config_excludes(b"[core]\nexcludesFile = /one\n[core]\n  excludesfile=/two ; c\n"),
            Some(b"/two".to_vec()));
        assert_eq!(git_config_excludes(b"[core] excludesFile = /same\n"), Some(b"/same".to_vec()));
        assert_eq!(git_config_excludes(b"; [core]\n# excludesFile = /no\n"), None);
        // The environment decides which config is read.
        let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("fe2o3_file_walk_gx");
        let _ = fs::remove_dir_all(&root);
        res!(fs::create_dir_all(root.join("xdg/git")));
        res!(fs::write(root.join("ign"), "*.tmp\n"));
        res!(fs::write(root.join("cfg"), format!("[core]\n\texcludesFile = {}\n", root.join("ign").display())));
        res!(fs::write(root.join("xdg/git/ignore"), "*.xdg\n"));
        let saved: Vec<(&str, Option<std::ffi::OsString>)> = ["GIT_CONFIG_GLOBAL", "XDG_CONFIG_HOME", "HOME"]
            .iter().map(|k| (*k, std::env::var_os(k))).collect();
        std::env::set_var("HOME", root.join("home"));
        std::env::set_var("XDG_CONFIG_HOME", root.join("xdg"));
        std::env::set_var("GIT_CONFIG_GLOBAL", root.join("cfg"));
        let named = git_global_excludes();
        std::env::remove_var("GIT_CONFIG_GLOBAL");
        let default = git_global_excludes();
        std::env::set_var("XDG_CONFIG_HOME", root.join("none"));
        let absent = git_global_excludes();
        for (k, v) in saved {
            match v {
                Some(v) => std::env::set_var(k, v),
                None    => std::env::remove_var(k),
            }
        }
        assert_eq!(res!(named), Some(b"*.tmp\n".to_vec()), "core.excludesFile from GIT_CONFIG_GLOBAL");
        assert_eq!(res!(default), Some(b"*.xdg\n".to_vec()), "XDG git/ignore when unset");
        assert_eq!(res!(absent), None);
        let _ = fs::remove_dir_all(&root);
        Ok(())
    }));

    res!(test_it(filter, &["A .git file's repository excludes are read through its pointer 000", "all", "walk"], || {
        // A linked worktree: its gitdir names the main repository's worktrees entry, whose
        // commondir leads back to the repository, where info/exclude lives.  A submodule: its
        // gitdir is the superproject's modules entry, with no commondir.
        let tree: &[(&str, &str)] = &[
            ("repo/.git/info/exclude",          "*.secret\n"),
            ("repo/.git/worktrees/wt/commondir", "../..\n"),
            ("repo/.git/modules/sub/info/exclude", "*.key\n"),
            ("repo/a.txt",                      ""),
            ("wt/.git",                         "gitdir: ../repo/.git/worktrees/wt\n"),
            ("wt/a.secret",                     ""),
            ("wt/doc/b.secret",                 ""),
            ("wt/doc/b.txt",                    ""),
            ("repo/sub/.git",                   "gitdir: ../.git/modules/sub\r\n"),
            ("repo/sub/k.key",                  ""),
            ("repo/sub/k.txt",                  ""),
        ];
        let mut src = Mem::new(tree);
        let (mut found, end) = res!(walk_all(Walk::new(1000), &mut src));
        found.sort();
        assert_eq!(end, WalkEnd::Complete);
        assert!(!found.iter().any(|f| f.ends_with(".secret") || f.ends_with(".key")),
            "a .git file's info/exclude is in force: {:?}", found);
        assert!(found.contains(&"wt/doc/b.txt".to_string()));
        assert!(found.contains(&"repo/sub/k.txt".to_string()));
        assert!(src.reads.contains(&"repo/.git/worktrees/wt/commondir".to_string()));
        // The same from a start beneath the worktree, where the .git file is never listed.
        let mut src = Mem::new(tree);
        let (found, _) = res!(walk_all(Walk::new(1000).beneath("wt/doc"), &mut src));
        assert_eq!(found, vec!["wt/doc/b.txt"], "read blind above the start");
        // Rules hand the layer back, named by the file that decided.
        let mut src = Mem::new(tree);
        let mut walk = Walk::new(1000);
        loop {
            match res!(walk.next()) {
                Step::List(d)   => res!(walk.listed(res!(src.list(&d)))),
                Step::Read(p)   => res!(walk.read(res!(src.read(&p)))),
                Step::Done      => break,
                _               => (),
            }
        }
        assert_eq!(walk.rules().why(b"wt/x.secret", false), Some(b"repo/.git/info/exclude".to_vec()));
        // A pointer that leaves the root, absolute or by climbing, is reported, not followed.
        for target in ["/home/u/repo/.git/worktrees/wt", "../../elsewhere/.git"] {
            let content = format!("gitdir: {}\n", target);
            let mut src = Mem::new(&[("wt/.git", content.as_str()), ("wt/a.txt", "")]);
            let mut walk = Walk::new(1000);
            let mut beyond = Vec::new();
            loop {
                match res!(walk.next()) {
                    Step::List(d)   => res!(walk.listed(res!(src.list(&d)))),
                    Step::Read(p)   => res!(walk.read(res!(src.read(&p)))),
                    Step::Beyond(p) => beyond.push(p),
                    Step::Done      => break,
                    _               => (),
                }
            }
            assert_eq!(beyond, vec![b"wt/.git".to_vec()], "{}", target);
            assert!(!src.reads.iter().any(|r| r.contains("elsewhere") || r.starts_with('/')), "{:?}", src.reads);
        }
        // So is a commondir that leaves it.
        let mut src = Mem::new(&[
            ("wt/.git",                 "gitdir: ../g\n"),
            ("g/commondir",             "/abs/repo/.git\n"),
            ("wt/a.txt",                ""),
        ]);
        let mut walk = Walk::new(1000);
        let mut beyond = Vec::new();
        loop {
            match res!(walk.next()) {
                Step::List(d)   => res!(walk.listed(res!(src.list(&d)))),
                Step::Read(p)   => res!(walk.read(res!(src.read(&p)))),
                Step::Beyond(p) => beyond.push(p),
                Step::Done      => break,
                _               => (),
            }
        }
        assert_eq!(beyond, vec![b"wt/.git".to_vec()]);
        // A .git file that is no pointer names no repository, as git refuses it.
        let mut src = Mem::new(&[("wt/.git", "not a pointer\n"), ("wt/a.txt", "")]);
        let (found, end) = res!(walk_all(Walk::new(1000), &mut src));
        assert_eq!((found, end), (vec!["wt/.git".to_string(), "wt/a.txt".to_string()], WalkEnd::Complete));
        Ok(())
    }));

    Ok(())
}
