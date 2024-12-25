use oxedize_fe2o3_core::prelude::*;
use oxedize_fe2o3_stds::regions::Country;

use std::{
    convert::TryFrom,
    fmt::{self},
};

pub struct PhoneNumbers;

impl PhoneNumbers {
    
    pub fn country_to_prefixes(c: &Country) -> Outcome<Vec<u16>> {
        match c {
            Country::Australia => Ok(vec![61]),
            _ => Err(err!(errmsg!(
                "No prefix defined for country {:?}.", c,
            ))),
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
            return Err(err!(errmsg!(
                "Trying to interpret an email address from '{}': \
                length is zero.", s,
            ), ErrTag::Decode, ErrTag::String, ErrTag::Invalid, ErrTag::Input));
        }
        let mut at = None;
        for (i, c) in s.chars().enumerate() {
            match c {
                '@' => match at {
                    None => at = Some(i),
                    Some(j) => return Err(err!(errmsg!(
                        "Trying to interpret an email address from '{}': '@' \
                        character found at position {} found previously at \
                        position {}.", s, i, j,
                    ), ErrTag::Decode, ErrTag::String, ErrTag::Invalid, ErrTag::Input)),
                },
                ' ' => return Err(err!(errmsg!(
                    "Trying to interpret an email address from '{}': Space \
                    characters are invalid, space found at position {}.", s, i,
                ), ErrTag::Decode, ErrTag::String, ErrTag::Invalid, ErrTag::Input)),
                _ => (),
            }
        }
        match at {
            None => Err(err!(errmsg!(
                "Trying to interpret an email address from '{}': '@' \
                character not found.", s,
            ), ErrTag::Decode, ErrTag::String, ErrTag::Invalid, ErrTag::Input)),
            Some(i) => {
                let (left, right) = s.split_at(i);
                if left.len() == 0 {
                    return Err(err!(errmsg!(
                        "Trying to interpret an email address from '{}': \
                        local (left) part of email address has length \
                        zero.", s,
                    ), ErrTag::Decode, ErrTag::String, ErrTag::Invalid, ErrTag::Input));
                }
                if right.len() == 0 {
                    return Err(err!(errmsg!(
                        "Trying to interpret an email address from '{}': \
                        domain (right) part of email address has length \
                        zero.", s,
                    ), ErrTag::Decode, ErrTag::String, ErrTag::Invalid, ErrTag::Input));
                }
                let right = &right[1..];
                match right.find('@') {
                    Some(j) => return Err(err!(errmsg!(
                        "Trying to interpret an email address from '{}': \
                        invalid '@' character found at position {} in the \
                        domain part '{}'.", s, j, right,
                    ), ErrTag::Decode, ErrTag::String, ErrTag::Invalid, ErrTag::Input)),
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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OverlayAddress {
    real:    String,
    virt:    String,
}

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
            ContactAddress::Overlay(omail) => write!(f, "ox:{}//{}", omail.real, omail.virt),
        }
    }
}

// FINISHME
impl TryFrom<&str> for ContactAddress {
    type Error = Error<ErrTag>;

    fn try_from(s: &str) -> std::result::Result<Self, Self::Error> {
        let s = s.trim_start();
        if s.starts_with("ox:") {
            let omail = OverlayAddress::default();
            Ok(ContactAddress::Overlay(omail))
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
            return Err(err!(errmsg!(
                "Decoding of address '{}' should produce '{:?}'.", email, expected,
            ), ErrTag::Invalid, ErrTag::Input, ErrTag::Decode, ErrTag::String));
        }
        Ok(())
    }

    #[test]
    fn test_email_address_decoding_01() -> Outcome<()> {
        let email = "test@my@domain";
        let result = EmailAddress::try_from(email);
        if result.is_ok() {
            return Err(err!(errmsg!(
                "Decoding of address '{}' should produce an error.", email,
            ), ErrTag::Invalid, ErrTag::Input, ErrTag::Decode, ErrTag::String));
        }
        Ok(())
    }

    #[test]
    fn test_email_address_decoding_02() -> Outcome<()> {
        let email = "test @my.domain";
        let result = EmailAddress::try_from(email);
        if result.is_ok() {
            return Err(err!(errmsg!(
                "Decoding of address '{}' should produce an error.", email,
            ), ErrTag::Invalid, ErrTag::Input, ErrTag::Decode, ErrTag::String));
        }
        Ok(())
    }
}
