use crate::{
    draw::{
        canvas::CanvasConfig,
        outline::{
            Outline,
            OutlineConfigs,
            OutlineState,
        },
        status::{
            StatusStrip,
            StatusStripConfig,
            StatusStripContent,
            StatusStripRight,
            StatusStripType,
        },
        tab::TabbedTextManager,
        tbox::TextBox,
    },
    render::{
        Drawable,
        Drawer,
        Renderer,
        Sink,
        When,
    },
    text::{
        typ::{
            HighlightType,
            TextType,
        },
    },
    window::WindowMode,
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
        RectView,
    },
};
use oxedize_fe2o3_text::{
    lines::TextLines,
};


#[derive(Clone, Debug, Default)]
pub struct WindowConfig {
    pub typ:        WindowType,
    pub canvas:     CanvasConfig, // Default canvas config, overridden by tabs.
    pub outlines:   OutlineConfigs,
    pub header:     Option<StatusStripConfig>,
    pub footer:     Option<StatusStripConfig>,
    pub min_size:   AbsSize,
    pub menu_text:  Option<TextLines<TextType, HighlightType>>,
}

#[derive(Clone, Debug)]
pub enum TextBoxesState {
    Single(TextBox),
    Tabbed(TabbedTextManager),
}

impl TextBoxesState {
    pub fn get_text_box(&self) -> Option<&TextBox> {
        match self {
            Self::Single(tbox) => Some(&tbox),
            Self::Tabbed(tmgr) => {
                match tmgr.get_focal_tabbed_text_box() {
                    Some(ttbox) => Some(&ttbox.tbox),
                    None => None,
                }
            }
        }
    }
    pub fn get_text_box_mut(&mut self) -> Option<&mut TextBox> {
        match self {
            Self::Single(tbox) => Some(tbox),
            Self::Tabbed(tmgr) => {
                match tmgr.get_focal_tabbed_text_box_mut() {
                    Some(ttbox) => Some(&mut ttbox.tbox),
                    None => None,
                }
            }
        }
    }
}

impl Default for TextBoxesState {
    fn default() -> Self {
        Self::Single(TextBox::default())
    }
}

impl TextBoxesState {
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut TextBox> {
        match self {
            Self::Single(tbox) => {
                vec![tbox].into_iter()
            }
            Self::Tabbed(tmgr) => {
                tmgr.iter_mut().collect::<Vec<_>>().into_iter()
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct WindowState {
    pub header:     StatusStripContent,
    pub footer:     StatusStripContent,
    pub text_boxes: TextBoxesState,
    pub lines:      OutlineState,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum WindowType {
    #[default]
    Fixed,
    Dynamic,
}

impl From<WindowStateInit> for WindowState {
    fn from(wsi: WindowStateInit) -> Self {
        Self {
            header:     wsi.header.unwrap_or(StatusStripContent::default()),
            footer:     wsi.footer.unwrap_or(StatusStripContent::default()),
            text_boxes: wsi.text_boxes.unwrap_or(TextBoxesState::default()),
            lines:      OutlineState::default(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct WindowStateInit {
    pub header:     Option<StatusStripContent>,
    pub footer:     Option<StatusStripContent>,
    pub text_boxes: Option<TextBoxesState>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum WindowId {
    Menu,
    Help,
    Log,
    User(u8),
}

impl Default for WindowId {
    fn default() -> Self {
        Self::User(3)
    }
}

impl WindowId {
    pub fn title(&self) -> &str {
        match self {
            Self::Menu      => "Menu",
            Self::Help      => "Help",
            Self::Log       => "Log",
            Self::User(_)   => "User",
        }
    }
    pub fn label(&self) -> String {
        match self {
            Self::Menu          => fmt!("0"),
            Self::Help          => fmt!("1"),
            Self::Log           => fmt!("2"),
            Self::User(label)   => fmt!("{}", label),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Window {
    pub cfg:    WindowConfig,
    pub focus:  bool,
    pub mode:   WindowMode,
    pub view:   RectView, // The outer extent of the window.
    pub id:     WindowId,
    pub state:  WindowState,
    pub layer:  u8,
}

impl Drawable for Window {
    fn render<S: Sink, R: Renderer<S>>(
        &mut self,
        drawer: &mut Drawer<S, R>,
        when:   When,
    )
        -> Outcome<()>
    {
        let term = AbsRect::from(res!(drawer.rend.size()));
        let win_view = match self.view.relative_to(term) {
            Some(view) => view,
            None => return Ok(()), // Window is outside terminal view.
        };

        // Initialise window extent.  We'll progressively draw some components and modify these
        // metrics to reveal the final viewing area.
        let (mut x, mut y, mut w, mut h) = win_view.tup();

        // Account for outline.
        if w > Dim(2) && h > Dim(2) {
            x += 1;
            y += 1;
            w -= 2;
            h -= 2;
        }

        // Account for header and draw if necessary.
        if let Some(cfg) = &self.cfg.header {
            let content = self.clone_and_update_status_strip(StatusStripType::Header);
            let mut header = StatusStrip::new(
                cfg.clone(),
                content,
                AbsRect::from((x, y, w, h)),
            );
            res!(header.render(drawer, When::Later));
            y += 1;
            h -= 1;
        }

        // Account for footer and draw if necessary.
        if let Some(cfg) = &self.cfg.footer {
            let content = self.clone_and_update_status_strip(StatusStripType::Footer);
            let mut footer = StatusStrip::new(
                cfg.clone(),
                content,
                AbsRect::from((x, y, w, h)),
            );
            res!(footer.render(drawer, When::Later));
            h -= 1;
        }

        // Account for a tab strip and draw if necessary.
        if let TextBoxesState::Tabbed(ttmgr) = &mut self.state.text_boxes {
            if !ttmgr.is_empty() {
                res!(ttmgr.update_tabs(AbsRect::from((x, y, w, h))));
                res!(ttmgr.render(drawer, When::Later));
                let dh = ttmgr.strip_height();
                y += dh;
                h -= dh;
            }
        }

        // This is the viewing area.
        let content_view = AbsRect::from((x, y, w, h));

        // Draw the outline if necessary.
        let mut outline = Outline {
            cfgs:   &self.cfg.outlines,
            view:   win_view.clone(),
            state:  &self.state.lines,
            focus:  self.focus,
            wmode:  self.mode,
        };
        res!(outline.render(drawer, When::Later));

        // Display the text view.  We are either drawing a single tex box, with or without an
        // associated one-row tab strip, or multiple tiled text boxes under two rows of tabs.
        match &mut self.state.text_boxes {
            TextBoxesState::Single(tbox) => {
                tbox.view = content_view;
                res!(tbox.render(drawer, When::Later));
            }
            TextBoxesState::Tabbed(ttmgr) => {
                let focus_is_on_tiled = ttmgr.focus_is_on_tiled();
                match &mut ttmgr.tiled {
                    Some(tiled) => {
                        if focus_is_on_tiled {
                            let mut start_x = x;
                            for tabbed_tile in tiled {
                                let ttbox = &mut ttmgr.tboxes[tabbed_tile.index];
                                let wt = tabbed_tile.width;
                                ttbox.tbox.view = AbsRect::from((start_x, y, wt, h));
                                res!(ttbox.tbox.render(drawer, When::Later));
                                start_x += wt;
                            }
                        } else {
                            if let Some(ttbox) = ttmgr.get_focal_tabbed_text_box_mut() {
                                ttbox.tbox.view = content_view;
                                res!(ttbox.tbox.render(drawer, When::Later));
                            }
                        }
                    }
                    None => {
                        if let Some(ttbox) = ttmgr.get_focal_tabbed_text_box_mut() {
                            ttbox.tbox.view = content_view;
                            res!(ttbox.tbox.render(drawer, When::Later));
                        }
                    }
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

impl Window {

    pub fn new(
        id:     WindowId,
        cfg:    WindowConfig,
        view:   RectView,
        state:  Option<WindowStateInit>,
    )
        -> Outcome<Self>
    {
        if cfg.typ == WindowType::Dynamic {
            if let RectView::AlwaysRelative(_) = view {
                return Err(err!(errmsg!(
                    "A dynamic window view cannot be set as {:?}.", view,
                    ), Input, Invalid));
            }
        }
        Ok(Self {
            id,
            cfg,
            view,
            state: match state {
                Some(wsi) => WindowState::from(wsi),
                None => WindowState::default(),
            },
            ..Default::default()
        })
    }

    pub fn set_state(&mut self, state:WindowStateInit) {
        self.state = WindowState::from(state);
    }

    pub fn set_window_focus(&mut self, focus: bool) {
        self.focus = focus;
    }

    pub fn set_window_mode(&mut self, mode: WindowMode) {
        self.mode = mode;
    }

    pub fn get_focal_text_box(&self) -> Option<&TextBox> {
        match &self.state.text_boxes {
            TextBoxesState::Single(tbox) => Some(&tbox),
            TextBoxesState::Tabbed(ttmgr) => match ttmgr.get_focal_tabbed_text_box() {
                Some(ttbox) => Some(&ttbox.tbox),
                None => None,
            }
        }
    }

    pub fn get_focal_text_box_mut(&mut self) -> Option<&mut TextBox> {
        match &mut self.state.text_boxes {
            TextBoxesState::Single(tbox) => Some(tbox),
            TextBoxesState::Tabbed(ttmgr) => match ttmgr.get_focal_tabbed_text_box_mut() {
                Some(ttbox) => Some(&mut ttbox.tbox),
                None => None,
            }
        }
    }

    pub fn clone_and_update_status_strip(
        &self,
        typ: StatusStripType,
    )
        -> StatusStripContent
    {
        let mut content = match typ {
            StatusStripType::Header => self.state.header.clone(),
            StatusStripType::Footer => self.state.footer.clone(),
        };
        // The left side currently contains an optional origin, which does not need to be
        // updated frequently.
        if let Some(tbox) = self.get_focal_text_box() {
            match &mut content.right {
                Some(right) => match right {
                    StatusStripRight::Cursor(coord) => {
                        if let Some(cursor) = tbox.tview.vtyp.get_cursor() {
                            *coord = *cursor + Coord::from((1, 1));
                        }
                    }
                    StatusStripRight::Label(label) => {
                        *label = self.id.label();
                    }
                    StatusStripRight::Mode(mode_opt) => {
                        *mode_opt = if self.mode == WindowMode::Interaction {
                            if let Some(editor) = tbox.tview.vtyp.get_editor() {
                                Some(editor.mode)
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                    }
                    StatusStripRight::ModeLabel(mode_opt, label) => {
                        *mode_opt = if self.mode == WindowMode::Interaction {
                            if let Some(editor) = tbox.tview.vtyp.get_editor() {
                                Some(editor.mode)
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                        *label = self.id.label();
                    }
                }
                _ => {}
            }
        }
        content
    }

    pub fn create_tab(
        &mut self,
    )
        -> Outcome<()>
    {
        Ok(())
    }
}
