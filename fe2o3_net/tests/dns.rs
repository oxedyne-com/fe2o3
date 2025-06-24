use oxedyne_fe2o3_net::{
    dns::Fqdn,
};

use oxedyne_fe2o3_core::{
    prelude::*,
    test::test_it,
};


pub fn test_dns(filter: &'static str) -> Outcome<()> {

    res!(test_it(filter, &["Simple", "all", "dns"], || {

        let dn_255 = &("a".repeat(63) + "." + &"b".repeat(63) + "." + &"b".repeat(63) + "." + &"b".repeat(63));
        let dn_256 = &("a".repeat(63) + "." + &"b".repeat(63) + "." + &"b".repeat(63) + "." + &"b".repeat(64));
        let dn_lab_63 = &("a".repeat(63) + ".com");

        let domains: Vec<(&str, bool)> = vec![
            // Valid domains
            ("example.com", true),              // Simple valid domain
            ("sub.example.com", true),          // Valid subdomain
            ("sub-domain.example.com", true),   // Valid hyphen in label
            ("xn--mnchen-3ya.de", true),        // Valid IDN (München)
            ("a.b.c.d.e.f.g.h.i.j.com", true),  // Many subdomains (within 253 char limit)
            ("123.example.com", true),          // Numbers in subdomain
            ("example-123.com", true),          // Alphanumeric with hyphen
            ("sub.sub.example.co.uk", true),    // Multiple levels with country code
            ("valid.domain-name.org", true),    // Hyphen in second level
            ("a-very-long-domain-name-but-still-under-63-chars.com", true), // Long but valid label
            ("localhost", true),                // Special case - localhost is valid per RFC 6761
            ("localhost.", true),               // Ends with dot is ok
            ("A.B.C", true),                    // Upper case
            ("a1.b2.c3", true),                 // Single char with numbers
            ("1.2.3", true),                    // All numeric labels (but not special)
            ("x.y.z", true),                    // Single letter labels
            (dn_255, true),                     // Exactly 255 bytes total
            (dn_lab_63, true),                  // Exactly 63 bytes in label
            ("test", true),                     // RFC 6761 special
            ("invalid", true),                  // RFC 6761 special
            ("example", true),                  // RFC 6761 special
            ("example.123", true),              // All-numeric TLD, allowed but not used
            ("example.c", true),                // Single char TLD allowed but not used
            // Edge cases for hyphen rules
            ("a--b.example.com", true),         // Multiple consecutive hyphens in middle
            ("123example.com", true),           // Numbers at start
            ("example123.com", true),           // Numbers at end
            ("exa123mple.com", true),           // Numbers in middle
            ("123.456.789.com", true),          // All-numeric non-TLD labels
        
            // Invalid domains
            ("", false),                        // Empty string
            (".", false),                       // Single dot
            (".com", false),                    // Starts with dot
            ("com.", false),                    // Single label with dot
            (".", false),                       // Root only
            ("\u{0000}.com", false),            // Null character
            ("x\u{00A0}y.com", false),          // Non-ASCII space
            ("example..com", false),            // Consecutive dots
            ("-example.com", false),            // Starts with hyphen
            ("example-.com", false),            // Ends with hyphen
            ("exam ple.com", false),            // Contains space
            ("exam@ple.com", false),            // Contains @ symbol
            ("ex*mple.com", false),             // Contains asterisk
            (".example.com.", false),           // Ends with dot
            ("really-long-subdomain-that-exceeds-sixty-three-characters-which-is-not-allowed.com", false), // Label > 63 chars
            (dn_256, false),                    // Exactly 256 bytes total
            ("exa!mple.com", false),            // Contains invalid character
            ("ab", false),                      // Too short overall
            // Edge cases for hyphen rules
            ("-ab.example.com", false),         // Hyphen at start of first label
            ("ab-.example.com", false),         // Hyphen at end of first label
            ("ab.-cd.example.com", false),      // Hyphen at start of middle label
            ("ab.cd-.example.com", false),      // Hyphen at end of middle label
        ];
        for (domain, valid) in domains {
            match Fqdn::new(domain) {
                Ok(_fqdn) => {
                    if !valid {
                        return Err(err!(
                            "Domain name '{}' was incorrectly classed as valid.", domain;
                            Test, Invalid, Input, String));
                    } else {
                        test!("'{}' was correctly classed as valid.", domain);
                    }
                }
                Err(e) => {
                    if valid {
                        return Err(err!(e,
                            "Domain name '{}' was incorrectly classed as invalid.", domain;
                            Test, Invalid, Input, String));
                    } else {
                        test!("'{}' was correctly classed as invalid.", domain);
                    }
                }
            }
        }
        Ok(())
    }));

    Ok(())
}
