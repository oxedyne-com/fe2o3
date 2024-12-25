use oxedize_fe2o3_core::prelude::*;

use std::str;

#[derive(Clone, Debug, PartialEq)]
pub enum PhraseType {
    Word,
    HyphenRight,
    HyphenLeft,
    EndSentence,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PhraseTag {
    BreakBefore,
}

#[derive(Clone, Debug, Default)]
pub struct PhraseMeta {
    pub typ: Option<PhraseType>,
    pub tag: Option<PhraseTag>,
    pub len: usize, // unicode length
}

impl PhraseMeta {

    pub fn new() -> Self {
        Self {
            ..Default::default()
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn set_type(&mut self, c: char) {
        self.typ = match c {
            ' ' => Some(PhraseType::Word),
            '-' => Some(PhraseType::HyphenRight),
            '.' => Some(PhraseType::EndSentence),
            _ => None,
        }
    }

    //fn get_type(&self) -> Option<PhraseType> {
    //    self.typ.clone()
    //}

    //pub fn val_ref<'a>(&'a self) -> &'a str {
    //    &self.val
    //}

    //pub fn val_clone(&self) -> String {
    //    self.val.clone()
    //}
}

#[derive(Clone, Debug)]
pub enum Phrase {
    Plain(String),
    Classified(String, PhraseMeta),
}

impl Phrase {

    pub fn push(&mut self, c: char) {
        match self {
            Self::Plain(s) => s.push(c),   
            Self::Classified(s, _) => s.push(c),   
        }
    }
    
    pub fn to_val(self) -> String {
        match self {
            Self::Plain(s) => s,   
            Self::Classified(s, _) => s,   
        }
    }

    pub fn val_ref(&self) -> &str {
        match self {
            Self::Plain(ref s) => s,   
            Self::Classified(ref s, _) => s,   
        }
    }

    pub fn get_type(&self) -> Option<PhraseType> {
        match self {
            Self::Plain(_) => None,   
            Self::Classified(_, meta) => meta.typ.clone(),   
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Self::Plain(s) => s.len(),   
            Self::Classified(_, meta) => meta.len,   
        }
    }

    pub fn get_tag(&self) -> Option<PhraseTag> {
        match self {
            Self::Plain(_) => None,   
            Self::Classified(_, meta) => meta.tag.clone(),   
        }
    }

    pub fn inc_len(&mut self) {
        match self {
            Self::Plain(_) => {},   
            Self::Classified(_, meta) => {
                meta.len += 1;
            },
        }
    }

    pub fn set_len(&mut self, len: usize) {
        match self {
            Self::Plain(_) => {},   
            Self::Classified(_, meta) => {
                meta.len = len;
            },
        }
    }

    pub fn set_hyphen_left(&mut self) {
        match self {
            Self::Plain(_) => {},   
            Self::Classified(_, meta) => {
                meta.typ = Some(PhraseType::HyphenLeft);
            },
        }
    }
}
