use crate::{
    style::Style,
    render::{
        Drawable,
        Drawer,
        Renderer,
        Sink,
        When,
    },
};

use oxedize_fe2o3_core::prelude::*;
use oxedize_fe2o3_geom::{
    dim::{
        Coord,
        Dim,
    },
};


#[derive(Clone, Debug, Default)]
pub struct ScrollBarsConfig {
    pub style:  Style,
    pub horiz:  char,
    pub vert:   char,
    pub corner: Option<char>,
    pub always: bool,
}

impl ScrollBarsConfig {
    pub fn horiz(&self) -> String { self.horiz.to_string() }
    pub fn vert(&self) -> String { self.vert.to_string() }
}

///```ignore
///
///     text extent
///     +--------------------------------------+
///     |           ^            ^             |
///     |           |            |             |
///     |           | start      | extent      |
///     |           |            |             |
///     |           v text view  |             |
///     |       +----------+     |             |
///     |       |   ^      |     |             |
///     |<----->|<--+----->|<----+------------>|
///     | start |   |  len |     |    end      |
///     |       |   |      |     |             |
///     |       |   | len  |     |             |
///     |       |   v      |     |             |
///     |       +----------+     |             |
///     |           ^            |             |
///     |           |            |             |
///     |<----------+------------+------------>|
///     |           |            |    extent   |
///     |           |            |             |
///     |           | end        |             |
///     |           |            |             |
///     |           |            |             |
///     |           |            |             |
///     |           v            v             |
///     +--------------------------------------+
///
///```
#[derive(Clone, Debug)]
pub struct TextViewDim {
    pub start:  Dim,
    pub len:    Dim,
    pub end:    Dim,
    pub extent: Dim,
}

impl TextViewDim {
    pub fn new(
        start:  Dim,
        len:    Dim,
        end:    Dim,
    )
        -> Self
    {
        Self {
            start,
            len,
            end,
            extent: start + len + end,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ScrollBar {
    pub top_left:   Coord, // Terminal coordinates of the start of the scroll bar.
    pub scroll_len: Dim, // Length of whole scrollbar structure in terminal.
    pub tview:      TextViewDim, // Text view lengths in the scrollbar direction.
}

impl ScrollBar {

    /// Scale the text view metrics to the terminal coordinates.
    pub fn term_lengths(&self) -> Outcome<(Dim, Dim, Dim)> {

        let mut len_start = try_div!(self.tview.start * self.scroll_len, self.tview.extent); 
        let len_bar =
            try_div!(self.tview.len.min(self.tview.extent) * self.scroll_len, self.tview.extent);
        let mut len_end = self.scroll_len - len_start - len_bar; 

        // The scaling can result in the bar not appearing right up against the start or end when
        // it should, so perform a correction if necessary.
        if self.tview.start == Dim(0) {
            while len_start > Dim(0) {
                len_start = len_start - 1;
                len_end = len_end + 1;
            }
        }
        if self.tview.end == Dim(0) {
            while len_end > Dim(0) {
                len_end = len_end - 1;
                len_start = len_start + 1;
            }
        }

        Ok((len_start, len_bar, len_end))
    }
}

#[derive(Clone, Debug)]
pub struct ScrollBars {
    pub cfg:    ScrollBarsConfig,
    pub x:      Option<ScrollBar>,
    pub y:      Option<ScrollBar>,
}

impl Drawable for ScrollBars {
    fn render<S: Sink, R: Renderer<S>>(
        &mut self,
        drawer: &mut Drawer<S, R>,
        when:   When,
    )
        -> Outcome<()>
    {
        res!(self.cfg.style.render(drawer, When::Later));
        if let Some(bar) = &self.x {
            let (len_start, len_bar, len_end) = res!(bar.term_lengths());
            res!(drawer.rend.set_cursor(bar.top_left, When::Later));
            res!(drawer.rend.print(&" ".to_string().repeat(*len_start), When::Later));
            res!(drawer.rend.print(&self.cfg.horiz().repeat(*len_bar), When::Later));
            res!(drawer.rend.print(&" ".to_string().repeat(*len_end), When::Later));
            if let Some(corner) = &self.cfg.corner {
                res!(drawer.rend.print(&corner.to_string(), When::Later));
            }
        }
        if let Some(bar) = &self.y {
            let (len_start, len_bar, len_end) = res!(bar.term_lengths());
            let mut j = bar.top_left.y;
            let mut count = len_start;
            while count > Dim(0) {
                res!(drawer.rend.set_cursor(Coord::new((bar.top_left.x, j)), When::Later));
                res!(drawer.rend.print(&" ".to_string(), When::Later));
                j += 1;
                count -= 1;
            }
            count = len_bar;
            while count > Dim(0) {
                res!(drawer.rend.set_cursor(Coord::new((bar.top_left.x, j)), When::Later));
                res!(drawer.rend.print(&self.cfg.vert(), When::Later));
                j += 1;
                count -= 1;
            }
            count = len_end;
            while count > Dim(0) {
                res!(drawer.rend.set_cursor(Coord::new((bar.top_left.x, j)), When::Later));
                res!(drawer.rend.print(&" ".to_string(), When::Later));
                j += 1;
                count -= 1;
            }
            if let Some(corner) = &self.cfg.corner {
                res!(drawer.rend.set_cursor(Coord::new((bar.top_left.x, j)), When::Later));
                res!(drawer.rend.print(&corner.to_string(), When::Later));
            }
        }
        res!(drawer.rend.reset_style(When::Later));
        
        if when == When::Now {
            res!(drawer.rend.flush());
        }
        Ok(())
    }
}
