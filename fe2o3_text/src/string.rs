use oxedize_fe2o3_core::prelude::*;

use std::fmt;


#[derive(Clone, Debug, PartialEq)]
pub enum Quote {
    Single,
    Double,
    None,
}

impl Default for Quote {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Clone, Debug, Default)]
pub struct Indenter {
    level: usize,
}

impl Indenter {
    const INDENT_LIMIT: usize = 50;

    pub fn new() -> Self {
        Self::default()
    }

    pub fn level(&self) -> usize {
        self.level
    }
    
    pub fn inc(&mut self) {
        if self.level < Self::INDENT_LIMIT {
            self.level += 1;
        }
    }
    
    pub fn dec(&mut self) {
        if self.level > 0 {
            self.level -= 1;
        }
    }

    pub fn plus(&self, n: usize) -> usize {
        let new_level = self.level + n;
        if new_level <= Self::INDENT_LIMIT {
            new_level
        } else {
            Self::INDENT_LIMIT
        }
    }
    
    pub fn minus(&self, n: usize) -> usize {
        if self.level >= n {
            self.level - n
        } else {
            0
        }
    }
}

new_type!(Stringer, String, Clone, Debug, Eq, Ord, PartialEq, PartialOrd);

impl fmt::Display for Stringer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Stringer {

    pub fn new<S: Into<String>>(s: S) -> Self {
        Self(s.into())
    }

    pub fn inner(self) -> String { self.0 }

    /// Takes a one-line string representation containing maps and lists and creates a multi-line
    /// representation with indentations using the given tab string.  Assumes the incoming strings has
    /// no no newlines, and matching brackets.  This can be used with the `Debug` representation of a
    /// Rust struct.  A "no look ahead" policy is used for simplicity and speed.
    pub fn to_lines<'a>(&self, tab: &'a str) -> Vec<String> {
        let s = &self.0;
        let mut lines = Vec::new();
        let mut indenter = Indenter::new();
        let mut line = String::new();
        let mut prev_char: Option<char> = None;
        let mut printed_char: bool;
        let mut quote_protection: Quote = Quote::None;
        let mut backslash_active: bool = false;
        for c in s.chars() {
            match c {
                '"' => {
                    if quote_protection != Quote::Single {
                        if quote_protection == Quote::Double {
                            quote_protection = Quote::None;
                        } else {
                            quote_protection = Quote::Double;
                        }
                    }
                },
                '\'' => {
                    if quote_protection != Quote::Double {
                        if quote_protection == Quote::Single {
                            quote_protection = Quote::None;
                        } else {
                            quote_protection = Quote::Single;
                        }
                    }
                },
                _ => (),
            }
            if quote_protection != Quote::None {
                if c == '\\' {
                    backslash_active = true;
                    line.push(c);
                    lines.push(line);
                    line = String::new();
                    line.push_str(&tab.repeat(indenter.plus(1)));
                    continue;
                }
                match c {
                    ' ' | '\t' | '\n' | '\\' if backslash_active => continue,
                    _ => backslash_active = false,
                }
                let opening_quote = (quote_protection == Quote::Single && c == '\'')
                    || (quote_protection == Quote::Double && c == '"' );
                if !opening_quote {
                    // If we are in the midst of a quoted string, print the character and continue.
                    // Otherwise, we want the opening quote to respond like other characters to important
                    // boundaries like ','.
                    line.push(c);
                    continue;
                }
            }
            if let Some(p) = prev_char {
                // Ignore single space after these characters.
                if c == ' ' && (p == ',' || p == '[' || p == '{')  {
                    continue;
                }
            }
            printed_char = false;
    
            // Indentation.
            match prev_char {
                Some('[') if c == ']' => {
                    line.push(c);
                    printed_char = true;
                },
                Some('{') if c == '}' => {
                    line.push(c);
                    printed_char = true;
                },
                Some(',') | Some('[') | Some('{') => {
                    if quote_protection == Quote::None
                        || (quote_protection == Quote::Single && c == '\'')
                        || (quote_protection == Quote::Double && c == '"' )
                    {
                        lines.push(line);
                        line = String::new();
                        if prev_char != Some(',') {
                            // If the previous character is an opening bracket, we increment the
                            // indent.
                            indenter.inc();
                        } else if c == ']' || c == '}' {
                            // The previous character is a ',' and the current character is a closing
                            // bracket, so decrement the indent.
                            indenter.dec();
                        }
                        line.push_str(&tab.repeat(indenter.level()));
                        line.push(c);
                        printed_char = true;
                    }
                },
                _ => match c {
                    ']' | '}' if quote_protection == Quote::None => {
                        lines.push(line);
                        line = String::new();
                        indenter.dec();
                        line.push_str(&tab.repeat(indenter.level()));
                        line.push(c);
                        printed_char = true;
                    },
                    _ => (),
                },
            } 
            if !printed_char {
                if line.len() == 0 {
                    line.push_str(&tab.repeat(indenter.level()));
                }      
                line.push(c);
            }
            prev_char = Some(c);
        }
        if line.len() > 0 {
            lines.push(line);
        }
        lines
    }

    pub fn wrap_lines(&self, width: usize, prefix: Option<&str>) -> Vec<String> {

        let lines = self.0.lines().map(String::from);
        let mut wrapped_lines = Vec::new();
    
        let prefix_width = prefix.map(|p| p.chars().count()).unwrap_or(0);

        for line in lines {
            if line.chars().count() <= width {
                wrapped_lines.push(line);
            } else {
                let mut current_line = String::new();
                let mut current_width = 0;
                let mut first_word = true;

                for word in line.split_whitespace() {
                    let word_width = word.chars().count();

                    if current_width + word_width + 1 > width {
                        if !current_line.is_empty() {
                            wrapped_lines.push(current_line.trim_end().to_string());
                            current_line = prefix.map(|p| p.to_string()).unwrap_or_default();
                            current_width = prefix_width;
                            first_word = true;
                        }
                    }

                    if current_width + word_width <= width {
                        if !first_word || current_width > prefix_width {
                            current_line.push(' ');
                            current_width += 1;
                        }
                        current_line.push_str(word);
                        current_width += word_width;
                        first_word = false;
                    } else {
                        let line_with_prefix = prefix
                            .map(|p| fmt!("{}{}", p, word))
                            .unwrap_or(word.to_string());
                        wrapped_lines.push(line_with_prefix);
                        current_line = prefix.map(|p| p.to_string()).unwrap_or_default();
                        current_width = prefix_width;
                        first_word = true;
                    }
                }

                if !current_line.is_empty() {
                    wrapped_lines.push(current_line.trim_end().to_string());
                }
            }
        }
    
        wrapped_lines
    }

    pub fn trim_newline(&mut self) {
        let s = &mut self.0;
        if s.ends_with('\n') {
            s.pop();
            if s.ends_with('\r') {
                s.pop();
            }
        }
    }
    
    /// Insert the given string at regular character intervals, from left to right.
    /// 
    /// ```ignore
    /// let s = Stringer::new(format!("{}", 10_000_000));
    /// assert_eq!(&s.insert_every("_", 3), "10_000_000");
    ///
    /// ```
    pub fn insert_every(&self, sep: &str, every: usize) -> Self {
        if every == 0 {
            return Self(self.0.clone());
        }
    
        let mut s = String::new();
        for (index, c) in self.chars().enumerate() {
            if index > 0 && index % every == 0 {
                s.push_str(sep);
            }
            s.push(c);
        }
    
        Self(s.chars().collect())
    }

    pub fn pad_ends(self) -> Self {
        Self::new(fmt!(" {} ", self))
    }

    pub fn fit_into(
        &mut self,
        new_len:        usize,
        truncation_str: &str,
    ) {
        let ts_len = truncation_str.len();
        if new_len > ts_len {
            let len = self.chars().count();
            let mut result = self.0.clone();
            if new_len < len {
                result.truncate(new_len - ts_len);
                result.push_str(truncation_str);
            }
            self.0 = result;
        } else {
            self.0 = String::new();
        }
    }
}

