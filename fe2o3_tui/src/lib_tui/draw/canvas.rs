use crate::lib_tui::{
    style::Style,
    render::{
        Drawable,
        Drawer,
        Renderer,
        Sink,
        When,
    },
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_geom::{
    dim::Coord,
    rect::AbsRect,
};


#[derive(Clone, Debug, Default)]
pub struct CanvasConfig {
    pub style: Style,
}

#[derive(Clone, Debug)]
pub struct Canvas {
    pub cfg:    CanvasConfig,
    pub view:   AbsRect,
}

impl Drawable for Canvas {
    fn render<S: Sink, R: Renderer<S>>(
        &mut self,
        drawer: &mut Drawer<S, R>,
        when:   When,
    )
        -> Outcome<()>
    {
        let (x, y, w, h) = self.view.tup();

        res!(self.cfg.style.render(drawer, When::Later));
        for j in 0..h.as_index() {
            res!(drawer.rend.set_cursor(Coord::new((x, y + j)), When::Later));
            res!(drawer.rend.print(&" ".to_string().repeat(w.as_usize()), When::Later));
        }
        res!(drawer.rend.reset_style(When::Later));
        
        if when == When::Now {
            res!(drawer.rend.flush());
        }
        
        Ok(())
    }
}

impl Canvas {
    pub fn new(
        cfg:    CanvasConfig,
        view:   AbsRect,
    )
        -> Self
    {
        Self {
            cfg,
            view,
        }
    }
}
