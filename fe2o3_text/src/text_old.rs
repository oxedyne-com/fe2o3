use oxedyne_fe2o3_core::prelude::*;

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

#[derive(Default)]
pub struct Text<'a> {
    src: &'a str,
    pub words: Vec<Phrase>,
    widths: Vec<usize>,
}

impl<'a> Text<'a> {

    pub fn new(text: &'a str) -> Self {
        Self {
            src: text,
            ..Default::default()
        }
    }

    ///// Enumerate ordered subsequences.
    //pub fn ordered_subsequences(n: u32) -> Outcome<Vec<Vec<Vec<usize>>>> {
    //    let mut result: Vec<Vec<Vec<usize>>> = Vec::new();
    //    let (partitions, oflow) = 2usize.overflowing_pow(n-1);
    //    if oflow {
    //        return Err(err!(
    //            "Size of sequence, {}, is too large to enumerate ordered \
    //            subsequences.",
    //            n,
    //        ), "numeric", "overflow"));
    //    }
    //    let nu = n as usize;
    //    for k in 0..partitions {
    //        let mut subsequence: Vec<Vec<usize>> = Vec::new();
    //        let mut b = k;
    //        let mut v: Vec<usize> = vec![0];
    //        for j in 1..=nu {
    //            if (j == nu) || (b & 0x01) != 0 {
    //                subsequence.push(v);
    //                v = vec![j];
    //            } else {
    //                v.push(j);
    //            }
    //            b = b >> 1;
    //        }
    //        result.push(subsequence);
    //    }
    //    Ok(result)
    //}
    
    ///// Identify word boundaries and length as tuples (start, end, len) in terms of unicode characters.
    //pub fn optimise(&self, width_first: usize, width_rest: usize) -> Outcome<Vec<Vec<usize>>> {
    //    let mut min_seq: Vec<Vec<usize>> = Vec::new();
    //    let mut min_err = std::usize::MAX;
    //    let mut found_valid_lines = false;
    //    let mut bs = BitString::new(self.len()-1)?;
    //    let mut counter = 0;
    //    'lines: loop {
    //        let lines = bs.ordered_subsequence();
    //        let mut lines_are_valid = true;
    //        let mut first = true;
    //        let mut err_sum = 0;
    //        'line: for line in &lines {
    //            let mut len = 0;
    //            for wi in line {
    //                len += self.0[*wi].len() + 1;
    //            }
    //            len -= 1;
    //            if first {
    //                first = false;
    //                if len > width_first {
    //                    //msg!("first no good");
    //                    lines_are_valid = false;
    //                    break 'line;    
    //                }
    //                let (err, oflow) = (width_first - len).overflowing_pow(2);
    //                if oflow {
    //                    return Err(err!(
    //                        "The line configuration {:?} caused a numerical overflow, \
    //                        most likely because the first line width {} provided is \
    //                        too large",
    //                        line.clone(),
    //                        width_first,
    //                    ), "numeric", "overflow"));
    //                } else {
    //                    err_sum += err; 
    //                    if err_sum > min_err {
    //                        msg!("short circuit break on first line");
    //                        break 'line; // short circuit
    //                    }
    //                }
    //            } else {
    //                if len > width_rest {
    //                    //msg!("rest no good");
    //                    lines_are_valid = false;
    //                    break 'line;    
    //                }
    //                let (err, oflow) = (width_rest - len).overflowing_pow(2);
    //                if oflow {
    //                    return Err(err!(
    //                        "The line configuration {:?} caused a numerical overflow, \
    //                        most likely because the main line width {} provided is \
    //                        too large",
    //                        line.clone(),
    //                        width_rest,
    //                    ), "numeric", "overflow"));
    //                } else {
    //                    err_sum += err; 
    //                    if err_sum > min_err {
    //                        msg!("short circuit break on following line");
    //                        break 'line; // short circuit
    //                    }
    //                }
    //            }
    //            counter += 1;
    //            msg!("counter = {} min_err = {} min_seq = {:?}", counter, min_err, min_seq);
    //        }
    //        if lines_are_valid && err_sum < min_err {
    //            min_err = err_sum;
    //            min_seq = lines;
    //            found_valid_lines = true;
    //            msg!("counter = {} min_err = {}", counter, min_err);
    //        }
    //        if !bs.inc() {
    //            break 'lines;
    //        }
    //    }
    //    if !found_valid_lines {
    //        return Err(err!(
    //            "Could not find a valid set of lines for which the given text will \
    //            fit into the widths specified (first: {}, rest: {}).",
    //            width_first,
    //            width_rest,
    //        ), "input", "invalid"));
    //    }
    //    Ok(min_seq)
    //}

    pub fn into_iter(self) -> std::vec::IntoIter<String> {
        let mut v = Vec::new();
        for word in self.words {
            v.push(word.to_val());
        }
        v.into_iter()
    }

    /// Simply writes the word strings to line strings using the given vectors of word indices.
    pub fn write(&self, word_index: Vec<Vec<usize>>) -> Outcome<Vec<String>> {
        // Error checking?
        let mut lines = Vec::new();
        for line_list in word_index {
            let mut line = String::new();
            let mut first_word = true;
            for wi in line_list {
                let word = &self.words[wi];
                if !first_word {
                    if word.get_type() != Some(PhraseType::HyphenLeft) {
                        line.push(' '); // before word
                    }
                }
                line.push_str(word.val_ref()); 
                first_word = false;
            }
            lines.push(line);
        }
        Ok(lines)
    }

    /// Creates a simple width vector based on a different first line width.
    pub fn set_simple_widths(&mut self, width_first: usize, width_rest: usize) {
        self.widths = vec![width_rest; self.words.len()];
        self.widths[0] = width_first;
    }

    /// Performs simple word wrapping, but allows a calling loop to specify tags for words that,
    /// for example, insert manual line breaks.  Calculates the sum of the square of the errors.
    /// The width vector must first be set correctly.
    pub fn arrange(&self) -> Outcome<(Vec<Vec<usize>>, usize)> {
        if self.widths.len() != self.words.len() {
            return Err(err!(
                "Only {} line widths have been specified, number of widths must match \
                the number of words {}",
                self.widths.len(),
                self.words.len();
            Input, Mismatch));
        }
        let mut len = 0;
        let mut dl: usize;
        let mut lines: Vec<Vec<usize>> = Vec::new();
        let mut line: Vec<usize> = Vec::new();
        let mut err: usize = 0;
        let mut line_count = 0;
        for wi in 0..self.words.len() {
            let word = &self.words[wi];
            let lw = word.len();
            dl = if len == 0 || word.get_type() == Some(PhraseType::HyphenLeft) { 0 } else { 1 };
            if len+lw+dl <= self.widths[line_count] && word.get_tag() != Some(PhraseTag::BreakBefore) {
                line.push(wi);
                len += lw+dl;
            } else {
                lines.push(line);
                line_count += 1;
                line = vec![wi];
                err += (self.widths[line_count]-len).pow(2);
                //msg!("ERR = {}", err);
                len = lw;
            }
        }
        if line.len() > 0 {
            lines.push(line);
            line_count += 1;
            err += (self.widths[line_count]-len).pow(2);
            //msg!("ERR = {}", err);
        }
        Ok((lines, err))
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::string::StringSplitter;

    #[test]
    fn test_text_split_basic() {
        let splitter = StringSplitter::default();
        let words = splitter.split(
        //   0123456
            " This   is a test"
        //          7891111111
        //             0123456
        );
        for word in &words {
            msg!("word = '{}'", word.val_ref());
        }
        assert_eq!(words.len(), 4);
        assert_eq!(words[0].val_ref(), "This");
        assert_eq!(words[1].val_ref(), "is");
        assert_eq!(words[2].val_ref(), "a");
        assert_eq!(words[3].val_ref(), "test");
    }

    #[test]
    fn test_text_split_quote_protection_01() {
        let splitter = StringSplitter::default();
        let words = splitter.split(
            r#" This   ' is ' " a  " test"#
        );
        for word in &words {
            msg!("word = '{}'", word.val_ref());
        }
        assert_eq!(words.len(), 4);
        assert_eq!(words[0].val_ref(), "This");
        assert_eq!(words[1].val_ref(), "' is '");
        assert_eq!(words[2].val_ref(), "\" a  \"");
        assert_eq!(words[3].val_ref(), "test");
    }

    #[test]
    fn test_text_split_quote_protection_02() {
        let splitter = StringSplitter::new().add_separators(Box::new([';']));
        let parts = splitter.split(
            r#" expr1 = " this is; a test "; expr2"#
        );
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].val_ref(), " expr1 = \" this is; a test \"");
        assert_eq!(parts[1].val_ref(), " expr2");
    }

    //#[test]
    //fn test_wordwrap_small_wrap() -> Outcome<()> {
    //    let words = Text::new(" This   is a test");
    //    // 1234567891111111111
    //    //          0123456789
    //    // This is  |
    //    // a test   |
    //    let widths = words.simple_widths(10, 10);
    //    let (lines, _err) = words.wrap(widths)?;
    //    assert_eq!(lines.len(), 2);
    //    assert_eq!(&lines[0], "This is");
    //    assert_eq!(&lines[1], "a test");
    //    // 1234567891111111111
    //    //          0123456789
    //    // This is a|
    //    // test|
    //    let widths = words.simple_widths(10, 5);
    //    let (lines, _err) = words.wrap(widths)?;
    //    assert_eq!(lines.len(), 2);
    //    assert_eq!(&lines[0], "This is a");
    //    assert_eq!(&lines[1], "test");
    //    // 1234567891111111111
    //    //          0123456789
    //    // This |
    //    // is a |
    //    // test |
    //    let widths = words.simple_widths(6, 6);
    //    let (lines, _err) = words.wrap(widths)?;
    //    assert_eq!(lines.len(), 3);
    //    assert_eq!(&lines[0], "This");
    //    assert_eq!(&lines[1], "is a");
    //    assert_eq!(&lines[2], "test");
    //    // 1234567891111111111
    //    //          0123456789
    //    // This |
    //    // is a test|
    //    let widths = words.simple_widths(6, 10);
    //    let (lines, _err) = words.wrap(widths)?;
    //    assert_eq!(lines.len(), 2);
    //    assert_eq!(&lines[0], "This");
    //    assert_eq!(&lines[1], "is a test");
    //    Ok(())
    //}

    //#[test]
    //fn test_text_wordwrap_long_wrap() -> Outcome<()> {
    //    let mut text = Text::new("   When he was nearly thirteen, my brother Jem got his arm badly broken at the elbow. When it healed, and Jem’s fears of never being able to play football were assuaged, he was seldom self-conscious about his injury. His left arm was somewhat shorter than his right; when he stood or walked, the back of his hand was at right angles to his body, his thumb parallel to his thigh. He couldn’t have cared less, so long as he could       pass and punt.");

    //    text = text.split(StringSplitter::new());
    //    text.set_simple_widths(40, 40);
    //    let (arr, err) = text.arrange()?;
    //    let mut arr2 = arr.clone();
    //    let lines = text.write(arr)?;

    //    msg!("{}|","0123456789".repeat(4));
    //    for line in &lines {
    //        msg!("{}|", line);
    //    }
    //    msg!("err = {}", err);

    //    assert_eq!(lines.len(), 12);
    //    assert_eq!(err, 533);
    //    assert_eq!(&lines[00], "When he was nearly thirteen, my brother");
    //    assert_eq!(&lines[01], "Jem got his arm badly broken at the"    );
    //    assert_eq!(&lines[02], "elbow. When it healed, and Jem’s fears" );
    //    assert_eq!(&lines[03], "of never being able to play football"   );
    //    assert_eq!(&lines[04], "were assuaged, he was seldom self-"     );
    //    assert_eq!(&lines[05], "conscious about his injury. His left arm");
    //    assert_eq!(&lines[06], "was somewhat shorter than his right;"   );
    //    assert_eq!(&lines[07], "when he stood or walked, the back of his");
    //    assert_eq!(&lines[08], "hand was at right angles to his body,"  );
    //    assert_eq!(&lines[09], "his thumb parallel to his thigh. He"    );
    //    assert_eq!(&lines[10], "couldn’t have cared less, so long as he");
    //    assert_eq!(&lines[11], "could pass and punt."                   );

    //    // Tag the word "brother" with BreakBefore
    //    let i = arr2[0].len();
    //    text.words[arr2[0][i-1]].tag = Some(WordTag::BreakBefore);
    //    //msg!("{:?}", text.words[arr2[0][i-1]].tag);
    //    let (arr, err) = text.arrange()?;
    //    let mut arr2 = arr.clone();
    //    let lines = text.write(arr)?;

    //    msg!("{}|","0123456789".repeat(4));
    //    for line in &lines {
    //        msg!("{}|", line);
    //    }
    //    msg!("err = {}", err);
    //    
    //    assert_eq!(lines.len(), 12);
    //    assert_eq!(err, 474);
    //    assert_eq!(&lines[00], "When he was nearly thirteen, my"        );
    //    assert_eq!(&lines[01], "brother Jem got his arm badly broken at");
    //    assert_eq!(&lines[02], "the elbow. When it healed, and Jem’s"   );
    //    assert_eq!(&lines[03], "fears of never being able to play"      );
    //    assert_eq!(&lines[04], "football were assuaged, he was seldom"  );
    //    assert_eq!(&lines[05], "self-conscious about his injury. His"   );
    //    assert_eq!(&lines[06], "left arm was somewhat shorter than his" );
    //    assert_eq!(&lines[07], "right; when he stood or walked, the back");
    //    assert_eq!(&lines[08], "of his hand was at right angles to his");
    //    assert_eq!(&lines[09], "body, his thumb parallel to his thigh.");
    //    assert_eq!(&lines[10], "He couldn’t have cared less, so long as");
    //    assert_eq!(&lines[11], "he could pass and punt."                );

    //    // Untag "brother" and tag the previous word "my" with BreakBefore
    //    let i = arr2[0].len();
    //    text.words[arr2[0][i-1]].tag = Some(WordTag::BreakBefore);
    //    text.words[arr2[1][0]].tag = None;
    //    msg!("{:?}", text.words[arr2[0][i-1]].val);
    //    let (arr, err) = text.arrange()?;
    //    let lines = text.write(arr)?;

    //    msg!("{}|","0123456789".repeat(4));
    //    for line in &lines {
    //        msg!("{}|", line);
    //    }
    //    msg!("err = {}", err);

    //    assert_eq!(lines.len(), 12);
    //    assert_eq!(err, 522);
    //    assert_eq!(&lines[00], "When he was nearly thirteen,"           );
    //    assert_eq!(&lines[01], "my brother Jem got his arm badly broken");
    //    assert_eq!(&lines[02], "at the elbow. When it healed, and Jem’s");
    //    assert_eq!(&lines[03], "fears of never being able to play"      );
    //    assert_eq!(&lines[04], "football were assuaged, he was seldom"  );
    //    assert_eq!(&lines[05], "self-conscious about his injury. His"   );
    //    assert_eq!(&lines[06], "left arm was somewhat shorter than his" );
    //    assert_eq!(&lines[07], "right; when he stood or walked, the back");
    //    assert_eq!(&lines[08], "of his hand was at right angles to his" );
    //    assert_eq!(&lines[09], "body, his thumb parallel to his thigh." );
    //    assert_eq!(&lines[10], "He couldn’t have cared less, so long as");
    //    assert_eq!(&lines[11], "he could pass and punt."                );
    //    Ok(())
    //}

    //#[test]
    //fn test_textwrap_long_with_pad() {
    //    let text = "   When he was nearly thirteen, my brother Jem got his arm badly broken at the elbow. When it healed, and Jem’s fears of never being able to play football were assuaged, he was seldom self-conscious about his injury. His left arm was somewhat shorter than his right; when he stood or walked, the back of his hand was at right angles to his body, his thumb parallel to his thigh. He couldn’t have cared less, so long as he could       pass and punt.";
    //    let w = 40;
    //    let lines = textwrap(text.to_string(), w, w, true);
    //    msg!("Original: {}",text);
    //    msg!("Wrapped");
    //    msg!("{}|","-".repeat(w));
    //    for line in &lines {
    //        msg!("{}|", line);
    //    }
    //    assert_eq!(lines.len(), 13);
    //    assert_eq!(lines[00], "When he was nearly thirteen, my         ");
    //    assert_eq!(lines[01], "brother Jem got his arm badly broken at ");
    //    assert_eq!(lines[02], "the elbow. When it healed, and Jem’s    ");
    //    assert_eq!(lines[03], "fears of never being able to play       ");
    //    assert_eq!(lines[04], "football were assuaged, he was seldom   ");
    //    assert_eq!(lines[05], "self-conscious about his injury. His    ");
    //    assert_eq!(lines[06], "left arm was somewhat shorter than his  ");
    //    assert_eq!(lines[07], "right; when he stood or walked, the     ");
    //    assert_eq!(lines[08], "back of his hand was at right angles to ");
    //    assert_eq!(lines[09], "his body, his thumb parallel to his     ");
    //    assert_eq!(lines[10], "thigh. He couldn’t have cared less,     ");
    //    assert_eq!(lines[11], "so long as he could       pass and      ");
    //    assert_eq!(lines[12], "punt."                                   );
    //}
}

