use oxedize_fe2o3_core::prelude::*;
use oxedize_fe2o3_num::prelude::*;

#[test]
fn test_aint_00() -> Outcome<()> {
    let a = aint!(-42);
    assert_eq!(a.to_string(), String::from("-42"));
    Ok(())
}

//#[test]
//fn test_aintstr_00() -> Outcome<()> {
//    let a = res!(aintstr!("-42"));
//    assert_eq!(a.to_string(), String::from("-42"));
//    Ok(())
//}
