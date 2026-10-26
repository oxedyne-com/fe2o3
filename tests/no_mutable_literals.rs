//! The mutable-literal gate over the whole kit (D-20261026-02): every `fe2o3_*` crate in one pass,
//! so a new crate is covered without anyone remembering. It fails on a hard-coded location, host,
//! model id, timing or limit absent from `mutable_literals.jdat`, and on an entry there that is no
//! longer found, so the baseline only shrinks. Regenerate it after a migration with
//! `cargo run -p oxedyne_fe2o3_jdat --bin lint_mutable -- --root . --write`.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::lint;

use std::path::PathBuf;


#[test]
fn test_no_new_mutable_literals() -> Outcome<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let rep = res!(lint::check_tree(&root, &root.join("mutable_literals.jdat")));
    assert!(rep.is_clean(), "\n{}", rep);
    Ok(())
}
