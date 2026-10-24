use oxedyne_fe2o3_core::file::{
    app_config_dir_from,
    config_home_from,
    xdg_config_home_from,
    CONFIG_VENDOR,
};

use std::{
    ffi::OsString,
    path::PathBuf,
};


fn os(s: &str) -> Option<OsString> { Some(OsString::from(s)) }

#[test]
fn test_xdg_config_home_from_absolute_only() {
    assert_eq!(xdg_config_home_from(os("/x/cfg")), Some(PathBuf::from("/x/cfg")));
    assert_eq!(xdg_config_home_from(os("rel/cfg")), None);
    assert_eq!(xdg_config_home_from(os("")), None);
    assert_eq!(xdg_config_home_from(None), None);
}

#[test]
fn test_config_home_absolute_xdg_wins() {
    let p = config_home_from(os("/x/cfg"), os("/home/u")).unwrap_or_default();
    assert_eq!(p, PathBuf::from("/x/cfg"));
}

#[test]
fn test_config_home_relative_or_empty_xdg_falls_back_to_home() {
    for xdg in [os("rel/cfg"), os(""), None] {
        let p = config_home_from(xdg, os("/home/u")).unwrap_or_default();
        assert_eq!(p, PathBuf::from("/home/u/.config"));
    }
}

#[test]
fn test_config_home_neither_names_both() {
    for home in [None, os("")] {
        match config_home_from(os("rel"), home) {
            Ok(p)   => panic!("expected an error, got {:?}", p),
            Err(e)  => {
                let s = e.to_string();
                assert!(s.contains("XDG_CONFIG_HOME"), "{}", s);
                assert!(s.contains("HOME"), "{}", s);
            }
        }
    }
}

#[test]
fn test_app_config_dir_from_ends_vendor_app() {
    assert_eq!(CONFIG_VENDOR, "oxedyne");
    let p = app_config_dir_from("ore", os("/x/cfg"), os("/home/u")).unwrap_or_default();
    assert_eq!(p, PathBuf::from("/x/cfg/oxedyne/ore"));
    assert!(p.ends_with("oxedyne/ore"));
    let p = app_config_dir_from("ore", None, os("/home/u")).unwrap_or_default();
    assert_eq!(p, PathBuf::from("/home/u/.config/oxedyne/ore"));
}

#[test]
fn test_app_config_dir_from_refuses_bad_name() {
    for app in ["", "a/b", "..", ".", "../ore", "/abs"] {
        assert!(app_config_dir_from(app, os("/x/cfg"), os("/home/u")).is_err(), "{:?}", app);
    }
}
