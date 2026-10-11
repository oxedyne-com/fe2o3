//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_file::{
    glob::IgnoreFile,
    walk::{
        Disk,
        Listed,
        Step,
        Walk,
        WalkEnd,
        WalkSource,
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
    lists:  usize,  // listings asked for
}

impl Mem {
    fn new(files: &[(&str, &str)]) -> Self {
        Self {
            files:  files.iter().map(|(p, c)| (p.as_bytes().to_vec(), c.as_bytes().to_vec())).collect(),
            lists:  0,
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

    Ok(())
}
