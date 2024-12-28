use crate::{
    draw::{
        canvas::{
            Canvas,
            CanvasConfig,
        },
        scrollbars::{
            ScrollBar,
            ScrollBars,
            ScrollBarsConfig,
            TextViewDim,
        },
    },
    render::{
        CursorStyle,
        Drawable,
        Drawer,
        Renderer,
        Sink,
        When,
    },
    style::Style,
    text::{
        nav::PositionCursor,
        typ::{
            HighlightType,
            TextType,
            TextViewType,
        },
        view::TextView,
    },
};

use oxedize_fe2o3_core::prelude::*;
use oxedize_fe2o3_geom::{
    dim::{
        Coord,
        Dim,
    },
    rect::{
        AbsRect,
        AbsSize,
    },
};
use oxedize_fe2o3_text::{
    access::AccessibleText,
    lines::TextLines,
};


#[derive(Clone, Debug, Default)]
pub struct TextBoxConfig {
    pub cursor_style:       Option<CursorStyle>,
    pub cursor_position:    PositionCursor,
    pub scrollbars:         Option<ScrollBarsConfig>,
    pub empty_line:         Option<(Style, String)>,
    pub highlight_styles:   Vec<Style>,
}

impl TextBoxConfig {
    pub fn cursor_is_visible(&self) -> bool {
        self.cursor_style.is_some()
    }
}

/// The text box adopts the tab canvas, or if there are no tabs, the window canvas.
#[derive(Clone, Debug, Default)]
pub struct TextBoxState {
    pub canvas: CanvasConfig,
}

#[derive(Clone, Debug, Default)]
pub struct TextBox {
    pub cfg:    TextBoxConfig,
    pub state:  TextBoxState,
    pub tview:  TextView,
    pub view:   AbsRect, // View in terminal.
}

impl Drawable for TextBox {
    fn render<S: Sink, R: Renderer<S>>(
        &mut self,
        drawer: &mut Drawer<S, R>,
        when:   When,
    )
        -> Outcome<()>
    {
        // Account for scrollbars and render them if necessary.
        if let Some(cfg) = &self.cfg.scrollbars {
            let (mut scrollbars, thickness) = self.scrollbars(
                cfg.clone(), 
                self.view,
            );
            res!(scrollbars.render(drawer, When::Later));
            self.view.size -= thickness;
        }

        res!(self.tview.update_view(
            &self.view,
            &self.cfg.cursor_position,
        ));

        // Paint the canvas.
        let mut canvas = Canvas::new(
            self.state.canvas.clone(),
            self.view,
        );
        res!(canvas.render(drawer, When::Later));

        res!(self.state.canvas.style.render(drawer, When::Later));

        //// We want to borrow the TextLines but then need mutable access to the TextBox to render.
        //// The Rust borrow rules during compilation don't allow a mutable borrow while an immutable
        //// one exists on all or part of TextBox.  So we use the Indiana Jones trick of taking it
        //// out and replacing it during runtime once we have finished, as a method of interior
        //// mutability.
        //// Credit:
        //// https://rust-unofficial.github.io/too-many-lists/first-push.html?highlight=indiana#push
        //// 
        let atext = std::mem::replace(&mut self.tview.atext, AccessibleText::default());

        // Access the text, and render it.
        match atext {
            AccessibleText::ThreadShared(ref locked) => {
                let text_lines = lock_write!(locked);
                res!(self.render_text_view(drawer, &text_lines));
            }
            AccessibleText::Shared(ref locked) => {
                let text_lines = lock_write!(locked);
                res!(self.render_text_view(drawer, &text_lines));
            }
        }

        // Replace the text back into the TextBox.
        self.tview.atext = atext;

        res!(drawer.rend.reset_style(When::Later));

        if when == When::Now {
            res!(drawer.rend.flush());
        }
        Ok(())
    }
}

impl TextBox {

    pub fn new(
        cfg:    TextBoxConfig,
        tview:  TextView,
    )
        -> Outcome<Self>
    {
        if cfg.highlight_styles.len() == 0 {
            return Err(err!(
                "TextBoxConfig highlight_styles vector is empty.";
            Input, Invalid, Missing));
        }
        Ok(Self {
            cfg,
            tview,
            ..Default::default()
        })
    }

    pub fn is_editable(&self) -> bool {
        match &self.tview.vtyp {
            TextViewType::Editable(..) => true,
            _ => false,
        }
    }

    /// Pan the text view up, if possible.  This is infallible with the use of saturating
    /// arithmetic.
    pub fn pan_up(&mut self, by: Dim) {
        if let Some(cursor) = self.tview.vtyp.get_cursor_mut() {
            let rel_cursor = *cursor - self.tview.text_view.top_left;
            self.tview.text_view.dec_y(by);
            *cursor = self.tview.text_view.top_left + rel_cursor;
        }
    }

    /// Pan the text view down, if possible.  This is infallible with the use of saturating
    /// arithmetic.
    pub fn pan_down(&mut self, by: Dim) {
        if let Some(cursor) = self.tview.vtyp.get_cursor_mut() {
            let dy = by.min(self.tview.extent.y
                + 1
                - self.tview.text_view.top_left.y
                - self.tview.text_view.size.y
            );
            let rel_cursor = *cursor - self.tview.text_view.top_left;
            self.tview.text_view.inc_y(dy);
            *cursor = self.tview.text_view.top_left + rel_cursor;
        }
    }

    /// Pan the text view left, if possible.  This is infallible with the use of saturating
    /// arithmetic.
    pub fn pan_left(&mut self, by: Dim) {
        if let Some(cursor) = self.tview.vtyp.get_cursor_mut() {
            let rel_cursor = *cursor - self.tview.text_view.top_left;
            self.tview.text_view.dec_x(by);
            *cursor = self.tview.text_view.top_left + rel_cursor;
        }
    }

    /// Pan the text view right, if possible.  This is infallible with the use of saturating
    /// arithmetic.
    pub fn pan_right(&mut self, by: Dim) {
        if let Some(cursor) = self.tview.vtyp.get_cursor_mut() {
            let dx = by.min(self.tview.extent.x
                + 1
                - self.tview.text_view.top_left.x
                - self.tview.text_view.size.x
            );
            let rel_cursor = *cursor - self.tview.text_view.top_left;
            self.tview.text_view.inc_x(dx);
            *cursor = self.tview.text_view.top_left + rel_cursor;
        }
    }

    /// Move the cursor up one line, if possible.  This is infallible with the use of saturating
    /// arithmetic.
    pub fn cursor_up(&mut self) {
        if let Some(cursor) = self.tview.vtyp.get_cursor_mut() {
            if self.cfg.cursor_is_visible() && cursor.y > Dim(0) {
                // Order important here, need to test whether cursor is at top of text view before
                // moving it.
                if cursor.y == self.tview.text_view.top() {
                    self.tview.text_view.dec_y(Dim(1));
                }
                cursor.dec_y(Dim(1));
            }
        }
    }

    /// Move the cursor down one line, if possible.  This is infallible with the use of saturating
    /// arithmetic.
    pub fn cursor_down(&mut self) {
        if let Some(cursor) = self.tview.vtyp.get_cursor_mut() {
            if self.cfg.cursor_is_visible() && cursor.y < self.tview.extent.y {
                cursor.inc_y(Dim(1));
                if cursor.y == self.tview.text_view.bottom() {
                    self.tview.text_view.inc_y(Dim(1));
                }
            }
        }
    }

    /// Move the cursor left one character, if possible.  This is method is infallible.
    pub fn cursor_left(&mut self) {
        if let Some(cursor) = self.tview.vtyp.get_cursor_mut() {
            if self.cfg.cursor_is_visible() && cursor.x > Dim(0) {
                // Order important here, need to test whether cursor is at the left of the text view
                // before moving it.
                if cursor.x == self.tview.text_view.left() {
                    self.tview.text_view.dec_x(Dim(1));
                }
                cursor.dec_x(Dim(1));
            }
        }
    }

    /// Move the cursor right one character, if possible.  This is method is infallible.
    pub fn cursor_right(&mut self) {
        if let Some(cursor) = self.tview.vtyp.get_cursor_mut() {
            if self.cfg.cursor_is_visible() && cursor.x < self.tview.extent.x {
                cursor.inc_x(Dim(1));
                if cursor.x == self.tview.text_view.right() {
                    self.tview.text_view.inc_x(Dim(1));
                }
            }
        }
    }

    /// Returns the `ScrollBars` and scrollbar widths.
    pub fn scrollbars(
        &self,
        cfg:        ScrollBarsConfig,
        term_view:  AbsRect,
    )
        -> (ScrollBars, AbsSize)
    {
        let (x, y, mut w, mut h) = term_view.tup();
        let (start_x, start_y) = self.tview.text_view.top_left.tup();
        let (extent_x, extent_y) = self.tview.extent.tup();
        if cfg.always {
            w = w - 1;
            h = h - 1;
            (
                ScrollBars {
                    cfg,
                    x: Some(ScrollBar {
                        top_left:   Coord::new((x, y + h)),
                        scroll_len: w,
                        tview:      TextViewDim::new(
                            start_x,
                            w,
                            extent_x - start_x - w,
                        ),
                    }),
                    y: Some(ScrollBar {
                        top_left:   Coord::new((x + w, y)),
                        scroll_len: h,
                        tview:      TextViewDim::new(
                            start_y,
                            h,
                            extent_y - start_y - h,
                        ),
                    }),
                },
                AbsSize::from((Dim(1), Dim(1))),
            )
        } else {
            let mut x_scrollbar = false;
            let mut y_scrollbar = false;
            let (wext, hext) = self.tview.extent.tup();
            if wext > w {
                h = h - 1;
                x_scrollbar = true;
            }
            if hext > h {
                w = w - 1;
                y_scrollbar = true;
            }
            (
                ScrollBars {
                    cfg,
                    x: if x_scrollbar {
                        Some(ScrollBar {
                            top_left:   Coord::new((x, y + h + 1)),
                            scroll_len: w,
                            tview:      TextViewDim::new(
                                start_x,
                                w,
                                extent_x - start_x - w,
                            ),
                        })
                    } else {
                        None
                    },
                    y: if y_scrollbar {
                        Some(ScrollBar {
                            top_left:   Coord::new((x + w + 1, y)),
                            scroll_len: h,
                            tview:      TextViewDim::new(
                                start_y,
                                h,
                                extent_y - start_y - h,
                            ),
                        })
                    } else {
                        None
                    },
                },
                AbsSize::from((
                    Dim(if y_scrollbar { 1 } else { 0 }),
                    Dim(if x_scrollbar { 1 } else { 0 }),
                )),
            )
        }
    }

    /// Get the rectangle defining the line focus, if it is visible in the text view.
    pub fn get_highlight(
        &self,
        text_lines: &TextLines<TextType, HighlightType>,
    )
        -> Outcome<Option<(Style, AbsRect)>>
    {
        if let Some(highlighter) = text_lines.get_highlighter() {
            if let Some(highlighted) = highlighter.get_highlighted() {
                let line_range = highlighted.get_range();
                match self.tview.text_view.clip(line_range.to_abs_rect()) {
                    Some(mut abs_rect) => {
                        abs_rect.top_left -= self.tview.text_view.top_left;
                        let style_index = try_rem!(
                            highlighted.get_level() as usize,
                            self.cfg.highlight_styles.len(),
                        );
                        return Ok(Some(
                            (
                                self.cfg.highlight_styles[style_index].clone(),
                                abs_rect,
                            )
                        ));
                    }
                    None => {}
                }
            }
        }
        Ok(None)
    }

    pub fn render_text_view<S: Sink, R: Renderer<S>>(
        &mut self,
        drawer:     &mut Drawer<S, R>,
        text_lines: &TextLines<TextType, HighlightType>,
    )
        -> Outcome<()>
    {
        // Take a snapshot of the text lines within the text view.
        let lines = TextView::extract_view(&text_lines.lines, &self.tview.text_view);
        // Identify whether the line focus range is within view, and if so adjust its
        // location to be relative to the view coordinates (where top left is (0,0).
        let mut highlight_opt = res!(self.get_highlight(&text_lines));
        let (x, y, _w, h) = self.tview.term_view.tup();

        for j in 0..h.as_index() {
            res!(drawer.rend.set_cursor(Coord::new((x, y + j)), When::Later));
            if let Some((style, empty_line_marker)) = &mut self.cfg.empty_line {
                if j < lines.len() {
                    res!(self.render_text_line(
                        drawer,
                        lines[j],
                        j,
                        &mut highlight_opt,
                    ));
                } else {
                    // Print empty line marker.
                    res!(style.render(drawer, When::Later));
                    res!(drawer.rend.print(&empty_line_marker, When::Later));
                }
            } else {
                // Print without empty line markers.
                if j < lines.len() {
                    res!(self.render_text_line(
                        drawer,
                        lines[j],
                        j,
                        &mut highlight_opt,
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn render_text_line<S: Sink, R: Renderer<S>>(
        &mut self,
        drawer:         &mut Drawer<S, R>,
        line:           &str,
        j:              usize,
        highlight_opt:  &mut Option<(Style, AbsRect)>,
    )
        -> Outcome<()>
    {
        if let Some((style, abs_rect)) = highlight_opt {
            let (xf, yf, wf, _hf) = abs_rect.tup();
            if j == yf {
                // Print with highlighting.
                let (pre, rest) = line.split_at(xf.as_index());
                let (focus, post) = rest.split_at(wf.as_index());
                res!(drawer.rend.print(pre, When::Later));
                res!(style.render(drawer, When::Later));
                res!(drawer.rend.print(focus, When::Later));
                res!(self.state.canvas.style.render(drawer, When::Later));
                res!(drawer.rend.print(post, When::Later));
                return Ok(());
            }
        }
        // Print as normal without any highlighting.
        res!(self.state.canvas.style.render(drawer, When::Later));
        res!(drawer.rend.print(line, When::Later));
        Ok(())
    }
}

