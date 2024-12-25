use oxedize_fe2o3_core::prelude::*;

use std::ops::{
    Index,
    IndexMut,
};


#[derive(Clone, Copy, Debug, Default)]
pub struct LineParts {
    pub top_left:   char,
    pub top_right:  char,
    pub bot_right:  char,
    pub bot_left:   char,
    pub horiz:      char,
    pub vert:       char,
}

impl LineParts {
    pub fn top_left(&self)  -> String { self.top_left.to_string() }
    pub fn top_right(&self) -> String { self.top_right.to_string() }
    pub fn bot_right(&self) -> String { self.bot_right.to_string() }
    pub fn bot_left(&self)  -> String { self.bot_left.to_string() }
    pub fn horiz(&self)     -> String { self.horiz.to_string() }
    pub fn vert(&self)      -> String { self.vert.to_string() }
}

#[derive(Clone, Copy, Debug)]
#[repr(usize)]
pub enum LineType {
    Blank,
    SingleSharp,
    DoubleSharp,
    SingleRounded,
    ThickSingleSharp,
}

new_enum!(LineType;
    Blank,
    SingleSharp,
    DoubleSharp,
    SingleRounded,
    ThickSingleSharp,
);

#[derive(Clone, Debug)]
pub struct LineLibrary {
    pub lines: [LineParts; LineType::num_of_variants()],
}

impl Default for LineLibrary {
    fn default() -> Self {
        let mut lines = [LineParts::default(); LineType::num_of_variants()];
        for (i, typ) in LineType::variants().iter().enumerate() {
            lines[i] = match typ {
                LineType::Blank => LineParts {
                    top_left:   ' ',
                    top_right:  ' ',
                    bot_right:  ' ',
                    bot_left:   ' ',
                    horiz:      ' ',
                    vert:       ' ',
                },
                LineType::SingleSharp => LineParts {
                    top_left:   '┌',
                    top_right:  '┐',
                    bot_right:  '┘',
                    bot_left:   '└',
                    horiz:      '─',
                    vert:       '│',
                },
                LineType::ThickSingleSharp => LineParts {
                    top_left:   '┏',
                    top_right:  '┓',
                    bot_right:  '┛',
                    bot_left:   '┗',
                    horiz:      '━',
                    vert:       '┃',
                },
                LineType::DoubleSharp => LineParts {
                    top_left:   '╔',
                    top_right:  '╗',
                    bot_right:  '╝',
                    bot_left:   '╚',
                    horiz:      '═',
                    vert:       '║',
                },
                LineType::SingleRounded => LineParts {
                    top_left:   '╭',
                    top_right:  '╮',
                    bot_right:  '╯',
                    bot_left:   '╰',
                    horiz:      '─',
                    vert:       '│',
                },
            };
        }
        Self { lines }
    }
}

impl Index<LineType> for LineLibrary {
    type Output = LineParts;

    fn index(&self, typ: LineType) -> &Self::Output {
        &self.lines[typ as usize]
    }
}

impl IndexMut<LineType> for LineLibrary {
    fn index_mut(&mut self, typ: LineType) -> &mut Self::Output {
        &mut self.lines[typ as usize]
    }
}

