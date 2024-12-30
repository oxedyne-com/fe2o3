use crate::constant;

use oxedize_fe2o3_core::prelude::*;

use std::net::ToSocketAddrs;


new_type!(Fqdn, String, Clone, Debug, Default, PartialEq);

/// Fully Qualified Domain Name
impl Fqdn {

    pub fn new<S: Into<String>>(fqdn: S) -> Outcome<Self> {
        let fqdn = fqdn.into();
        match Self::validate(&fqdn) {
            Ok(()) => Ok(Self(fqdn)),
            Err(e) => Err(e),
        }
    }

    pub fn as_str(&self)    -> &str     { &self.0 }
    pub fn to_string(&self) -> String   { self.0.clone() }

    /// Validates a Fully Qualified Domain Name (FQDN) according to:
    /// - RFC 1035 Domain Name Implementation and Specification (base DNS rules)
    /// - RFC 1123 Internet Host Requirements (allowing digits in first character)
    /// - RFC 6761 Special-Use Domain Names (localhost etc.)
    ///
    /// Implements the "preferred name syntax" from RFC 952/1123 which requires:
    /// - Labels separated by dots, 2-63 bytes each
    /// - Labels use only ASCII alphanumeric chars and hyphens
    /// - Labels don't start or end with hyphens
    /// - Total name length ≤ 255 bytes
    /// - At least two labels (except special cases from RFC 6761)
    pub fn validate(fqdn: &str) -> Outcome<()> {

        if fqdn.is_empty() {
            return Err(err!(
                "'{}': Domain name cannot be empty.", fqdn;
            Invalid, Input, Size));
        }

        // Check for leading dots.
        if fqdn.starts_with('.') {
            return Err(err!(
                "'{}': Domain name must not begin with dots.", fqdn;
            Invalid, Input, String));
        }

        // Handle trailing dots - only single trailing dot is valid.
        if fqdn.ends_with("..") {
            return Err(err!(
                "'{}': Domain name must not end with multiple dots.", fqdn;
            Invalid, Input, String));
        }

        // Check total length - RFC 1035 limits domain names to 255 bytes.
        if fqdn.len() > 255 {
            return Err(err!(
                "'{}': Domain name length of {} exceeds 255 bytes.", fqdn, fqdn.len();
            Invalid, Input, Size));
        }

        // Remove single trailing dot if present for remaining validation
        let name = fqdn.strip_suffix('.').unwrap_or(fqdn);

        // Check for special cases first.
        if constant::SPECIAL_DOMAINS.contains(&name) {
            return Ok(());
        }

        // Split the FQDN into domain labels.
        let labels: Vec<&str> = name.split('.').collect();

        // Check if the FQDN has at least two labels (e.g., "example.com").
        if labels.len() < 2 {
            return Err(err!(
                "'{}': Domain name must have at least one '.'.", fqdn;
            Invalid, Input));
        }

        // Validate each domain label.
        for label in labels {

            // Check if the label is empty (consecutive dots).
            if label.is_empty() {
                return Err(err!(
                    "'{}': Empty domain label.", label;
                Invalid, Input, Missing));
            }

            // Check if the label exceeds the maximum length (63 bytes max).
            if label.len() > 63 {
                return Err(err!(
                    "'{}': Domain name label length of {} exceeds 63.", label, label.len();
                Invalid, Input, Size));
            }

            // Get first and last chars - we've already checked it's not empty.
            let mut chars = label.chars();
            let first_char = match chars.next() {
                Some(c) => c,
                None => return Err(err!(
                    "'{}': Invalid domain label.", label;
                Invalid, Input, String))
            };

            let last_char = match chars.last() {
                Some(c) => c,
                None => first_char
            };

            // Check first character is letter or digit.
            if !first_char.is_ascii_alphanumeric() {
                return Err(err!(
                    "'{}': Domain name label must begin with a letter or digit.", label;
                Invalid, Input, String));
            }

            // Check last character is letter or digit.
            if !last_char.is_ascii_alphanumeric() {
                return Err(err!(
                    "'{}': Domain name label must end with a letter or digit.", label;
                Invalid, Input, String));
            }

            // Check all characters.
            if !label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                return Err(err!(
                    "'{}': Domain name label can only contain ASCII alphanumeric \
                    or '-' characters.", label;
                Invalid, Input, String));
            }
        }

        Ok(())
    }

    /// Attempt to resolve the FQDN to an IP address.
    pub fn exists(&self) -> bool {
        match self.to_socket_addrs() {
            Ok(mut addrs) => addrs.next().is_some(),
            Err(_) => false,
        }
    }
}

