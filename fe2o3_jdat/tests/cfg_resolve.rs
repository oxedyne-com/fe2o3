//! Every reference form a configuration value may hold, as Steel accepted them
//! before its private copy of the resolver was removed (26 Oct 2026, D-20261026-02).
//! `fe2o3_jdat::cfg` is now the one resolver, so each form is held here.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::cfg;

use std::path::{
    Path,
    PathBuf,
};


fn scratch(name: &str) -> Outcome<PathBuf> {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("cfg_resolve").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    res!(std::fs::create_dir_all(&dir));
    Ok(dir)
}

enum Want {
    Is(String), // resolves to exactly this
    Fails,      // refused
}

#[test]
fn every_form_steel_accepts_resolves_as_before() -> Outcome<()> {
    let root = res!(scratch("forms"));
    res!(std::fs::write(root.join("present.key"), "s3cret\n"));
    res!(std::fs::write(root.join("notdir"), "x"));
    res!(std::fs::create_dir_all(root.join("sub")));
    res!(std::fs::write(root.join("sub").join("nested.key"), "  inner  \n"));
    let abs = root.join("present.key");
    let abs = abs.to_string_lossy();

    std::env::set_var("FE2O3_CFG_RESOLVE_SET", "value");
    std::env::set_var("FE2O3_CFG_RESOLVE_KEY", "present.key");
    std::env::set_var("FE2O3_CFG_RESOLVE_DIR", "sub");
    std::env::set_var("FE2O3_CFG_RESOLVE_EMPTY", "");
    std::env::remove_var("FE2O3_CFG_RESOLVE_UNSET");

    let table: Vec<(String, Want)> = vec![
        // Plain text.
        (fmt!("plain"),                                         Want::Is(fmt!("plain"))),
        (fmt!(""),                                              Want::Is(fmt!(""))),
        // Environment.
        (fmt!("{{env:FE2O3_CFG_RESOLVE_SET}}"),                 Want::Is(fmt!("value"))),
        (fmt!("a {{env:FE2O3_CFG_RESOLVE_SET}} b"),             Want::Is(fmt!("a value b"))),
        (fmt!("{{env:FE2O3_CFG_RESOLVE_UNSET:dflt}}"),          Want::Is(fmt!("dflt"))),
        (fmt!("{{env:FE2O3_CFG_RESOLVE_UNSET:}}"),              Want::Is(fmt!(""))),
        (fmt!("{{env:FE2O3_CFG_RESOLVE_UNSET:a:b}}"),           Want::Is(fmt!("a:b"))),
        (fmt!("{{env:FE2O3_CFG_RESOLVE_EMPTY:dflt}}"),          Want::Is(fmt!("dflt"))),
        (fmt!("{{env:FE2O3_CFG_RESOLVE_SET:dflt}}"),            Want::Is(fmt!("value"))),
        (fmt!("{{env:FE2O3_CFG_RESOLVE_UNSET}}"),               Want::Fails),
        (fmt!("{{env:FE2O3_CFG_RESOLVE_EMPTY}}"),               Want::Fails),
        (fmt!("{{env:unclosed"),                                Want::Fails),
        // Files, relative to the root and trimmed.
        (fmt!("{{file:present.key}}"),                          Want::Is(fmt!("s3cret"))),
        (fmt!("{{file:sub/nested.key}}"),                       Want::Is(fmt!("inner"))),
        (fmt!("Bearer {{file:present.key}}"),                   Want::Is(fmt!("Bearer s3cret"))),
        (fmt!("{{file:absent.key}}"),                           Want::Fails),
        (fmt!("{{file:unclosed"),                               Want::Fails),
        // An absolute path replaces the root.
        (fmt!("{{file:{}}}", abs),                              Want::Is(fmt!("s3cret"))),
        // The optional form: absent is empty, any other failure still fails.
        (fmt!("{{file?:present.key}}"),                         Want::Is(fmt!("s3cret"))),
        (fmt!("{{file?:absent.key}}"),                          Want::Is(fmt!(""))),
        (fmt!("{{file?:notdir/child}}"),                        Want::Fails),
        (fmt!("{{file?:unclosed"),                              Want::Fails),
        (fmt!("a{{file?:absent.key}}b{{file:present.key}}c"),   Want::Is(fmt!("abs3cretc"))),
        (fmt!("{{file:present.key}}{{file?:absent.key}}"),      Want::Is(fmt!("s3cret"))),
        // The environment inside a file reference.
        (fmt!("{{file:{{env:FE2O3_CFG_RESOLVE_KEY}}}}"),        Want::Is(fmt!("s3cret"))),
        (fmt!("{{file:{{env:FE2O3_CFG_RESOLVE_DIR}}/nested.key}}"), Want::Is(fmt!("inner"))),
        (fmt!("{{file:{{env:FE2O3_CFG_RESOLVE_UNSET:present.key}}}}"), Want::Is(fmt!("s3cret"))),
        (fmt!("{{file?:{{env:FE2O3_CFG_RESOLVE_UNSET:absent.key}}}}"), Want::Is(fmt!(""))),
        (fmt!("{{file:{{env:FE2O3_CFG_RESOLVE_UNSET}}}}"),      Want::Fails),
    ];

    for (value, want) in &table {
        let got = cfg::resolve(value, &root);
        match (want, got) {
            (Want::Is(w), Ok(g))    => assert_eq!(&g, w, "{:?}", value),
            (Want::Is(w), Err(e))   => panic!("{:?} should resolve to {:?}, failed: {}", value, w, e),
            (Want::Fails, Ok(g))    => panic!("{:?} should be refused, resolved to {:?}", value, g),
            (Want::Fails, Err(_))   => (),
        }
    }

    // A present file the process may not read is refused by the optional form too.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let locked = root.join("locked.key");
        res!(std::fs::write(&locked, "nope\n"));
        res!(std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)));
        if std::fs::read_to_string(&locked).is_err() { // root reads mode 000
            assert!(cfg::resolve("{file?:locked.key}", &root).is_err());
        }
        let _ = std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o600));
    }

    let _ = std::fs::remove_dir_all(&root);
    Ok(())
}

#[test]
fn file_refs_names_every_file_after_env_and_reads_none() -> Outcome<()> {
    let root = Path::new("/nonexistent/root");
    std::env::set_var("FE2O3_CFG_RESOLVE_REFS", "from_env.key");
    let got = res!(cfg::file_refs(
        "a{file:one.key}b{file?:{env:FE2O3_CFG_RESOLVE_REFS}}c{file:/abs/three.key}", root));
    assert_eq!(got, vec![
        root.join("one.key"),
        root.join("from_env.key"),
        PathBuf::from("/abs/three.key"),
    ]);
    assert!(cfg::file_refs("{file:unclosed", root).is_err());
    assert_eq!(res!(cfg::file_refs("plain", root)), Vec::<PathBuf>::new());
    Ok(())
}

#[test]
fn the_app_root_rule_resolves_every_spec() -> Outcome<()> {
    let cwd = res!(std::env::current_dir());
    for spec in ["current", "here", "auto", "."] {
        assert_eq!(res!(cfg::resolve_root(spec, "FE2O3_CFG_ROOT_UNUSED")), cwd, "{}", spec);
    }
    std::env::set_var("FE2O3_CFG_ROOT_SET", "/srv/app");
    assert_eq!(res!(cfg::resolve_root("env", "FE2O3_CFG_ROOT_SET")), PathBuf::from("/srv/app"));
    std::env::remove_var("FE2O3_CFG_ROOT_UNSET");
    assert!(cfg::resolve_root("env", "FE2O3_CFG_ROOT_UNSET").is_err());
    let home = res!(std::env::var("HOME"));
    assert_eq!(res!(cfg::resolve_root("~/apps/x", "V")), PathBuf::from(fmt!("{}/apps/x", home)));
    assert_eq!(res!(cfg::resolve_root("./apps/x", "V")), cwd.join("apps/x"));
    assert_eq!(res!(cfg::resolve_root("/abs/x", "V")), PathBuf::from("/abs/x"));
    assert_eq!(res!(cfg::resolve_root("rel/x", "V")), PathBuf::from("rel/x"));
    Ok(())
}

#[test]
fn a_file_is_inserted_verbatim_and_never_rescanned() -> Outcome<()> {
    let root = res!(scratch("verbatim"));
    let held = "a{file:/etc/passwd}b{env:FE2O3_CFG_RESOLVE_INNER}c{file?:x}{file:unclosed";
    res!(std::fs::write(root.join("held.key"), fmt!("{}\n", held)));
    res!(std::fs::write(root.join("self.key"), "{file:self.key}"));
    std::env::set_var("FE2O3_CFG_RESOLVE_INNER", "expanded");
    assert_eq!(res!(cfg::resolve("{file:held.key}", &root)), held);
    assert_eq!(res!(cfg::resolve("[{file?:held.key}]", &root)), fmt!("[{}]", held));
    // A file naming itself would loop if its contents were scanned again.
    assert_eq!(res!(cfg::resolve("{file:self.key}-{file:self.key}", &root)),
        "{file:self.key}-{file:self.key}");
    let _ = std::fs::remove_dir_all(&root);
    Ok(())
}

// QA gwmain P3-3: an environment value is inserted verbatim, as a file's contents are, and the
// scan goes on after it. A value naming its own variable used to hang start-up, and a value
// holding `{file:...}` was then read as a key file that no path check saw.
#[test]
fn an_env_value_is_inserted_verbatim_and_never_rescanned() -> Outcome<()> {
    let root = res!(scratch("env_verbatim"));
    res!(std::fs::write(root.join("probe.key"), "read"));
    std::env::set_var("FE2O3_CFG_RESOLVE_SELF", "{env:FE2O3_CFG_RESOLVE_SELF}");
    std::env::set_var("FE2O3_CFG_RESOLVE_HOLDS_FILE", "x{file:probe.key}y");
    std::env::set_var("FE2O3_CFG_RESOLVE_HOLDS_ENV", "{env:FE2O3_CFG_RESOLVE_SET2}");
    std::env::set_var("FE2O3_CFG_RESOLVE_SET2", "inner");
    std::env::set_var("FE2O3_CFG_RESOLVE_BRACE", "pro}be.key");
    let (tx, rx) = std::sync::mpsc::channel();
    let r = root.clone();
    std::thread::spawn(move || {
        let _ = tx.send((
            cfg::resolve_env("a{env:FE2O3_CFG_RESOLVE_SELF}b").map_err(|e| fmt!("{}", e)),
            cfg::resolve("<{env:FE2O3_CFG_RESOLVE_SELF}>", &r).map_err(|e| fmt!("{}", e)),
        ));
    });
    let (env_only, whole) = match rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(v)  => v,
        Err(_) => panic!("a self-referring environment value hangs the resolver"),
    };
    assert_eq!(env_only, Ok(fmt!("a{{env:FE2O3_CFG_RESOLVE_SELF}}b")));
    assert_eq!(whole, Ok(fmt!("<{{env:FE2O3_CFG_RESOLVE_SELF}}>")));
    assert_eq!(res!(cfg::resolve_env("{env:FE2O3_CFG_RESOLVE_HOLDS_ENV}")), "{env:FE2O3_CFG_RESOLVE_SET2}");
    // A {file:} inside an environment value is text, not a reference: nothing is read.
    assert_eq!(res!(cfg::resolve("{env:FE2O3_CFG_RESOLVE_HOLDS_FILE}", &root)), "x{file:probe.key}y");
    assert!(res!(cfg::file_refs("{env:FE2O3_CFG_RESOLVE_HOLDS_FILE}", &root)).is_empty());
    // A brace in a variable naming a file does not end the reference early.
    res!(std::fs::write(root.join("pro}be.key"), "braced"));
    assert_eq!(res!(cfg::resolve("{file:{env:FE2O3_CFG_RESOLVE_BRACE}}", &root)), "braced");
    assert_eq!(res!(cfg::file_refs("{file:{env:FE2O3_CFG_RESOLVE_BRACE}}", &root)), vec![root.join("pro}be.key")]);
    let _ = std::fs::remove_dir_all(&root);
    Ok(())
}
