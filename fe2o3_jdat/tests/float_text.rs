//! A decimal read from text becomes the nearest float, as every JSON reader makes it.
//!
//! Dated 2026-09-23: `get_float64` turned a decoded decimal into a float by scaling its digits
//! by a power of ten in floating point, which rounds twice, so `0.3` came back as
//! `0.30000000000000004`.  Found rebuilding a Natural Earth world from GeoJSON, where a
//! coordinate one unit in the last place off rounds to a different metre.

use oxedyne_fe2o3_jdat::{
    prelude::*,
    string::dec::DecoderConfig,
    usr::{
        UsrKind,
        UsrKindCode,
        UsrKindId,
    },
};

use oxedyne_fe2o3_core::prelude::*;

use std::collections::BTreeMap;

#[test]
fn test_a_json_decimal_reads_as_the_nearest_float_00() -> Outcome<()> {
    let text = "[0.3, 115.8571, -31.9535, 0.1, 1.7976931348623157e308, 5e-324, \
        -179.99999999999997, 2.5e3, 12345678901234567890.123]";
    let cfg = DecoderConfig::<BTreeMap<UsrKindCode, UsrKind>, BTreeMap<String, UsrKindId>>::json(None);
    let dat = res!(Dat::decode_string_with_config(text, &cfg));
    let list = match dat {
        Dat::List(v) => v,
        other => return Err(err!("Expected a list, got {:?}.", other.kind(); Mismatch)),
    };
    // The oracle is the standard library's parser, which rounds to nearest as IEEE 754 and
    // every conforming JSON reader do.
    let want: Vec<f64> = res!(text.trim_matches(|c| c == '[' || c == ']')
        .split(',')
        .map(|s| s.trim().parse::<f64>())
        .collect::<Result<Vec<f64>, _>>());
    req!(list.len(), want.len());
    for (d, w) in list.iter().zip(want.iter()) {
        let got = match d.get_float64() {
            Some(f) => f.0,
            None => return Err(err!("{:?} did not read as a float.", d; Mismatch)),
        };
        req!(got.to_bits(), w.to_bits(), "{:?} read as {:e}, not {:e}.", d, got, w);
    }
    Ok(())
}
