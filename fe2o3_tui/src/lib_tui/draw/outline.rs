use crate::lib_tui::{
    cfg::line::LineType,
    render::{
        Drawable,
        Drawer,
        Renderer,
        Sink,
        When,
    },
    style::Style,
    window::WindowMode,
};

use oxedize_fe2o3_core::prelude::*;
use oxedize_fe2o3_geom::{
    dim::Coord,
    rect::{
        AbsRect,
        RectSide,
        RectSideIter,
    },
};


/// Outline state for an individual window.
#[derive(Clone, Copy, Debug, Default)]
pub enum BorderManagementMode {
    #[default]
    Adjust,
    Selection,
}

/// Outline state for an individual window.
#[derive(Clone, Debug)]
pub struct OutlineState {
    pub line: RectSide,
    pub iter: RectSideIter,
    pub mode: Option<BorderManagementMode>,
}

impl Default for OutlineState {
    fn default() -> Self {
        let line = RectSide::Bottom;
        let mut iter = RectSideIter::new(line);
        iter.next();
        Self {
            line,
            iter,
            mode: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct OutlineConfig {
    pub style:  Style,
    pub typ:    LineType,
}

impl Default for OutlineConfig {
    fn default() -> Self {
        Self {
            style:  Style::default(),
            typ:    LineType::SingleSharp,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct OutlineConfigs {
    pub normal:     OutlineConfig,
    pub focus:      OutlineConfig,
    pub interact:   OutlineConfig,
    pub mgmt_mode:  OutlineConfig,
    pub mgmt_tint:  OutlineConfig,
    pub mgmt_focus: OutlineConfig,
}

impl OutlineConfigs {
    pub fn get(
        &self,
        focus:  bool,
        mode:   WindowMode,
        state:  &OutlineState,
    )
        -> &OutlineConfig
    {
        if focus {
            match mode {
                WindowMode::WindowManagement => match state.mode {
                    Some(_) => &self.mgmt_tint,
                    _ => &self.mgmt_mode,
                }
                WindowMode::Interaction => &self.interact,
                _ => &self.focus,
            }
        } else {
            &self.normal
        }
    }
}

#[derive(Clone, Debug)]
pub struct Outline<'a> {
    pub view:   AbsRect,
    pub cfgs:   &'a OutlineConfigs,
    pub state:  &'a OutlineState,
    pub focus:  bool,
    pub wmode:  WindowMode,
}

impl<'a> Drawable for Outline<'a> {
    fn render<S: Sink, R: Renderer<S>>(
        &mut self,
        drawer: &mut Drawer<S, R>,
        when:   When,
    )
        -> Outcome<()>
    {
        let (x, y, w, h) = self.view.tup();
        let mut cfg = self.cfgs.get(self.focus, self.wmode, self.state).clone();
        let parts = drawer.lib.line[cfg.typ].clone();

        res!(cfg.style.render(drawer, When::Later));
        for j in 0..h.as_index() {
            res!(drawer.rend.set_cursor(Coord::new((x, y + j)), When::Later));
            if j == 0 {
                let top_border = fmt!("{}{}{}",
                    parts.top_left,
                    parts.horiz().repeat((w - 2).as_index()),
                    parts.top_right,
                );
                res!(drawer.rend.print(&top_border, When::Later));
            } else if j == (h - 1).as_index() {
                let bottom_border = fmt!("{}{}{}",
                    parts.bot_left,
                    parts.horiz().repeat((w - 2).as_index()),
                    parts.bot_right,
                );
                res!(drawer.rend.print(&bottom_border, When::Later));
            } else {
                res!(drawer.rend.print(&parts.vert(), When::Later));
                res!(drawer.rend.set_cursor(Coord::new((x + w - 1, y + j)), When::Later));
                res!(drawer.rend.print(&parts.vert(), When::Later));
            }
        }

        match self.state.mode {
            Some(_) => {
                let mut cfg = self.cfgs.mgmt_focus.clone();
                let parts = drawer.lib.line[cfg.typ].clone();
                res!(cfg.style.render(drawer, When::Later));
                match self.state.line {
                    RectSide::Top => {
                        res!(drawer.rend.set_cursor(Coord::new((x, y)), When::Later));
                        let top_border = fmt!("{}{}{}",
                            parts.top_left,
                            parts.horiz().repeat((w - 2).as_index()),
                            parts.top_right,
                        );
                        res!(drawer.rend.print(&top_border, When::Later));
                    }
                    RectSide::Right => {
                        res!(drawer.rend.set_cursor(Coord::new((x + w - 1, y)), When::Later));
                        res!(drawer.rend.print(&parts.top_right.to_string(), When::Later));
                        for j in 1..(h - 1).as_index() {
                            res!(drawer.rend.set_cursor(Coord::new((x + w - 1, y + j)), When::Later));
                            res!(drawer.rend.print(&parts.vert(), When::Later));
                        }
                        res!(drawer.rend.set_cursor(Coord::new((x + w - 1, y + h - 1)), When::Later));
                        res!(drawer.rend.print(&parts.bot_right.to_string(), When::Later));
                    }
                    RectSide::Bottom => {
                        res!(drawer.rend.set_cursor(Coord::new((x, y + h - 1)), When::Later));
                        let bottom_border = fmt!("{}{}{}",
                            parts.bot_left,
                            parts.horiz().repeat((w - 2).as_index()),
                            parts.bot_right,
                        );
                        res!(drawer.rend.print(&bottom_border, When::Later));
                    }
                    RectSide::Left => {
                        res!(drawer.rend.set_cursor(Coord::new((x, y)), When::Later));
                        res!(drawer.rend.print(&parts.top_left.to_string(), When::Later));
                        for j in 1..(h - 1).as_index() {
                            res!(drawer.rend.set_cursor(Coord::new((x, y + j)), When::Later));
                            res!(drawer.rend.print(&parts.vert(), When::Later));
                        }
                        res!(drawer.rend.set_cursor(Coord::new((x, y + h - 1)), When::Later));
                        res!(drawer.rend.print(&parts.bot_left.to_string(), When::Later));
                    }
                }
            }
            _ => {}
        }

        res!(drawer.rend.reset_style(When::Later));
        
        if when == When::Now {
            res!(drawer.rend.flush());
        }
        Ok(())
    }
}
