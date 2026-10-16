use oxedyne_fe2o3_core::{
    prelude::*,
    error::ErrWire,
};

use std::str::FromStr;


#[test]
fn every_tag_round_trips_by_name() -> Outcome<()> {
    assert!(ErrTag::ALL.len() > 70);
    for (i, tag) in ErrTag::ALL.iter().enumerate() {
        let name = fmt!("{}", tag);
        assert_eq!(name, tag.name());
        let back = res!(ErrTag::from_str(&name));
        assert_eq!(&back, tag);
        assert_eq!(ErrTag::from_name(&name), Some(&ErrTag::ALL[i]));
    }
    Ok(())
}

#[test]
fn an_unknown_tag_name_is_refused_not_a_panic() -> Outcome<()> {
    assert!(ErrTag::from_str("NoSuchTag").is_err());
    assert!(ErrTag::from_str("").is_err());
    assert_eq!(ErrTag::from_name("invalid"), None);
    Ok(())
}

fn chained() -> Error<ErrTag> {
    let inner = err!("the inner fault {}", 7; Input, Missing);
    let mid = Error::Upstream(std::sync::Arc::new(inner), ErrMsg {
        msg:    "the middle frame".to_string(),
        tags:   &[ErrTag::Decode],
    });
    let foreign = std::io::Error::new(std::io::ErrorKind::Other, "a foreign fault");
    let other = err!(foreign, "wrapping io"; IO);
    Error::Collection(vec![Box::new(mid), Box::new(other)])
}

#[test]
fn an_error_keeps_its_text_and_tags_through_the_wire() -> Outcome<()> {
    let e = chained();
    let wire = e.to_wire();
    let back = wire.to_error();
    assert_eq!(back.tags(), e.tags());
    assert_eq!(back.msgs(), e.msgs());
    assert_eq!(back.plain(), e.plain());
    // A second trip changes nothing a reader can see.
    let again = back.to_wire().to_error();
    assert_eq!(again.tags(), e.tags());
    assert_eq!(again.msgs(), e.msgs());
    Ok(())
}

#[test]
fn a_frame_with_many_tags_keeps_them_in_order() -> Outcome<()> {
    let e = err!("three tags"; Wire, Decode, Unknown);
    let back = e.to_wire().to_error();
    assert_eq!(back.tags(), vec![ErrTag::Wire, ErrTag::Decode, ErrTag::Unknown]);
    assert_eq!(back.msgs(), e.msgs());
    Ok(())
}

#[test]
fn an_unknown_tag_on_the_wire_is_kept_as_text() -> Outcome<()> {
    let wire = ErrWire::Frame {
        msg:    "from a newer peer".to_string(),
        tags:   vec!["Decode".to_string(), "Marmalade".to_string()],
        cause:  None,
    };
    let e = wire.to_error();
    assert_eq!(e.tags(), vec![ErrTag::Decode]);
    let plain = e.plain();
    assert!(plain.contains("from a newer peer"), "{}", plain);
    assert!(plain.contains("Marmalade"), "{}", plain);
    Ok(())
}
