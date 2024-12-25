use crate::{
    prelude::*,
    daticle::{
        IterDatValsMut,
    },
};

use oxedize_fe2o3_core::prelude::*;


impl Dat {

    pub fn normalise_string_values(&mut self, tab: &str) -> Outcome<()> {
        let iter = IterDatValsMut::new(self);
        for d in iter {
            if let Dat::Str(s) = d {
                let mut snew = String::new();
                let mut backslash_active = false;
                for c in s.chars() {
                    match c {
                        '\n' | '\r' => continue,
                        '\t' => snew.push_str(&tab),
                        '\\' => if !backslash_active {
                            backslash_active = true;
                        } else {
                            continue;
                        },
                        ' ' if backslash_active => continue,
                        _ => {
                            if backslash_active && c != ' ' {
                                backslash_active = false;
                            }
                            snew.push(c);
                        },
                    }
                }
                *s = snew;
            }
        }
        Ok(())
    }
}
