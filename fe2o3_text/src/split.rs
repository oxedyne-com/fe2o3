use crate::{
    phrase::{
        Phrase,
        PhraseMeta,
    },
    string::Quote,
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
    str,
};

use std::collections::HashSet;


#[derive(Clone, Debug)]
pub struct StringSplitter {
    separators:             HashSet<char>,
    quote_protection:       bool,
    keep_protected_quotes:  bool,
    dash_as_hyphen:         bool,
    classify:               bool,
}

/// The default is to, at a minimum, split at spaces.
impl Default for StringSplitter {

    fn default() -> Self {
        let mut seps = HashSet::new();
        seps.insert(' ');
        Self {
            separators:             seps,
            quote_protection:       true,
            keep_protected_quotes:  true,
            dash_as_hyphen:         false,
            classify:               false,
        }
    }

}

impl StringSplitter {

    /// This constructor does not assume that space splitting is required.
    pub fn new() -> Self {
        Self {
            separators:             HashSet::new(),
            quote_protection:       true,
            keep_protected_quotes:  true,
            dash_as_hyphen:         false,
            classify:               false,
        }
    }

    pub fn protect_quotes(mut self) -> Self {
        self.quote_protection = true;
        self
    }
    
    pub fn keep_protected_quotes(mut self) -> Self {
        self.keep_protected_quotes = true;
        self
    }

    pub fn dashes_are_hypens(mut self) -> Self {
        self.dash_as_hyphen = true;
        self
    }

    pub fn classify(mut self) -> Self {
        self.classify = true;
        self
    }

    pub fn add_separators(mut self, seps: Box<[char]>) -> Self {
        for i in 0..seps.len() {
            self.separators.insert(seps[i]);
        }
        self
    }

    pub fn clear_separators(mut self) -> Self {
        self.separators = HashSet::new();
        self
    }

    /// Split unicode string using `StringSplitter` settings.  There are two cases.
    /// Quote protection ON:
    /// While a quote is active, words can only be terminated by the closing quote.
    /// e.g.
    ///   "this is     a test" -> word = "this is     a test"
    ///   ^                  ^
    ///   +- quote active    +- quote inactive
    /// While quotes are inactive, the rules for no quote protection apply...
    ///
    /// Quote protection OFF:
    /// Quotes are treated as normal characters.  That is, words are terminated by the following
    /// characters:
    ///  ' ' - space, discarded
    ///  '-' - hyphen, with the word tagged as the hyphenation of the previous word
    ///  '.', ';', ','
    ///
    // quote_protection ON
    // th" is  " is -> 'th', ' is  ', 'is'
    // quote_protection OFF
    // th" is  " is -> 'th"', 'is', '"', 'is'
    pub fn split(&self, input: &str) -> Vec<Phrase> {
        let mut parts = Vec::new();
        let mut is_part = false;
        let mut is_hyphened = false;
        let mut part = if self.classify {
            Phrase::Classified(String::new(), PhraseMeta::new())
        } else {
            Phrase::Plain(String::new())
        }; 
        let mut i_start = 0;
        let mut i = 0;
        let mut quote: Quote = Quote::None;
        for c in input.chars() {
            i += 1;
            if self.quote_protection {
                match c {
                    '"'  => {
                        if quote != Quote::Single {
                            if quote == Quote::Double {
                                quote = Quote::None;
                            } else {
                                quote = Quote::Double;
                            }
                            if !self.keep_protected_quotes {
                                continue;
                            }
                        }
                    },
                    '\''  => {
                        if quote != Quote::Double {
                            if quote == Quote::Single {
                                quote = Quote::None;
                            } else {
                                quote = Quote::Single;
                            }
                            if !self.keep_protected_quotes {
                                continue;
                            }
                        }
                    },
                    _ => {},
                }
            }
            if self.separators.contains(&c) {
                if quote == Quote::None {
                    if is_part { // end of part, start new part
                        is_part = false;
                        if self.classify {
                            match &mut part {
                                Phrase::Classified(_, meta) => { 
                                    if meta.typ == None {
                                        meta.set_type(c);  
                                    }
                                    meta.len = i - i_start - 1;
                                },
                                _ => {},
                            }
                        }
                        if (self.dash_as_hyphen && c == '-') ||
                            c == '.'
                        {
                            part.push(c);
                            part.inc_len();
                            if c == '-' {
                                is_hyphened = true;
                            }
                        }
                        parts.push(part);
                        part = if self.classify {
                            Phrase::Classified(String::new(), PhraseMeta::new())
                        } else {
                            Phrase::Plain(String::new())
                        }; 
                        i_start = i;
                    } else { // re-start of part
                        i_start += 1;
                    }
                } else {
                    part.push(c);
                }
            } else {
                if !is_part {
                    is_part = true;
                }
                if is_hyphened {
                    part.set_hyphen_left();
                    is_hyphened = false;
                }
                part.push(c);
            }
        }
        if is_part {
            part.set_len(i - i_start - 1);
            parts.push(part);
        }
        parts
    }

}

//#[derive(Debug, Clone, Copy)]
//pub enum StringFormat {
//    Json,
//}
//
//#[derive(Debug, Clone, Copy)]
//pub struct StrFmtCfg<'a> {
//    pub fmt:                StringFormat,
//    pub multiline:          bool,
//    pub prepend_first_line: bool,
//    pub show_kind:          bool,
//    pub indent:             &'a str,
//    pub next:               &'a str,
//    pub end:                &'a str,
//    pub dat_open_bracket:   &'a str,
//    pub dat_close_bracket:  &'a str,
//    pub kind_separator:     &'a str,
//    pub list_open_bracket:  &'a str,
//    pub list_close_bracket: &'a str,
//    pub map_open_bracket:   &'a str,
//    pub map_close_bracket:  &'a str,
//    pub map_separator:      &'a str,
//}
//
//impl<'a> StrFmtCfg<'a> {
//    pub fn flat_json() -> StrFmtCfg<'a> {
//        StrFmtCfg {
//            fmt:                StringFormat::Json,
//            multiline:          false,
//            prepend_first_line: false,
//            show_kind:          true,
//            //indent:             "\t",
//            indent:             "  ",
//            next:               "",
//            end:                ", ",
//            dat_open_bracket:   "(",
//            dat_close_bracket:  ")",
//            kind_separator:     "|",
//            list_open_bracket:  "[",
//            list_close_bracket: "]",
//            map_open_bracket:   "{",
//            map_close_bracket:  "}",
//            map_separator:      ": ",
//        }
//    }
//    pub fn multiline_json() -> StrFmtCfg<'a> {
//        let mut config = StrFmtCfg::flat_json();
//        config.multiline = true;
//        config.prepend_first_line = true;
//        config.next = "\n";
//        config.end = ",\n";
//        config
//    }
//    pub fn show_kind(&mut self, b: bool) {
//        self.show_kind = b;
//    }
//    pub fn prepend_first_line(&mut self, b: bool) {
//        self.prepend_first_line = b;
//    }
//}
