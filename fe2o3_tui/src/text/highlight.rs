//! Highlights are intended to be non-verlapping spans of text within lines that can be styled and
//! navigated using a focus.  They are stored with the text lines themselves and so may need to be
//! updated when the text changes.
//!
use crate::{
    text::{
        typ::{
            HighlightType,
            TextType,
        },
    },
};

use oxedize_fe2o3_core::prelude::*;
use oxedize_fe2o3_text::{
    Text,
    highlight::Highlight,
    lines::LineRange,
};


#[derive(Clone, Debug)]
pub enum HighlightBuilder {
    FullLine,
    IgnoreWhiteSpace,
    Marked(String, String), // Start marker, end marker.
}

impl Default for HighlightBuilder {
    fn default() -> Self {
        Self::FullLine
    }
}

impl HighlightBuilder {

    pub fn build(
        &self,
        pairs: Vec<(Text<TextType>, Option<HighlightType>)>,
    )
        -> Outcome<Vec<Highlight<HighlightType>>>
    {
        let mut result = Vec::new();
        match &self {
            Self::FullLine => {
                for (i, (text, data)) in pairs.into_iter().enumerate() {
                    match text.typ {
                        TextType::Plain     |
                        TextType::MenuItem  => {
                            result.push(Highlight::new(
                                LineRange::new(i, (0, text.len() - 1)),
                                0,
                                data,
                            ));    
                        }
                        _ => {}
                    }
                }
            }
            Self::IgnoreWhiteSpace => {
                for (i, (text, data)) in pairs.into_iter().enumerate() {
                    match text.typ {
                        TextType::Plain     |
                        TextType::MenuItem  => {
                            let trimmed = text.txt.trim();
                            let mut iter = trimmed.chars();
                            // Find the start index of the first non-whitespace character.
                            let start = iter.next().and_then(|first_non_white| {
                                text.txt.find(first_non_white)
                            });

                            // Find the end index of the last non-whitespace character.
                            let end = iter.last().and_then(|last_non_white| {
                                text.txt.rfind(last_non_white)
                            });
                            if let Some(start) = start {
                                if let Some(end) = end {
                                    result.push(Highlight::new(
                                        LineRange::new(i, (start, end + 1)),
                                        0,
                                        data,
                                    ));    
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            Self::Marked(prefix, suffix) => {
                for (i, (text, data)) in pairs.into_iter().enumerate() {
                    let mut prefix_index = 0;
                    while let Some(start) = text.txt[prefix_index..].find(prefix) {
                        let start = prefix_index + start + prefix.len();
                        if let Some(end) = text.txt[start..].find(suffix) {
                            let end = start + end;
                            result.push(Highlight::new(
                                LineRange::new(i, (start, end - start)),
                                0,
                                data.clone(),
                            ));
                            prefix_index = end + suffix.len();
                        } else {
                            result.push(Highlight::new(
                                LineRange::new(i, (start, text.len() - 1)),
                                0,
                                data.clone(),
                            ));
                            break;
                        }
                    }
                    if prefix_index == 0 && !text.is_empty() {
                        result.push(Highlight::new(
                            LineRange::new(i, (0, text.len() - 1)),
                            0,
                            data.clone(),
                        ));
                    }
                }
            }
            //_ => {}
        }
        Ok(result)
    }
}

//#[derive(Clone, Debug, Default)]
//pub struct StyledHighlighter {
//    pub highlighter:    Highlighter<HighlightType>,
//    pub build_typ:      BuilderType,
//    pub styles:         Vec<Style>,
//}
//
//impl StyledHighlighter {
//    pub fn new(
//        build_typ:  BuilderType,
//        styles:     Vec<Style>,
//    )
//        -> Self
//    {
//        Self {
//            build_typ,
//            styles,
//            ..Default::default()
//        }
//    }   
//
//    pub fn is_empty(&self) -> bool {
//        self.highlighter.is_empty()
//    }
//
//    pub fn get_focus(&self) -> usize {
//        self.highlighter.get_focus()
//    }
//
//    pub fn get_highlighted(&self) -> Option<&Highlight<HighlightType>> {
//        self.highlighter.get_highlighted()
//    }
//
//    pub fn get_highlighted_data(&self) -> Option<&HighlightType> {
//        self.highlighter.get_highlighted()
//    }
//
//    pub fn inc_focus(&mut self) {
//        self.highlighter.inc_focus()
//    }
//
//    pub fn dec_focus(&mut self) {
//        self.highlighter.dec_focus()
//    }
//
//}
