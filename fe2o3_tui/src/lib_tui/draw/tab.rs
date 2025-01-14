use crate::lib_tui::{
    style::Style,
    draw::{
        canvas::CanvasConfig,
        tbox::TextBox,
    },
    render::{
        Drawable,
        Drawer,
        Renderer,
        Sink,
        When,
    },
};

use oxedize_fe2o3_core::{
    prelude::*,
    ord::ord_string,
};
use oxedize_fe2o3_geom::{
    dim::{
        Coord,
        Dim,
    },
    rect::AbsRect,
};
use oxedize_fe2o3_text::string::Stringer;

use std::collections::HashSet;


#[derive(Clone, Debug)]
pub struct TabStripConfig {
    pub canvases: Vec<CanvasConfig>,
}

impl Default for TabStripConfig {
    fn default() -> Self {
        Self {
            canvases: Vec::new(),
        }
    }
}

impl TabStripConfig {

    pub fn new(styles: Vec<Style>) -> Outcome<Self> {
        let len = styles.len();
        if len < 3 {
            return Err(err!(
                "Please provide at least three tab styles, starting with the empty tab, \
                only {} provided.", len;
            Input, TooSmall, Init)); 
        }
        let mut canvases = Vec::new();
        for style in styles {
            canvases.push(CanvasConfig { style, });
        }
        Ok(Self {
            canvases,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct Tab {
    pub width:      Dim,
    pub canvas:     CanvasConfig,
    pub label:      String,
}

#[derive(Clone, Debug, Default)]
pub struct TabbedTextBox {
    pub tab:    Tab,
    pub tbox:   TextBox,
}

#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct TabbedTile {
    pub index:  usize,
    pub width:  Dim,
}

/// A "tab strip" consists of one or more rows of tabs.  The first is for window-width text boxes.
/// The second is optionally for tiled text boxes`n` equal length tabs where the rightmost tab is
/// empty of content.
#[derive(Clone, Debug, Default)]
pub struct TabbedTextManager {
    pub cfg:        TabStripConfig,
    pub tboxes:     Vec<TabbedTextBox>,
    pub tiled:      Option<Vec<TabbedTile>>,
    pub focus:      usize,
    pub term_view:  AbsRect,
}

impl Drawable for TabbedTextManager {
    fn render<S: Sink, R: Renderer<S>>(
        &mut self,
        drawer: &mut Drawer<S, R>,
        when:   When,
    )
        -> Outcome<()>
    {
        let (x, y) = self.term_view.top_left.tup();

        // First row.
        let mut start_x = x;
        for tabbed_tbox in self.tboxes.iter_mut() {
            let tab = &mut tabbed_tbox.tab;
            let width = tab.width;
            let mut label = Stringer::new(tab.label.clone());
            let avail_len = width - 2;
            label.fit_into(*avail_len, "...");
            let lab_len = label.chars().count();
            let trailing_padding = " ".repeat(*(width - lab_len - 1));

            res!(tab.canvas.style.render(drawer, When::Later));
            res!(drawer.rend.set_cursor(Coord::new((start_x, y)), When::Later));
            res!(drawer.rend.print(&fmt!(" {}{}", label, trailing_padding), When::Later));
            start_x += width;
        }
        // Possible second row.
        if let Some(tabbed_tiles) = &self.tiled {
            if self.focus_is_on_tiled() {
                let mut start_x = x;
                for tabbed_tile in tabbed_tiles {
                    let tabbed_tbox = &mut self.tboxes[tabbed_tile.index];
                    let tab = &mut tabbed_tbox.tab;
                    let width = tabbed_tile.width;
                    let mut label = Stringer::new(tab.label.clone());
                    let avail_len = width - 2;
                    label.fit_into(*avail_len, "...");
                    let lab_len = label.chars().count();
                    let trailing_padding = " ".repeat(*(width - lab_len - 1));

                    res!(tab.canvas.style.render(drawer, When::Later));
                    res!(drawer.rend.set_cursor(Coord::new((start_x, y + 1)), When::Later));
                    res!(drawer.rend.print(&fmt!(" {}{}", label, trailing_padding), When::Later));
                    start_x += width;
                }
            }
        }

        res!(drawer.rend.reset_style(When::Later));
        
        if when == When::Now {
            res!(drawer.rend.flush());
        }
        Ok(())
    }
}

impl TabbedTextManager {

    pub fn new(
        cfg:        TabStripConfig,
        mut tboxes: Vec<TabbedTextBox>,
        tiled:      Option<Vec<TabbedTile>>,
        focus:      Option<usize>,
    )
        -> Outcome<Self>
    {
        for ttbox in tboxes.iter_mut() {
            ttbox.tbox.state.canvas = ttbox.tab.canvas.clone();
        }
        if let Some(tiled) = &tiled {
            res!(Self::check_tiles(&tiled, tboxes.len()));
        }

        Ok(Self {
            cfg,
            tboxes,
            tiled,
            focus: if let Some(f) = focus { f } else { 0 },
            term_view: AbsRect::default(),
        })
    }

    pub fn is_empty(&self) -> bool {
        if self.tboxes.len() == 0 {
            true
        } else {
            false
        }
    }

    fn check_tiles(tiled: &Vec<TabbedTile>, num_tabs: usize) -> Outcome<()> {
        let mut set = HashSet::new();
        for (i, tabbed_tile) in tiled.iter().enumerate() {
            if !set.insert(tabbed_tile) {
                return Err(err!(
                    "The {} tabbed tile {:?} duplicates an existing tabbed tile.",
                    ord_string(i), tabbed_tile;
                Input, Duplicate, Invalid));
            }
            if tabbed_tile.index >= num_tabs {
                return Err(err!(
                    "The {} tabbed tile {:?} has an index exceeding the number \
                    of tabs, {}.", ord_string(i), tabbed_tile, num_tabs;
                Input, TooBig, Invalid));
            }
        }
        Ok(())
    }

    pub fn strip_height(&self) -> Dim {
        let h1 = if self.tboxes.len() > 0 {
            Dim(1)
        } else {
            return Dim(0);
        };
        let h2 = if self.focus_is_on_tiled() {
            Dim(1)
        } else {
            Dim(0)
        };
        h1 + h2
    }

    pub fn get_number_of_tabs(&self) -> (usize, usize) {
        (
            self.tboxes.len(),
            if let Some(tiled) = &self.tiled { tiled.len() } else { 0 },
        )
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut TextBox> {
        self.tboxes.iter_mut().map(|ttbox| &mut ttbox.tbox)
    }

    pub fn next_focus(&mut self) -> Outcome<()> {
        let (num_non_tiled, num_tiled) = self.get_number_of_tabs();
        self.focus = try_rem!(self.focus + 1, num_non_tiled + num_tiled);
        Ok(())
    }

    pub fn focus_is_on_tiled(&self) -> bool {
        self.focus >= self.tboxes.len()
    }

    pub fn get_focal_tabbed_text_box(&self) -> Option<&TabbedTextBox> {
        if self.tboxes.len() > 0 {
            let (num_non_tiled, _num_tiled) = self.get_number_of_tabs();
            if self.focus < num_non_tiled {
                Some(&self.tboxes[self.focus])
            } else {
                if let Some(tiled) = &self.tiled {
                    let tabbed_tile = &tiled[self.focus - num_non_tiled];
                    Some(&self.tboxes[tabbed_tile.index])
                } else {
                    None
                }
            }
        } else {
            None
        }
    }

    pub fn get_focal_tabbed_text_box_mut(&mut self) -> Option<&mut TabbedTextBox> {
        if self.tboxes.len() > 0 {
            let (num_non_tiled, _num_tiled) = self.get_number_of_tabs();
            if self.focus < num_non_tiled {
                Some(&mut self.tboxes[self.focus])
            } else {
                if let Some(tiled) = &self.tiled {
                    let tabbed_tile = &tiled[self.focus - num_non_tiled];
                    Some(&mut self.tboxes[tabbed_tile.index])
                } else {
                    None
                }
            }
        } else {
            None
        }
    }

    pub fn update_tabs(&mut self, term_view: AbsRect) -> Outcome<()> {
        self.term_view = term_view;
        let w = term_view.size.x;
        // First row.
        let n = self.tboxes.len();
        let canvas_len = self.cfg.canvases.len();
        let widths = res!(Self::divide_evenly(w, n));
        for t in 0..n {
            // When the number of tabs exceeds the number of canvas configurations, cycle back
            // through the configurations.
            let canvas_ind = if t < canvas_len { t } else { try_rem!(t, canvas_len) + 1 };
            self.tboxes[t].tab.width = widths[t];
            self.tboxes[t].tab.canvas = self.cfg.canvases[canvas_ind].clone();
            self.tboxes[t].tbox.state.canvas = self.tboxes[t].tab.canvas.clone();
        }
        // Possible second row.
        if let Some(tabbed_tiles) = &mut self.tiled {
            let n = tabbed_tiles.len();
            if n > 0 {
                let widths = res!(Self::divide_evenly(w, n));
                for t in 0..tabbed_tiles.len() {
                    tabbed_tiles[t].width = widths[t];
                }
            }
        }
        Ok(())
    }

    /// Divide the interval `w` as equally as possibly `n` ways, using integer arithmetic.
    /// Distribute any remainder amongst the leading parts.
    pub fn divide_evenly(w: Dim, n: usize) -> Outcome<Vec<Dim>> {
        let n0 = n;
        let n = Dim::from(n);
        let mut parts = vec![try_div!(w, n); n0];
        let rem = Dim::from(try_rem!(w, *n));
        for i in 0..rem.as_index() {
            parts[i] += 1;
        }
        Ok(parts)
    }
}
