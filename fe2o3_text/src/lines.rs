use crate::{
    Text,
    highlight::{
        Highlight,
        Highlighter,
    },
};

use oxedize_fe2o3_core::prelude::*;
use oxedize_fe2o3_geom::{
    dim::{
        Coord,
        Dim,
        Span,
    },
    rect::{
        AbsRect,
        AbsSize,
    },
};

use std::{
    collections::BinaryHeap,
    fmt,
};


#[derive(Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct LineRange {
    pub line: Dim,
    pub span: Span,
}

impl LineRange {

    pub fn new<D: Into<Dim>>(
        line:           D,
        (start, len):   (D, D),
    )
        -> Self
    {
        Self {
            line: line.into(),
            span: Span::from((start.into(), len.into())),
        }
    }

    pub fn to_abs_rect(&self) -> AbsRect {
        AbsRect {
            top_left:   Coord::new((self.span.start(), self.line)), 
            size:       AbsSize::new((self.span.len(), Dim(1))),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct TextLines<
    T: Clone + fmt::Debug + Default,
    D: Clone + fmt::Debug + Default,
> {
    pub lines:          Vec<Text<T>>,
    pub widths:         BinaryHeap<usize>,
    pub changed:        bool,
    pub highlighter:    Option<Highlighter<D>>,
}

impl <
    T: Clone + fmt::Debug + Default,
    D: Clone + fmt::Debug + Default,
    I: IntoIterator<Item = Text<T>>
>
    From<I> for TextLines<T, D>
{
    fn from(lines: I) -> Self {
        let mut text_lines = TextLines {
            lines:  Vec::new(),
            widths: BinaryHeap::new(),
            ..Default::default()
        };
        for line in lines {
            let width = line.len();
            text_lines.lines.push(line);
            text_lines.widths.push(width);
        }
        text_lines
    }
}

impl<
    T: Clone + fmt::Debug + Default,
    D: Clone + fmt::Debug + Default,
>
    TextLines<T, D>
{
    pub fn new(
        lines:          Vec<Text<T>>,
        highlighter:    Option<Highlighter<D>>,
    )
        -> Self
    {
        let mut widths = BinaryHeap::new();
        for line in &lines {
            let width = line.len();
            widths.push(width);
        }

        Self {
            lines,
            widths,
            highlighter,
            ..Default::default()
        }
    }

    pub fn max_width(&self) -> usize {
        self.widths.peek().copied().unwrap_or(0)
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn get_highlighter(&self) -> Option<&Highlighter<D>> {
        self.highlighter.as_ref()
    }

    pub fn get_highlighter_mut(&mut self) -> Option<&mut Highlighter<D>> {
        self.highlighter.as_mut()
    }

    pub fn set_highlighter(&mut self, highlighter: Option<Highlighter<D>>) {
        self.highlighter = highlighter;
    }

    pub fn has_changed(&self) -> bool {
        self.changed
    }

    pub fn set_changed(&mut self, changed: bool) {
        self.changed = changed;
    }

    pub fn last(&self) -> Option<&Text<T>> {
        if !self.is_empty() {
            Some(&self.lines[self.lines.len() - 1])
        } else {
            None
        }
    }

    /// Get type of last line, if it exists.
    pub fn type_of_last(&self) -> Option<T> {
        if let Some(last) = self.last() {
            Some(last.typ().clone())
        } else {
            None
        }
    }

    pub fn size(&self) -> AbsSize {
        AbsSize::from((self.max_width(), self.len()))
    }

    pub fn append_string(&mut self, s: String) {
        if s.len() > 0 {
            for line in s.lines() {
                let txt = Text::new(line, None);
                let width = txt.len();
                self.lines.push(txt);
                self.widths.push(width);
            }
            self.changed = true;
        }
    }

    /// Append a new line of text.  If an associated highlight is included, it is assumed to apply
    /// to the new line, and so the enclosed line value is updated.
    pub fn append_text_line(
        &mut self,
        txt:        Text<T>,
        highlight:  Option<Highlight<D>>,
    ) {
        if !txt.is_empty() {
            let width = txt.len();
            self.lines.push(txt);
            self.widths.push(width);
            if let Some(mut new_highlight) = highlight {
                new_highlight.range.line = Dim::new(self.len() - 1);
                match &mut self.highlighter {
                    Some(highlighter) => {
                        highlighter.ranges.push(new_highlight);
                    }
                    None => {
                        self.highlighter = Some(Highlighter::new(vec![new_highlight], None));
                    }
                }
            }
            self.changed = true;
        }
    }

    /// Append a new set of lines to the current `TextLines`.  The line numbers of the associated
    /// highlights in the `TextLines` to be appended are adjusted.
    pub fn append_text_lines(
        &mut self,
        text_lines: Self,
    ) {
        let orig_len = self.len();
        if text_lines.len() > 0 {
            for line in text_lines.lines {
                self.append_text_line(line, None);
            }
        }
        if let Some(mut new_highlighter) = text_lines.highlighter {
            new_highlighter.ranges.iter_mut().for_each(|highlight| {
                highlight.range.line += orig_len;
            });
            match &mut self.highlighter {
                Some(highlighter) => {
                    for new_highlight in new_highlighter.ranges {
                        highlighter.ranges.push(new_highlight);
                    }
                }
                None => {
                    self.highlighter = Some(Highlighter::new(new_highlighter.ranges, None));
                }
            }
        }
    }

    /// Insert a new line of text after the given line.  If an associated highlight is included, it
    /// is assumed to apply to the new line, and so the enclosed line value is updated.  If the
    /// insertion line number exceeds the current length, the new text is simply appended.
    pub fn insert_text_line(
        &mut self,
        y:          usize,
        txt:        Text<T>,
        highlight:  Option<Highlight<D>>,
    ) {
        let width = txt.len();
        if y < self.lines.len() {
            // Insert a new line.
            self.lines.insert(y + 1, txt);
            self.inc_highlight_lines(y + 1); 
        } else {
            // Append a new line.
            self.lines.push(txt);
        }
        self.widths.push(width);
        if let Some(mut new_highlight) = highlight {
            new_highlight.range.line = Dim::new(y + 1);
            match &mut self.highlighter {
                Some(highlighter) => {
                    highlighter.insert(new_highlight);
                }
                None => {
                    self.highlighter = Some(Highlighter::new(vec![new_highlight], None));
                }
            }
        }
    }

    /// Delete any highlight that encloses the given coordinates `(x, y)`.
    pub fn delete_highlight_enclosing(&mut self, (x, y): (Dim, Dim)) {
        if let Some(highlighter) = &mut self.highlighter {
            highlighter.delete_highlight_enclosing((x, y));
        }
    }

    /// Delete any highlight on the given line `y`.
    pub fn delete_highlight_line(&mut self, y: usize) {
        if let Some(highlighter) = &mut self.highlighter {
            highlighter.delete_highlight_line(y);
        }
    }

    /// Increment the line number for all highlights for which the current line number is equal to
    /// or greater than the given `y`.
    pub fn inc_highlight_lines(&mut self, y: usize) {
        if let Some(highlighter) = &mut self.highlighter {
            highlighter.inc_highlight_lines(y);
        }
    }

    /// Decrement the line number for all highlights for which the current line number is equal to
    /// or less than the given `y`.
    pub fn dec_highlight_lines(&mut self, y: usize) {
        if let Some(highlighter) = &mut self.highlighter {
            highlighter.dec_highlight_lines(y);
        }
    }

    pub fn add_char(&mut self, cursor: &mut Coord, c: char, replace: bool) {
        let (x, y) = cursor.tup();
        if y < self.lines.len() {
            let width = self.lines[y.as_index()].txt.chars().count();

            self.delete_highlight_enclosing((x, y));

            if x <= width {
                let mut chars: Vec<char> = self.lines[y.as_index()].txt.chars().collect();
                if replace {
                    chars[x.as_index()] = c;
                } else {
                    chars.insert(x.as_index(), c);
                }
                let new_line: String = chars.into_iter().collect();
                self.remove_width(width);
                let new_width = new_line.chars().count();
                self.widths.push(new_width);
                self.lines[y.as_index()].txt = new_line;
            } else {
                // End of line.
                self.lines[y.as_index()].txt.push(c);
                self.remove_width(width);
                let new_width = width + 1;
                self.widths.push(new_width);
            }
            cursor.inc_x(Dim(1));

        } else {
            // New line.
            let mut line = String::new();
            line.push(c);
            let new_width = 1;
            self.widths.push(new_width);
            self.lines.push(Text::new(line, self.type_of_last()));
            cursor.y = Dim::new(self.lines.len() - 1);
            cursor.x = Dim(0);
        }
    }

    pub fn add_str(&mut self, cursor: &mut Coord, s: &str, replace: bool) {
        let (x, y) = cursor.tup();
        if y < self.lines.len() {
            let width = self.lines[y.as_index()].txt.chars().count();
    
            self.delete_highlight_enclosing((x, y));

            if x <= width {
                // Insert into current line.
                let mut chars: Vec<char> = self.lines[y.as_index()].txt.chars().collect();
                if replace {
                    chars.splice(x.as_index()..(x + s.chars().count()).as_index(), s.chars());
                } else {
                    for (i, c) in s.chars().enumerate() {
                        chars.insert(x.as_index() + i, c);
                    }
                }
                let new_line: String = chars.into_iter().collect();
                self.remove_width(width);
                let new_width = new_line.chars().count();
                self.widths.push(new_width);
                self.lines[y.as_index()].txt = new_line;
            } else {
                // End of line.
                self.lines[y.as_index()].txt.push_str(s);
                self.remove_width(width);
                let new_width = width + s.chars().count();
                self.widths.push(new_width);
            }
            cursor.x += s.chars().count();
    
        } else {
            // The string starts a newly appended line.
            let mut line = String::new();
            line.push_str(s);
            let new_width = s.chars().count();
            self.widths.push(new_width);
            self.lines.push(Text::new(line, self.type_of_last()));
            cursor.y = Dim::new(self.lines.len() - 1);
            cursor.x = Dim::new(s.chars().count());
        }
    }

    pub fn backspace(&mut self, cursor: &mut Coord) {
        let (x, y) = cursor.tup();
        if y < self.lines.len() {
            let width = self.lines[y.as_index()].txt.chars().count();
    
            self.delete_highlight_enclosing((x - 1, y));

            if x > Dim(0) {
                // Shorten current line.
                let mut chars: Vec<char> = self.lines[y.as_index()].txt.chars().collect();
                chars.remove((x - 1).as_index());
                let new_line: String = chars.into_iter().collect();
                self.remove_width(width);
                let new_width = new_line.chars().count();
                self.widths.push(new_width);
                self.lines[y.as_index()].txt = new_line;
                cursor.dec_x(Dim(1));
            } else if y > Dim(0) {
                // Join current line with previous line.
                let prev_line_width = self.lines[(y - 1).as_index()].txt.chars().count();
                let current_line = self.lines.remove(y.as_index());
                self.remove_width(width);
    
                self.lines[(y - 1).as_index()].txt.push_str(&current_line.txt);
                self.remove_width(prev_line_width);
                let new_width = self.lines[(y - 1).as_index()].txt.chars().count();
                self.widths.push(new_width);
                self.dec_highlight_lines(y.as_index());
                cursor.y -= 1;
                cursor.x = Dim::new(prev_line_width);
            }
        }
    }
    
    pub fn delete_char(&mut self, cursor: &mut Coord) {
        let (x, y) = cursor.tup();
        if y < self.lines.len() {
            let width = self.lines[y.as_index()].txt.chars().count();
    
            self.delete_highlight_enclosing((x, y));

            if x < width {
                // Pull remainder of current line to the left.
                let mut chars: Vec<char> = self.lines[y.as_index()].txt.chars().collect();
                chars.remove(x.as_index());
                let new_line: String = chars.into_iter().collect();
                self.remove_width(width);
                let new_width = new_line.chars().count();
                self.widths.push(new_width);
                self.lines[y.as_index()].txt = new_line;
            } else if y < self.lines.len() - 1 {
                // Join current line with next line.
                let next_line = self.lines.remove((y + 1).as_index());
                self.remove_width(width);
                self.remove_width(next_line.txt.chars().count());
    
                self.lines[y.as_index()].txt.push_str(&next_line.txt);
                let new_width = self.lines[y.as_index()].txt.chars().count();
                self.widths.push(new_width);
                self.dec_highlight_lines((y + 1).as_index());
            }
        }
    }

    /// Insert a new line after the line specified by the given coordinates.
    pub fn enter_new_line(&mut self, cursor: &mut Coord) {
        let (x, y) = cursor.tup();
        if y < self.lines.len() {
            let width = self.lines[y.as_index()].txt.chars().count();

            if x < width {
                let typ = self.lines[y.as_index()].typ.clone();
                // Split the line at the cursor position.
                let left = self.lines[y.as_index()].txt[..x.as_index()].to_string();
                let right = self.lines[y.as_index()].txt[x.as_index()..].to_string();
                let left_width = left.chars().count();
                let right_width = right.chars().count();

                self.remove_width(width);
                self.widths.push(left_width);
                self.widths.push(right_width);

                self.lines[y.as_index()].txt = left.to_string();
                self.lines.insert(
                    (y + 1).as_index(),
                    Text::new(right.to_string(), Some(typ)),
                );
                self.delete_highlight_enclosing((x, y));
                self.inc_highlight_lines((y + 1).as_index());
            } else {
                // Insert a new line.
                self.lines.insert((y + 1).as_index(), Text::new("", self.type_of_last()));
                self.widths.push(0);
            }
        } else {
            // Append a new line.
            self.lines.push(Text::new("", self.type_of_last()));
            self.widths.push(0);
        }
        cursor.y += 1;
        cursor.x = Dim(0);
    }

    pub fn remove_line(&mut self, y: usize) {
        if y < self.lines.len() {
            let removed_width = self.lines[y].txt.chars().count();
            self.lines.remove(y);
            self.remove_width(removed_width);
            self.delete_highlight_line(y);
            self.dec_highlight_lines(y);
        }
    }

    pub fn remove_width(&mut self, width: usize) {
        self.widths.retain(|&w| w != width);
    }
}
