use crate::{
    dns::Fqdn,
    email::msg::EmailMessage,
    smtp::codes::SmtpResponseCode,
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
    fmt,
};


#[derive(Clone, Debug, PartialEq)]
pub enum SmtpCommand {
    Helo(Fqdn),
    Ehlo(Fqdn),
    MailFrom(String),
    RcptTo(String),
    Data,
    Quit,
    Rset,
    Vrfy(String),
    Expn(String),
    Help(Option<String>),
    Noop,
    Auth(String),
    StartTls,
    Email(EmailMessage),
    Response(SmtpResponseCode, String),
}

impl fmt::Display for SmtpCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Helo(fqdn)            => write!(f, "HELO {}", fqdn.as_str()),
            Self::Ehlo(fqdn)            => write!(f, "EHLO {}", fqdn.as_str()),
            Self::MailFrom(address)     => write!(f, "MAIL FROM:<{}>", address),
            Self::RcptTo(address)       => write!(f, "RCPT TO:<{}>", address),
            Self::Data                  => write!(f, "DATA"),
            Self::Quit                  => write!(f, "QUIT"),
            Self::Rset                  => write!(f, "RSET"),
            Self::Vrfy(address)         => write!(f, "VRFY {}", address),
            Self::Expn(address)         => write!(f, "EXPN {}", address),
            Self::Help(Some(topic))     => write!(f, "HELP {}", topic),
            Self::Help(None)            => write!(f, "HELP"),
            Self::Noop                  => write!(f, "NOOP"),
            Self::Auth(mechanism)       => write!(f, "AUTH {}", mechanism),
            Self::StartTls              => write!(f, "STARTTLS"),
            Self::Email(email)          => write!(f, "{}", email),
            Self::Response(code, msg)   => write!(f, "{} {}", code, msg),
        }
    }
}

impl FromStr for SmtpCommand {
    type Err = Error<ErrTag>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split_whitespace().collect();
        if parts.is_empty() {
            return Err(err!("Message is empty."; Invalid, Input, Missing));
        }

        if let Ok(code) = SmtpResponseCode::from_str(parts[0]) {
            let message = parts[1..].join(" ");
            return Ok(SmtpCommand::Response(code, message));
        }

        let cmd = parts[0].to_uppercase();

        match cmd.as_str() {
            "HELO" => {
                if parts.len() != 2 {
                    Err(err!(
                        "'{}' invalid: {} command requires a single address.", s, cmd;
                    Invalid, Input, Mismatch))
                } else {
                    let fqdn = parts[1].trim();
                    Ok(Self::Helo(res!(Fqdn::new(fqdn.to_string()))))
                }
            }
            "EHLO" => {
                if parts.len() != 2 {
                    Err(err!(
                        "'{}' invalid: {} command requires a single address.", s, cmd;
                    Invalid, Input, Mismatch))
                } else {
                    let fqdn = parts[1].trim();
                    Ok(Self::Ehlo(res!(Fqdn::new(fqdn.to_string()))))
                }
            }
            "MAIL" => {
                if parts.len() != 2 || !parts[1].to_uppercase().starts_with("FROM:") {
                    Err(err!(
                        "'{}' invalid: {} command requires 'FROM:' followed by an address.", s, cmd;
                    Invalid, Input))
                } else {
                    let address = parts[1].trim_start_matches("FROM:<").trim_end_matches('>').to_string();
                    Ok(SmtpCommand::MailFrom(address))
                }
            }
            "RCPT" => {
                if parts.len() != 2 || !parts[1].to_uppercase().starts_with("TO:") {
                    Err(err!(
                        "'{}' invalid: {} command requires 'FROM:' followed by an address.", s, cmd;
                    Invalid, Input))
                } else {
                    let address = parts[1].trim_start_matches("TO:<").trim_end_matches('>').to_string();
                    Ok(SmtpCommand::RcptTo(address))
                }
            }
            "DATA" => {
                if parts.len() != 1 {
                    Err(err!(
                        "'{}' invalid: {} command should have no additional arguments.", s, cmd;
                    Invalid, Input, Excessive))
                } else {
                    Ok(SmtpCommand::Data)
                }
            }
            "QUIT" => {
                if parts.len() != 1 {
                    Err(err!(
                        "'{}' invalid: {} command should have no additional arguments.", s, cmd;
                    Invalid, Input, Excessive))
                } else {
                    Ok(SmtpCommand::Quit)
                }
            }
            "RSET" => {
                if parts.len() != 1 {
                    Err(err!(
                        "'{}' invalid: {} command should have no additional arguments.", s, cmd;
                    Invalid, Input, Excessive))
                } else {
                    Ok(SmtpCommand::Rset)
                }
            }
            "VRFY" => {
                if parts.len() != 2 {
                    Err(err!(
                        "'{}' invalid: {} command requires a single argument.", s, cmd;
                    Invalid, Input, Mismatch))
                } else {
                    let address = parts[1].trim().to_string();
                    Ok(SmtpCommand::Vrfy(address))
                }
            }
            "EXPN" => {
                if parts.len() != 2 {
                    Err(err!(
                        "'{}' invalid: {} command requires a single argument.", s, cmd;
                    Invalid, Input, Mismatch))
                } else {
                    let address = parts[1].trim().to_string();
                    Ok(SmtpCommand::Expn(address))
                }
            }
            "HELP" => {
                if parts.len() == 1 {
                    Ok(SmtpCommand::Help(None))
                } else if parts.len() == 2 {
                    let topic = parts[1].trim().to_string();
                    Ok(SmtpCommand::Help(Some(topic)))
                } else {
                    Err(err!(
                        "'{}' invalid: {} command allows at most one argument.", s, cmd;
                    Invalid, Input, Excessive))
                }
            }
            "NOOP" => {
                if parts.len() != 1 {
                    Err(err!(
                        "'{}' invalid: {} command should have no additional arguments.", s, cmd;
                    Invalid, Input, Excessive))
                } else {
                    Ok(SmtpCommand::Noop)
                }
            }
            "AUTH" => {
                if parts.len() != 2 {
                    Err(err!(
                        "'{}' invalid: {} command requires a single argument.", s, cmd;
                    Invalid, Input, Mismatch))
                } else {
                    let mechanism = parts[1].trim().to_string();
                    Ok(SmtpCommand::Auth(mechanism))
                }
            }
            "STARTTLS" => {
                if parts.len() != 1 {
                    Err(err!(
                        "'{}' invalid: {} command should have no additional arguments.", s, cmd;
                    Invalid, Input, Excessive))
                } else {
                    Ok(SmtpCommand::StartTls)
                }
            }
            _ => Err(err!("Unrecognised command in '{}'.", s; Unknown, Input)),
        }
    }
}
