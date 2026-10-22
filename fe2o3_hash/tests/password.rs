use oxedyne_fe2o3_hash::kdf::{
    hash_password,
    verify_password,
    PASSWORD_MEM_COST,
    PASSWORD_TIME_COST,
};

use oxedyne_fe2o3_core::prelude::*;


#[test]
fn password_round_trip_and_wrong_password() -> Outcome<()> {
    let phc = res!(hash_password(b"correct horse battery"));
    let want = fmt!("$argon2id$v=19$m={},t={},p=1$", PASSWORD_MEM_COST, PASSWORD_TIME_COST);
    req!(phc.starts_with(&want), true, "The PHC string '{}' does not start '{}'.", phc, want);
    req!(phc.contains("correct"), false);
    req!(res!(verify_password(b"correct horse battery", &phc)), true);
    req!(res!(verify_password(b"correct horse batterz", &phc)), false);
    req!(res!(verify_password(b"", &phc)), false);
    Ok(())
}

#[test]
fn password_salts_differ() -> Outcome<()> {
    let a = res!(hash_password(b"same password"));
    let b = res!(hash_password(b"same password"));
    req!((a == b), false, "Two hashes of one password share a salt.");
    req!(res!(verify_password(b"same password", &b)), true);
    Ok(())
}

#[test]
fn password_verify_reads_a_foreign_weaker_string() -> Outcome<()> {
    // RFC 9106-style parameters made elsewhere (rust-argon2's own test vector, Argon2i, p=4).
    let phc = "$argon2i$v=19$m=4096,t=3,p=4$YWJjZGVmZ2hpamtsbW5vcHFyc3R1dnd4eXo$BvBk2OaSofBHfbrUW61nHrWB/43xgfs/QJJ5DkMAd8I";
    req!(res!(verify_password(b"foo", phc)), true);
    req!(res!(verify_password(b"bar", phc)), false);
    Ok(())
}

#[test]
fn password_verify_refuses_bad_strings() -> Outcome<()> {
    // A stored string asking 4 GiB is refused, not run.
    let big = "$argon2id$v=19$m=4194304,t=2,p=1$YWJjZGVmZ2hpamtsbW5vcA$BvBk2OaSofBHfbrUW61nHrWB/43xgfs/QJJ5DkMAd8I";
    req!(verify_password(b"x", big).is_err(), true);
    // A four byte salt.
    let short = "$argon2id$v=19$m=4096,t=2,p=1$YWJjZA$BvBk2OaSofBHfbrUW61nHrWB/43xgfs/QJJ5DkMAd8I";
    req!(verify_password(b"x", short).is_err(), true);
    req!(verify_password(b"x", "").is_err(), true);
    req!(verify_password(b"x", "not a hash").is_err(), true);
    Ok(())
}
