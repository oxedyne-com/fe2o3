use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_stds::regions::Country;

use std::{
    convert::TryFrom,
    fmt::{self},
};

pub struct PhoneNumbers;

impl PhoneNumbers {
    
    pub fn country_to_prefixes(c: &Country) -> Outcome<Vec<u16>> {
        match c {
            Country::Australia => Ok(vec![61]),
            _ => Err(err!(
                "No prefix defined for country {:?}.", c;
            Invalid, Input, Missing)),
        }
    }

    pub fn prefix_to_country(p: u16) -> Option<Country> {
        match p {
            61 => Some(Country::Australia),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PhoneNumber {
    pub prefix: u16,
    pub num:    String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EmailAddress {
    pub loc:    String,
    pub dom:    String,
}

impl TryFrom<&str> for EmailAddress {
    type Error = Error<ErrTag>;

    fn try_from(s: &str) -> std::result::Result<Self, Self::Error> {
        if s.len() == 0 {
            return Err(err!(
                "Trying to interpret an email address from '{}': \
                length is zero.", s;
            Decode, String, Invalid, Input));
        }
        let mut at = None;
        for (i, c) in s.chars().enumerate() {
            match c {
                '@' => match at {
                    None => at = Some(i),
                    Some(j) => return Err(err!(
                        "Trying to interpret an email address from '{}': '@' \
                        character found at position {} found previously at \
                        position {}.", s, i, j;
                    Decode, String, Invalid, Input)),
                },
                ' ' => return Err(err!(
                    "Trying to interpret an email address from '{}': Space \
                    characters are invalid, space found at position {}.", s, i;
                Decode, String, Invalid, Input)),
                _ => (),
            }
        }
        match at {
            None => Err(err!(
                "Trying to interpret an email address from '{}': '@' \
                character not found.", s;
            Decode, String, Invalid, Input)),
            Some(i) => {
                let (left, right) = s.split_at(i);
                if left.len() == 0 {
                    return Err(err!(
                        "Trying to interpret an email address from '{}': \
                        local (left) part of email address has length \
                        zero.", s;
                    Decode, String, Invalid, Input));
                }
                if right.len() == 0 {
                    return Err(err!(
                        "Trying to interpret an email address from '{}': \
                        domain (right) part of email address has length \
                        zero.", s;
                    Decode, String, Invalid, Input));
                }
                let right = &right[1..];
                match right.find('@') {
                    Some(j) => return Err(err!(
                        "Trying to interpret an email address from '{}': \
                        invalid '@' character found at position {} in the \
                        domain part '{}'.", s, j, right;
                    Decode, String, Invalid, Input)),
                    None => (),
                }
                Ok(EmailAddress {
                    loc: left.to_string(),
                    dom: right.to_string(),
                })
            },
        }
    }
}

/// An address on a Hematite-native overlay identity layer.
///
/// `real` is the underlying backing identity (whatever the overlay
/// treats as the host or account behind the address) and `virt` is
/// the virtual face presented to correspondents. The wire form is
/// `overlay:<real>//<virt>`. Left as a placeholder type until the
/// overlay's address model is nailed down by downstream consumers.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OverlayAddress {
    real:    String,
    virt:    String,
}

/// One of the several address shapes a contact can be reached at:
/// phone, email, or a Hematite-native overlay address.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContactAddress {
    Phone(PhoneNumber),
    Email(EmailAddress),
    Overlay(OverlayAddress),
}

impl fmt::Display for ContactAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContactAddress::Phone(pn) => write!(f, "+{} {}", pn.prefix, pn.num),
            ContactAddress::Email(email) => write!(f, "{}@{}", email.loc, email.dom),
            ContactAddress::Overlay(addr) => write!(f, "overlay:{}//{}", addr.real, addr.virt),
        }
    }
}

// FINISHME
impl TryFrom<&str> for ContactAddress {
    type Error = Error<ErrTag>;

    fn try_from(s: &str) -> std::result::Result<Self, Self::Error> {
        let s = s.trim_start();
        if s.starts_with("overlay:") {
            let addr = OverlayAddress::default();
            Ok(ContactAddress::Overlay(addr))
        } else if s.contains('@') {
            let email = res!(EmailAddress::try_from(s));
            Ok(ContactAddress::Email(email))
        } else {
            let pn = PhoneNumber::default();
            Ok(ContactAddress::Phone(pn))
        }
    }

}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_email_address_decoding_00() -> Outcome<()> {
        let email = "test@my.domain";
        let addr = res!(EmailAddress::try_from(email));
        let expected = EmailAddress { loc: fmt!("test"), dom: fmt!("my.domain") };
        if addr != expected {
            return Err(err!(
                "Decoding of address '{}' should produce '{:?}'.", email, expected;
            Invalid, Input, Decode, String));
        }
        Ok(())
    }

    #[test]
    fn test_email_address_decoding_01() -> Outcome<()> {
        let email = "test@my@domain";
        let result = EmailAddress::try_from(email);
        if result.is_ok() {
            return Err(err!(
                "Decoding of address '{}' should produce an error.", email;
            Invalid, Input, Decode, String));
        }
        Ok(())
    }

    #[test]
    fn test_email_address_decoding_02() -> Outcome<()> {
        let email = "test @my.domain";
        let result = EmailAddress::try_from(email);
        if result.is_ok() {
            return Err(err!(
                "Decoding of address '{}' should produce an error.", email;
            Invalid, Input, Decode, String));
        }
        Ok(())
    }
}
