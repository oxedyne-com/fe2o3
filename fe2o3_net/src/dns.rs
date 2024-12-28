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

    /// (IETF) document RFC 1035, "Domain Names - Implementation and Specification." Section 2.3.1.
    pub fn validate(fqdn: &str) -> Outcome<()> {
        if fqdn.is_empty() {
            return Err(err!(
                "'{}': Domain name length of {} exceeds 255.", fqdn, fqdn.len();
            Invalid, Input, Size));
        }
        // Split the FQDN into domain labels.
        let labels: Vec<&str> = fqdn.split('.').collect();
        // Check if the FQDN has at least two labels (e.g., "example.com").
        if labels.len() < 2 {
            return Err(err!(
                "'{}': Domain name must have at least one '.'.", fqdn;
            Invalid, Input));
        }
        // Validate each domain label
        for label in labels {
            // Check if the label is empty.
            if label.is_empty() {
                return Err(err!(
                    "'{}': Empty domain label.", label;
                Invalid, Input, Missing));
            }
            // Check if the label exceeds the maximum length (63 characters).
            if label.len() > 63 {
                return Err(err!(
                    "'{}': Domain name label length of {} exceeds 63.", label, label.len();
                Invalid, Input, Size));
            }
            // Check if the label contains only allowed characters.
            if !label.chars().all(|c| c.is_alphanumeric() || c == '-') {
                return Err(err!(
                    "'{}': Domain name label can only contain alphanumeric or '-' characters.",
                    label;
                Invalid, Input, String));
            }
            // Check if the label starts or ends with a hyphen
            if label.starts_with('-') || label.ends_with('-') {
                return Err(err!(
                    "'{}': Domain name label must not begin or end with a '-'.", label;
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

