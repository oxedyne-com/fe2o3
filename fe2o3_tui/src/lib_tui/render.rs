use crate::lib_tui::{
    cfg::{
        style::StyleLibrary,
    },
    style::{
        Colour,
        Style,
    },
};

use oxedize_fe2o3_core::prelude::*;
use oxedize_fe2o3_geom::{
    dim::Coord,
    rect::AbsSize,
};

use crossterm;

use std::{
    io::{
        self,
        Write,
    },
    marker::PhantomData,
};


/// A target for terminal output.
pub trait Sink: Write {
    fn flush(&mut self) -> Outcome<()>;
}

/// Implement the Sink trait for io::Stdout
/// ```ignore
/// fn main() -> Outcome<()> {
///     let mut stdout = io::stdout();
///     res!(clear_canvas(&mut stdout));
///     Ok(())
/// }
/// ```
impl Sink for io::Stdout {
    fn flush(&mut self) -> Outcome<()> {
        res!(io::Write::flush(self));
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum When {
    Later,
    Now,
}

pub trait Drawable {
    fn render<S: Sink, R: Renderer<S>>(
        &mut self,
        drawer:     &mut Drawer<S, R>,
        when:       When,
    )
        -> Outcome<()>;
}

pub struct Drawer<S: Sink, R: Renderer<S>> {
    pub rend:       R,
    pub lib:        StyleLibrary,
    pub mouse_on:   bool,
    phantom:        PhantomData<S>,
}

impl<S: Sink, R: Renderer<S>> Drawer<S, R> {

    pub fn new(
        rend:       R,
        lib:        StyleLibrary,
        mouse_on:   bool,
    )
        -> Self
    {
        Self {
            rend,
            lib,
            mouse_on,
            phantom: PhantomData,
        }
    }

    pub fn sink(&mut self)  -> &mut S { self.rend.sink() }

    pub fn on(&mut self) -> Outcome<()> {
        res!(self.rend.on());
        res!(self.rend.mouse_capture(self.mouse_on));
        Ok(())
    }

    pub fn off(&mut self) -> Outcome<()> {
        res!(self.rend.off());
        res!(self.rend.mouse_capture(!self.mouse_on));
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub enum CursorStyle {
    DefaultUserShape,
    #[default]
    BlinkingBlock,
    SteadyBlock,
    BlinkingUnderScore,
    SteadyUnderScore,
    BlinkingBar,
    SteadyBar,
}

impl CursorStyle {
    pub fn to_crossterm(&self) -> crossterm::cursor::SetCursorStyle {
        match self {
            Self::DefaultUserShape    => crossterm::cursor::SetCursorStyle::DefaultUserShape,
            Self::BlinkingBlock       => crossterm::cursor::SetCursorStyle::BlinkingBlock,
            Self::SteadyBlock         => crossterm::cursor::SetCursorStyle::SteadyBlock,
            Self::BlinkingUnderScore  => crossterm::cursor::SetCursorStyle::BlinkingUnderScore,
            Self::SteadyUnderScore    => crossterm::cursor::SetCursorStyle::SteadyUnderScore,
            Self::BlinkingBar         => crossterm::cursor::SetCursorStyle::BlinkingBar,
            Self::SteadyBar           => crossterm::cursor::SetCursorStyle::SteadyBar,
        }
    }
}

pub trait Renderer<S: Sink>: Write {
    // Cursor
    fn hide_cursor(&mut self, when: When)           -> Outcome<()>;
    fn show_cursor(&mut self, when: When)           -> Outcome<()>;
    fn get_cursor(&mut self)                        -> Outcome<Coord>;
    fn set_cursor(&mut self, c: Coord, when: When)  -> Outcome<()>;
    fn set_cursor_style(&mut self, style: CursorStyle, when: When)  -> Outcome<()>;
    // Terminal
    fn clear(&mut self) -> Outcome<()>;
    fn off(&mut self)   -> Outcome<()>;
    fn on(&mut self)    -> Outcome<()>;
    fn size(&self)      -> Outcome<AbsSize>;
    // Output
    fn print(&mut self, text: &str, when: When)     -> Outcome<()>;
    fn sink(&mut self)                              -> &mut S;
    // Mouse
    fn mouse_capture(&mut self, capture_on: bool)   -> Outcome<()>;
    // Colours
    fn reset_colour(&mut self, when: When)                  -> Outcome<()>;
    fn set_back_colour(&mut self, col: &Colour, when: When) -> Outcome<()>;
    fn set_fore_colour(&mut self, col: &Colour, when: When) -> Outcome<()>;
    // Style
    fn set_style(&mut self, style: &Style, when: When)      -> Outcome<()>;
    fn reset_style(&mut self, when: When)                   -> Outcome<()>;
}

pub struct CrosstermRenderer<S: Sink> {
    pub sink: S,
}

impl<S: Sink> CrosstermRenderer<S> {

    pub fn new(sink: S) -> Self {
        Self { sink }
    }

    pub fn sink(&mut self) -> &mut S { &mut self.sink }
}

impl<S: Sink> Write for CrosstermRenderer<S> {

    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.sink.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        io::Write::flush(&mut self.sink)
    }
}

impl<S: Sink> Renderer<S> for CrosstermRenderer<S> {

    // Cursor
    
    fn hide_cursor(&mut self, when: When) -> Outcome<()> {
        match when {
            When::Later => res!(crossterm::queue!(
                self.sink,
                crossterm::cursor::Hide,
            )),
            When::Now => res!(crossterm::execute!(
                self.sink,
                crossterm::cursor::Hide,
            )),
        }
        Ok(())
    }

    fn show_cursor(&mut self, when: When) -> Outcome<()> {
        match when {
            When::Later => res!(crossterm::queue!(
                self.sink,
                crossterm::cursor::Show,
            )),
            When::Now => res!(crossterm::execute!(
                self.sink,
                crossterm::cursor::Show,
            )),
        }
        Ok(())
    }

    fn get_cursor(&mut self) -> Outcome<Coord> {
        let (x, y) = res!(crossterm::cursor::position());
        Ok(Coord::from((x as usize, y as usize)))
    }

    fn set_cursor(&mut self, c: Coord, when: When) -> Outcome<()> {
        let (x, y) = (try_into!(u16, c.x.as_usize()), try_into!(u16, c.y.as_usize()));
        match when {
            When::Later =>
                res!(crossterm::queue!(
                    self.sink,
                    crossterm::cursor::MoveTo(x, y),
                )),
            When::Now =>
                res!(crossterm::execute!(
                    self.sink,
                    crossterm::cursor::MoveTo(x, y),
                )),
        }
        Ok(())
    }

    fn set_cursor_style(&mut self, style: CursorStyle, when: When) -> Outcome<()> {
        let style = style.to_crossterm();
        match when {
            When::Later => res!(crossterm::queue!(self.sink, style)),
            When::Now => res!(crossterm::execute!(self.sink, style)),
        }
        Ok(())
    }

    // Terminal
    
    fn clear(&mut self) -> Outcome<()> {
        res!(crossterm::execute!(
            self.sink,
            crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
        ));
        Ok(())
    }

    fn off(&mut self) -> Outcome<()> {
        res!(crossterm::execute!(
            self.sink,
            crossterm::cursor::Show,
            crossterm::terminal::LeaveAlternateScreen,
        ));
        res!(crossterm::terminal::disable_raw_mode());
        Ok(())
    }

    fn on(&mut self) -> Outcome<()> {
        res!(crossterm::terminal::enable_raw_mode());
        res!(crossterm::execute!(
            self.sink,
            crossterm::terminal::EnterAlternateScreen,
            //crossterm::cursor::Hide,
            //crossterm::cursor::SetCursorStyle::BlinkingUnderScore,
        ));
        Ok(())
    }

    fn size(&self) -> Outcome<AbsSize> {
        Ok(AbsSize::from(res!(crossterm::terminal::size())))
    }

    // Output
    
    fn sink(&mut self) -> &mut S { &mut self.sink }

    fn print(&mut self, text: &str, when: When) -> Outcome<()> {
        match when {
            When::Later => res!(crossterm::queue!(
                self.sink,
                crossterm::style::Print(text),
            )),
            When::Now => res!(crossterm::execute!(
                self.sink,
                crossterm::style::Print(text),
            )),
        }
        Ok(())
    }

    // Mouse
    
    fn mouse_capture(&mut self, capture_on: bool) -> Outcome<()> {
        if capture_on {
            res!(crossterm::execute!(
                self.sink(),
                crossterm::event::EnableMouseCapture,
            ));
        } else {
            res!(crossterm::execute!(
                self.sink(),
                crossterm::event::DisableMouseCapture,
            ));
        }
        Ok(())
    }

    // Colours
    
    fn reset_colour(&mut self, when: When) -> Outcome<()> {
        match when {
            When::Later => res!(crossterm::queue!(
                self.sink,
                crossterm::style::ResetColor,
            )),
            When::Now => res!(crossterm::execute!(
                self.sink,
                crossterm::style::ResetColor,
            )),
        }
        Ok(())
    }

    fn set_back_colour(&mut self, col: &Colour, when: When) -> Outcome<()> {
        match when {
            When::Later =>
                res!(crossterm::queue!(
                    self.sink,
                    crossterm::style::SetBackgroundColor((*col).into()),
                )),
            When::Now =>
                res!(crossterm::execute!(
                    self.sink,
                    crossterm::style::SetBackgroundColor((*col).into()),
                )),
        }
        Ok(())
    }

    fn set_fore_colour(&mut self, col: &Colour, when: When) -> Outcome<()> {
        match when {
            When::Later =>
                res!(crossterm::queue!(
                    self.sink,
                    crossterm::style::SetForegroundColor((*col).into()),
                )),
            When::Now =>
                res!(crossterm::execute!(
                    self.sink,
                    crossterm::style::SetForegroundColor((*col).into()),
                )),
        }
        Ok(())
    }

    fn set_style(&mut self, style: &Style, when: When) -> Outcome<()> {
        // Set foreground color
        if let Some(col) = style.cols.fore {
            res!(self.set_fore_colour(&col, when));
        }

        // Set background color
        if let Some(col) = style.cols.back {
            res!(self.set_back_colour(&col, when));
        }

        // Set attributes
        for attr in &style.attr {
            let attr: crossterm::style::Attribute = (*attr).into();
            match when {
                When::Later => {
                    res!(crossterm::queue!(self.sink, crossterm::style::SetAttribute(attr)));
                }
                When::Now => {
                    res!(crossterm::execute!(self.sink, crossterm::style::SetAttribute(attr)));
                }
            }
        }

        Ok(())
    }

    fn reset_style(&mut self, when: When) -> Outcome<()> {
        res!(self.reset_colour(when));
        match when {
            When::Later => {
                res!(crossterm::queue!(
                    self.sink,
                    crossterm::style::SetAttribute(crossterm::style::Attribute::Reset)
                ));
            }
            When::Now => {
                res!(crossterm::execute!(
                    self.sink,
                    crossterm::style::SetAttribute(crossterm::style::Attribute::Reset)
                ));
            }
        }
        Ok(())
    }

}

/// Useful for debugging, all commands are displayed in the log but not executed in the terminal.
pub struct DebugCrosstermRenderer<S: Sink> {
    pub sink: S,
}

impl<S: Sink> DebugCrosstermRenderer<S> {

    pub fn new(sink: S) -> Self {
        Self { sink }
    }

    pub fn sink(&mut self) -> &mut S { &mut self.sink }
}

impl<S: Sink> Write for DebugCrosstermRenderer<S> {

    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.sink.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        io::Write::flush(&mut self.sink)
    }
}

impl<S: Sink> Renderer<S> for DebugCrosstermRenderer<S> {

    // Cursor
    
    fn hide_cursor(&mut self, when: When) -> Outcome<()> {
        debug!("hide_cursor {:?}", when);
        Ok(())
    }

    fn show_cursor(&mut self, when: When) -> Outcome<()> {
        debug!("show_cursor {:?}", when);
        Ok(())
    }

    fn get_cursor(&mut self) -> Outcome<Coord> {
        let (x, y) = res!(crossterm::cursor::position());
        debug!("get_cursor: (x, y) = ({}, {})", x, y);
        Ok(Coord::from((x as usize, y as usize)))
    }

    fn set_cursor(&mut self, c: Coord, when: When) -> Outcome<()> {
        let (x, y) = (try_into!(u16, c.x.as_usize()), try_into!(u16, c.y.as_usize()));
        debug!("set_cursor {:?}: (x, y) = ({}, {})", when, x, y);
        Ok(())
    }

    fn set_cursor_style(&mut self, style: CursorStyle, when: When) -> Outcome<()> {
        debug!("set_cursor_style {:?}: style = {:?}", when, style);
        Ok(())
    }

    // Terminal
    
    fn clear(&mut self) -> Outcome<()> {
        debug!("clear");
        Ok(())
    }

    fn off(&mut self) -> Outcome<()> {
        debug!("off");
        Ok(())
    }

    fn on(&mut self) -> Outcome<()> {
        debug!("on");
        Ok(())
    }

    fn size(&self) -> Outcome<AbsSize> {
        let size = res!(crossterm::terminal::size());
        debug!("size: {:?}", size);
        Ok(AbsSize::from(size))
    }

    // Output
    
    fn sink(&mut self) -> &mut S { &mut self.sink }

    fn print(&mut self, text: &str, when: When) -> Outcome<()> {
        debug!("print {:?}: text = '{}'", when, text);
        Ok(())
    }

    // Mouse
    
    fn mouse_capture(&mut self, capture_on: bool) -> Outcome<()> {
        debug!("mouse_capture {}", capture_on);
        Ok(())
    }

    // Colours
    
    fn reset_colour(&mut self, when: When) -> Outcome<()> {
        debug!("reset_colour {:?}", when);
        Ok(())
    }

    fn set_back_colour(&mut self, col: &Colour, when: When) -> Outcome<()> {
        debug!("set_back_colour {:?}: {:?}", when, col);
        Ok(())
    }

    fn set_fore_colour(&mut self, col: &Colour, when: When) -> Outcome<()> {
        debug!("set_fore_colour {:?}: {:?}", when, col);
        Ok(())
    }

    // Style
    
    fn set_style(&mut self, style: &Style, when: When) -> Outcome<()> {
        debug!("set_style {:?}: {:?}", when, style);
        Ok(())
    }

    fn reset_style(&mut self, when: When) -> Outcome<()> {
        debug!("reset_style {:?}", when);
        Ok(())
    }

}
