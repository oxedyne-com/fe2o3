//! External-oracle tests for engineering notation.
//!
//! The expected prefixes and values come from the SI brochure's prefix table
//! and from the IEC binary prefixes, not from the implementation: milli is
//! 10^-3, pico is 10^-12, kilo is 10^3, giga is 10^9, and a kibi is 1024. So
//! 0.005 is 5 milli, 2.8e-10 is 280 pico, and 1024 is 1 kibi. The table stops
//! at atto below and exa above, and a magnitude outside that span has no
//! prefix at all.

use oxedyne_fe2o3_units::{
    scale::{
        Mag,
        Scale,
        ScaleBasis,
    },
    si::SI,
    system::Units,
};

use oxedyne_fe2o3_core::prelude::*;

/// Asserts two floats agree to a relative tolerance.
fn close(a: f64, b: f64) -> bool {
    if b == 0.0 {
        a.abs() < 1.0e-12
    } else {
        ((a - b) / b).abs() < 1.0e-9
    }
}

#[test]
fn test_humanise_decimal_prefixes_01() -> Outcome<()> {
    // 0.005 is 5 x 10^-3, and 10^-3 is milli.
    let h = res!(Mag::one_decimal(0.005, 2)).humanise();
    assert!(close(h.val, 5.0), "got {}", h.val);
    assert_eq!(h.prefix(), "m");
    // 2.8e-10 is 280 x 10^-12, and 10^-12 is pico.
    let h = res!(Mag::one_decimal(2.8e-10, 2)).humanise();
    assert!(close(h.val, 280.0), "got {}", h.val);
    assert_eq!(h.prefix(), "p");
    // 123456 is 123.456 x 10^3, and 10^3 is kilo; three figures keep 123.
    let h = res!(Mag::one_decimal(123456.0, 3)).humanise();
    assert!(close(h.val, 123.0), "got {}", h.val);
    assert_eq!(h.prefix(), "k");
    // 1234 mega is 1.234 giga.
    let h = res!(Mag::mega(1234.0, 4)).humanise();
    assert!(close(h.val, 1.234), "got {}", h.val);
    assert_eq!(h.prefix(), "G");
    Ok(())
}

/// A prefix names a magnitude, so a negative value takes the same one its
/// positive counterpart does and keeps its sign in the value.
#[test]
fn test_humanise_negative_01() -> Outcome<()> {
    let h = res!(Mag::one_decimal(-0.005, 2)).humanise();
    assert!(close(h.val, -5.0), "got {}", h.val);
    assert_eq!(h.prefix(), "m");
    let h = res!(Mag::one_decimal(-123456.0, 3)).humanise();
    assert!(close(h.val, -123.0), "got {}", h.val);
    assert_eq!(h.prefix(), "k");
    let h = res!(Mag::mega(-1234.0, 4)).humanise();
    assert!(close(h.val, -1.234), "got {}", h.val);
    assert_eq!(h.prefix(), "G");
    Ok(())
}

/// A zero has no scale to find, and must come back as a zero with no prefix
/// rather than as a not-a-number.
#[test]
fn test_humanise_zero_01() -> Outcome<()> {
    let h = res!(Mag::one_decimal(0.0, 3)).humanise();
    assert_eq!(h.val, 0.0);
    assert_eq!(h.prefix(), "");
    let h = res!(Mag::milli(0.0, 3)).humanise();
    assert_eq!(h.val, 0.0);
    assert_eq!(h.prefix(), "");
    Ok(())
}

/// The prefix table runs from atto to exa. A magnitude beyond either end is
/// returned unscaled and unprefixed, and above all does not bring the process
/// down.
#[test]
fn test_humanise_beyond_the_prefix_table_01() -> Outcome<()> {
    for v in [1.0e21f64, -1.0e21, 1.0e-21, -1.0e-21, 1.0e30, 1.0e-30] {
        let h = res!(Mag::one_decimal(v, 3)).humanise();
        assert_eq!(h.prefix(), "", "{:e} should have no prefix", v);
        assert!(close(h.val, v), "{:e} came back as {:e}", v, h.val);
    }
    Ok(())
}

/// Every prefix in the decimal table is reachable, and each is a thousandfold
/// step from the last.
#[test]
fn test_humanise_reaches_every_prefix_01() -> Outcome<()> {
    let want = [
        (-18i32, "a"), (-15, "f"), (-12, "p"), (-9, "n"), (-6, "\u{00b5}"),
        (-3, "m"), (0, ""), (3, "k"), (6, "M"), (9, "G"), (12, "T"),
        (15, "P"), (18, "E"),
    ];
    for (exp, prefix) in want {
        let v = match format!("2.5e{}", exp).parse::<f64>() {
            Ok(v)   => v,
            Err(_)  => continue,
        };
        let h = res!(Mag::one_decimal(v, 2)).humanise();
        assert_eq!(h.prefix(), prefix, "2.5e{} should read in {}", exp, prefix);
        assert!(close(h.val, 2.5), "2.5e{} came back as {}", exp, h.val);
    }
    Ok(())
}

/// A kibi is 1024, so 1024 bytes read in binary is 1 KiB.
#[test]
fn test_humanise_binary_prefixes_01() -> Outcome<()> {
    let h = res!(Units::<SI>::bytes(1024.0, 4)).humanise();
    assert!(close(h.val(), 1.0), "got {}", h.val());
    assert_eq!(h.prefix(), "Ki");
    assert_eq!(h.symbol(), "B");
    // A mebi is 1024^2.
    let h = res!(Units::<SI>::bytes(1024.0 * 1024.0, 4)).humanise();
    assert!(close(h.val(), 1.0), "got {}", h.val());
    assert_eq!(h.prefix(), "Mi");
    Ok(())
}

/// A prefix lookup that has no entry reports it, rather than bringing the
/// process down.
#[test]
fn test_dec_exp_lookup_reports_a_miss_01() -> Outcome<()> {
    let dec = Scale::One(ScaleBasis::Decimal);
    assert!(dec.dec_exp_lookup(3).is_ok());
    assert!(dec.dec_exp_lookup(21).is_err());
    assert!(dec.dec_exp_lookup(-21).is_err());
    assert!(dec.dec_exp_lookup(4).is_err());
    let bin = Scale::One(ScaleBasis::Binary);
    assert!(bin.dec_exp_lookup(3).is_ok());
    assert!(bin.dec_exp_lookup(-3).is_err());
    Ok(())
}

/// Normalising puts the leading digit in the units place and reports the
/// decade it came from, for either sign.
#[test]
fn test_normalise_01() -> Outcome<()> {
    let (sig, exp) = res!(Mag::one_decimal(1234.0, 4)).normalise();
    assert!(close(sig, 1.234), "got {}", sig);
    assert_eq!(exp, 3);
    let (sig, exp) = res!(Mag::one_decimal(-1234.0, 4)).normalise();
    assert!(close(sig, -1.234), "got {}", sig);
    assert_eq!(exp, 3);
    // 0.0475 is 4.75 x 10^-2, and two figures round the tie away from zero.
    let (sig, exp) = res!(Mag::one_decimal(-0.0475, 2)).normalise();
    assert!(close(sig, -4.8), "got {}", sig);
    assert_eq!(exp, -2);
    // A zero has no decade.
    let (sig, exp) = res!(Mag::one_decimal(0.0, 3)).normalise();
    assert_eq!(sig, 0.0);
    assert_eq!(exp, 0);
    Ok(())
}
