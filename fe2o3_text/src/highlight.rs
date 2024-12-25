use crate::{
    lines::{
        LineRange,
    },
};

use oxedize_fe2o3_core::prelude::*;
use oxedize_fe2o3_geom::{
    dim::{
        Dim,
    },
};

use std::fmt;


#[derive(Clone, Debug, Default)]
pub struct Highlight<D: Clone + fmt::Debug + Default> {
    pub range:  LineRange,
    pub level:  u8,
    pub data:   Option<D>,
}

impl<
    D: Clone + fmt::Debug + Default
>
    Highlight<D>
{
    pub fn new(
        range:  LineRange,
        level:  u8,
        data:   Option<D>,
    )
        -> Self
    {
        Self {
            range,
            level,
            data,
        }
    }

    pub fn get_range(&self) -> &LineRange {
        &self.range
    }
    pub fn get_level(&self) -> u8 {
        self.level
    }
    pub fn get_data(&self) -> Option<&D> {
        self.data.as_ref()
    }
}

/// Build and manage line focus highlighting for some text.
#[derive(Clone, Debug, Default)]
pub struct Highlighter<D: Clone + fmt::Debug + Default> {
    pub ranges: Vec<Highlight<D>>,
    pub focus:  usize,
}

impl<
    D: Clone + fmt::Debug + Default
>
    Highlighter<D>
{
    pub fn new(
        ranges: Vec<Highlight<D>>,
        focus:  Option<usize>,
    )
        -> Self
    {
        let focus = if let Some(focus) = focus {
            if ranges.is_empty() {
                0
            } else {
                if focus < ranges.len() {
                    focus
                } else {
                    ranges.len() - 1
                }
            }
        } else {
            0
        };

        Self {
            ranges,
            focus,
            ..Default::default()
        }
    }
    
    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    pub fn len(&self) -> usize {
        self.ranges.len()
    }

    pub fn get_focus(&self) -> usize {
        self.focus
    }

    pub fn get_highlighted(&self) -> Option<&Highlight<D>> {
        if !self.is_empty() {
            if self.focus > self.ranges.len() - 1 {
                Some(&self.ranges[self.ranges.len() - 1])
            } else {
                Some(&self.ranges[self.focus])
            }
        } else {
            None
        }
    }

    pub fn inc_focus(&mut self) {
        if !self.is_empty() {
            if self.focus >= self.ranges.len() - 1 {
                self.focus = 0;
            } else {
                self.focus += 1;
            }
        } else {
            self.focus = 0;
        }
    }

    pub fn dec_focus(&mut self) {
        if !self.is_empty() {
            if self.focus == 0 {
                self.focus = self.ranges.len() - 1;
            } else {
                self.focus -= 1;
            }
        } else {
            self.focus = 0;
        }
    }

    pub fn normalise_focus(&mut self) {
        if self.focus > self.ranges.len() - 1 {
            self.focus = self.ranges.len() - 1;
        }
    }

    pub fn insert(&mut self, new_highlight: Highlight<D>) {
        let insert_pos = self.ranges.binary_search_by(|highlight| {
            highlight.range.line.cmp(&new_highlight.range.line)
                .then(highlight.range.span.start().cmp(&new_highlight.range.span.start()))
        }).unwrap_or_else(|pos| pos);
        self.ranges.insert(insert_pos, new_highlight);
    }

    pub fn delete_highlight_enclosing(&mut self, (x, y): (Dim, Dim)) {
        self.ranges.retain(|highlight| {
            let (span_x1, span_x2) = highlight.range.span.tup();
            !(highlight.range.line == y && x > span_x1 && x < span_x2)
        });
        self.normalise_focus();
    }

    pub fn delete_highlight_line(&mut self, y: usize) {
        self.ranges.retain(|highlight| {
            highlight.range.line != y
        });
        self.normalise_focus();
    }

    pub fn inc_highlight_lines(&mut self, y: usize) {
        self.ranges.iter_mut().for_each(|highlight| {
            if highlight.range.line.as_index() >= y {
                highlight.range.line += 1;
            }
        });
        if self.focus >= y {
            self.inc_focus();
        }
    }

    pub fn dec_highlight_lines(&mut self, y: usize) {
        self.ranges.iter_mut().for_each(|highlight| {
            if highlight.range.line.as_index() >= y {
                highlight.range.line -= 1;
            }
        });
        if self.focus >= y {
            self.dec_focus();
        }
    }
}
