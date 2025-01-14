use crate::lib_tui::{
    render::{
        Drawable,
        Drawer,
        Renderer,
        Sink,
        When,
    },
    style::Style,
    text::edit::EditorMode,
};

use oxedize_fe2o3_core::prelude::*;
use oxedize_fe2o3_geom::{
    dim::{
        Coord,
        Dim,
    },
    rect::AbsRect,
};
use oxedize_fe2o3_text::string::Stringer;

use std::fmt;


#[derive(Clone, Debug, Default)]
pub enum StatusStripType {
    #[default]
    Header,
    Footer,
}

#[derive(Clone, Debug)]
pub enum StatusStripLeft {
    Origin(String),
}

impl StatusStripLeft {
    
    pub fn display_fit(&self, avail_len: usize) -> String {
        match self {
            Self::Origin(origin) => {
                let mut origin = Stringer::new(origin);
                origin.fit_into(avail_len, "...");
                origin.to_string()
            }
        }
    }
}

#[derive(Clone, Debug)]
pub enum StatusStripRight {
    Cursor(Coord),
    Label(String),
    Mode(Option<EditorMode>),
    ModeLabel(Option<EditorMode>, String),
}

impl fmt::Display for StatusStripRight {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cursor(coord) => write!(f, "{},{}", coord.x, coord.y),
            Self::Label(label) => write!(f, "{}", label),
            Self::Mode(mode_opt) => match mode_opt {
                Some(mode) => write!(f, "{}", mode),
                None => write!(f, ""),
            }
            Self::ModeLabel(mode_opt, label) => match mode_opt {
                Some(mode) => write!(f, "{} {}", mode, label),
                None => write!(f, "{}", label),
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct StatusStripContent {
    pub left:   Option<StatusStripLeft>,
    pub right:  Option<StatusStripRight>,
    //pub cursor: Coord,
}

#[derive(Clone, Debug)]
pub struct StatusStripConfig {
    pub style:      Style,
    pub typ:        StatusStripType,
    pub right_len:  Dim,
}

impl Default for StatusStripConfig {
    fn default() -> Self {
        Self {
            style:      Style::default(),
            typ:        StatusStripType::default(),
            right_len:  Dim(10),
        }
    }
}

#[derive(Clone, Debug)]
pub struct StatusStrip {
    pub cfg:        StatusStripConfig,
    pub content:    StatusStripContent,
    pub rect:       AbsRect, // The window rectangle to which the strip is attached.
}

impl Drawable for StatusStrip {
    fn render<S: Sink, R: Renderer<S>>(
        &mut self,
        drawer: &mut Drawer<S, R>,
        when:   When,
    )
        -> Outcome<()>
    {
        let (x, y, w, h) = self.rect.tup();

        res!(self.cfg.style.render(drawer, When::Later));
        let ys = match self.cfg.typ {
            StatusStripType::Header => y,
            StatusStripType::Footer => y + h - 1,
        };
        res!(drawer.rend.set_cursor(Coord::new((x, ys)), When::Later));

        let right_str = if let Some(right) = &self.content.right {
            fmt!("{} ", right)
        } else {
            fmt!(" ")
        };
        let right_len = Dim::new(right_str.chars().count());
        let avail_len = w - right_len - Dim(2);
        
        let left_str = if let Some(left) = &self.content.left {
            fmt!(" {}", left.display_fit(avail_len.as_usize()))
        } else {
            fmt!(" ")
        };
        let left_len = Dim::new(left_str.chars().count());
        let pad = w - left_len - right_len;
        let strip_str = fmt!("{}{}{}", left_str, " ".repeat(pad.as_usize()), right_str);  


        //let right_len = right_str.chars().count();
        //let coord = fmt!("{},{}", self.content.cursor.x, self.content.cursor.y);
        //if coord.len() + strip_str.len() < w {
        //    strip_str = fmt!("{} {}", coord, strip_str);   
        //}
        //let mut origin = Stringer::new(self.content.origin.clone());
        //let avail_len = w - right_str.len() - 2;
        //origin.fit_into(*avail_len, "...");
        //let origin_len = origin.chars().count();
        //if origin_len > 0 {
        //    let pad = w - strip_str.len() - origin_len - 2;
        //    strip_str = fmt!(" {} {}{}", origin, &" ".repeat(*pad), strip_str);   
        //} else {
        //    let pad = w - strip_str.len();
        //    strip_str = fmt!("{}{}", &" ".repeat(*pad), strip_str);   
        //}
        res!(drawer.rend.print(&strip_str, When::Later));

        res!(drawer.rend.reset_style(When::Later));
        
        if when == When::Now {
            res!(drawer.rend.flush());
        }
        Ok(())
    }
}

impl StatusStrip {
    pub fn new(
        cfg:        StatusStripConfig,
        content:    StatusStripContent,
        rect:       AbsRect,
    )
        -> Self
    {
        Self {
            cfg,
            content,
            rect,
        }
    }
}
