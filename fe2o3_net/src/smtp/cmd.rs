use crate::{
    email::msg::EmailMessage,
    smtp::codes::SmtpResponseCode,
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
    fmt,
};


#[derive(Clone, Debug, PartialEq)]
pub enum SmtpCommand {
    /// HELO argument is an unvalidated string -- per RFC 5321 §4.1.1.1
    /// the client identifier may be either an FQDN *or* an
    /// address-literal like `[192.168.1.4]` or `[IPv6:::1]`, and even
    /// then a server SHOULD NOT reject the connection on the basis of
    /// the identifier failing to verify.
    Helo(String),
    /// EHLO argument: same rules as `Helo`.
    Ehlo(String),
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
            Self::Helo(domain)          => write!(f, "HELO {}", domain),
            Self::Ehlo(domain)          => write!(f, "EHLO {}", domain),
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
                let rest = s[4..].trim();
                let domain = rest.split_whitespace().next().unwrap_or("");
                if domain.is_empty() {
                    Err(err!(
                        "'{}' invalid: HELO command requires a domain.", s;
                    Invalid, Input, Mismatch))
                } else {
                    Ok(Self::Helo(domain.to_string()))
                }
            }
            "EHLO" => {
                // RFC 5321 §4.1.1.1: argument is a domain *or* an
                // address-literal `[IPv4]` / `[IPv6:...]`. Per
                // §4.1.4 the server SHOULD NOT refuse based on the
                // identifier failing to verify, so we accept any
                // non-empty token as-is.
                let rest = s[4..].trim();
                let domain = rest.split_whitespace().next().unwrap_or("");
                if domain.is_empty() {
                    Err(err!(
                        "'{}' invalid: EHLO command requires a domain.", s;
                    Invalid, Input, Mismatch))
                } else {
                    Ok(Self::Ehlo(domain.to_string()))
                }
            }
            "MAIL" => {
                // RFC 5321 §4.1.1.2: MAIL FROM:<reverse-path> [SP <esmtp-params>]
                // Accept whitespace between FROM: and the angle-bracketed
                // address, and silently ignore any ESMTP extension
                // parameters that follow the closing '>'.
                let after = res!(strip_verb(s, "MAIL"));
                let after_upper = after.to_uppercase();
                if !after_upper.starts_with("FROM:") {
                    return Err(err!(
                        "'{}' invalid: MAIL command requires 'FROM:'.", s;
                    Invalid, Input));
                }
                let after_colon = after[5..].trim_start();
                let addr = res!(parse_path(after_colon));
                Ok(SmtpCommand::MailFrom(addr))
            }
            "RCPT" => {
                // RFC 5321 §4.1.1.3: RCPT TO:<forward-path> [SP <esmtp-params>]
                let after = res!(strip_verb(s, "RCPT"));
                let after_upper = after.to_uppercase();
                if !after_upper.starts_with("TO:") {
                    return Err(err!(
                        "'{}' invalid: RCPT command requires 'TO:'.", s;
                    Invalid, Input));
                }
                let after_colon = after[3..].trim_start();
                let addr = res!(parse_path(after_colon));
                Ok(SmtpCommand::RcptTo(addr))
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
                // AUTH takes a mechanism name plus an optional inline
                // initial response, e.g. `AUTH PLAIN <base64>`. Pass
                // everything after the command verb through verbatim
                // so the session loop can split mech and payload.
                let rest = s[4..].trim_start();
                if rest.is_empty() {
                    Err(err!(
                        "'{}' invalid: AUTH command requires a mechanism.", s;
                    Invalid, Input, Mismatch))
                } else {
                    Ok(SmtpCommand::Auth(rest.to_string()))
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


/// Strip the leading verb (`MAIL`, `RCPT`, ...) from a command line and
/// return the remainder, trimmed of leading whitespace. Used by the
/// `MAIL FROM:` and `RCPT TO:` parsers, which need the raw remainder
/// rather than a whitespace-split view.
fn strip_verb(line: &str, verb: &str) -> Outcome<String> {
    if line.len() < verb.len() {
        return Err(err!(
            "Line '{}' shorter than verb '{}'.", line, verb;
            Invalid, Input));
    }
    let head = &line[..verb.len()];
    if !head.eq_ignore_ascii_case(verb) {
        return Err(err!(
            "Line '{}' does not start with verb '{}'.", line, verb;
            Invalid, Input));
    }
    Ok(line[verb.len()..].trim_start().to_string())
}

/// Parse an SMTP `<reverse-path>` or `<forward-path>` plus optional
/// trailing ESMTP parameters. Accepts both `<addr>` and the empty
/// path `<>`. Anything after the closing '>' is silently ignored
/// (this is where SIZE=, BODY=, ORCPT=, NOTIFY= etc. live).
fn parse_path(s: &str) -> Outcome<String> {
    let s = s.trim_start();
    if !s.starts_with('<') {
        return Err(err!(
            "Path '{}' must begin with '<'.", s;
            Invalid, Input));
    }
    let end = match s.find('>') {
        Some(i) => i,
        None => return Err(err!(
            "Path '{}' has no closing '>'.", s;
            Invalid, Input)),
    };
    Ok(s[1..end].to_string())
}
