use crate::lib_tui::{
    draw::{
        outline::BorderManagementMode,
        window::{
            TextBoxesState,
            WindowId,
        },
    },
    event::AppFlow,
    text::{
        edit::EditorMode,
        typ::{
            ContentType,
            HighlightType,
            TextViewType,
        },
    },
    window::{
        WindowManager,
        WindowMode,
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
        RectSide,
        RectView,
    },
};


#[derive(Clone, Copy, Debug, Default)]
pub enum Action {
    // All Modes
    MoveCurrentMenuFocusUp,
    MoveCurrentMenuFocusDown,
    ExecuteCurrentMenuAction,
    ExitApp,
    // Navigation
    #[default]
    MoveToNextWindow,
    EnterWindow,
    EnterWindowManagementMode,
    CreateNewWindow,
    // Window Management
    EnterBorderManagementMode,
    SlideWindowUp,
    SlideWindowDown,
    SlideWindowRight,
    SlideWindowLeft,
    CreateWindow,
    DeleteWindow,
    ReturnToWindowNavigationMode,
    // Border Management
    MoveToNextBorder,
    DragBorderUp,
    DragBorderDown,
    DragBorderRight,
    DragBorderLeft,
    ReturnToWindowManagementMode,
    // Interaction
    MoveToNextTab,   
    MoveCursorUp,   
    MoveCursorDown,   
    MoveCursorRight,   
    MoveCursorLeft,   
    PanTextViewUp,
    PanTextViewDown,
    PanTextViewRight,
    PanTextViewLeft,
    EnterEditorInsertMode,
    EnterEditorReplaceMode,
    EnterEditorModifyMode,
    ReturnToEditorNavigationMode,
    //ReturnToWindowNavigationMode,
    // Supplementary interaction
    CreateNewTextTab,
}

#[derive(Clone, Debug, Default)]
pub struct ActionData {
    pub term:   Option<AbsRect>,
    pub c:      Option<char>,
}

impl ActionData {

    pub fn must_get_term(self) -> Outcome<AbsRect> {
        if let Some(term) = self.term {
            Ok(term)
        } else {
            Err(err!(
                "Terminal view is missing from ActionData.";
            Bug, Data, Missing))
        }
    }

    pub fn must_get_char(self) -> Outcome<char> {
        if let Some(c) = self.c {
            Ok(c)
        } else {
            Err(err!(
                "Character is missing from ActionData.";
            Bug, Data, Missing))
        }
    }
}

impl WindowManager<'_> {

    pub fn must_get_term(data: Option<ActionData>) -> Outcome<AbsRect> {
        if let Some(data) = data {
            data.must_get_term()
        } else {
            Err(err!(
                "ActionData is missing.";
            Bug, Data, Missing))
        }
    }

    pub fn must_get_char(data: Option<ActionData>) -> Outcome<char> {
        if let Some(data) = data {
            data.must_get_char()
        } else {
            Err(err!(
                "ActionData is missing.";
            Bug, Data, Missing))
        }
    }

    pub fn act(
        &mut self,
        action: &Action,
        data:   Option<ActionData>,
    )
        -> Outcome<AppFlow>
    {
        match action {
            // ┌─────────────────────────────┐
            // │ ALL MODES                   │
            // └─────────────────────────────┘
            Action::MoveCurrentMenuFocusUp => {
                let result = self.get_window_by_id_mut(&WindowId::Menu);
                let win = res!(result);
                if let Some(tbox) = win.state.text_boxes.get_text_box_mut() {
                    {
                        let result = tbox.tview.atext.get_text_lines_mut();
                        let mut text_lines = res!(result);
                        if let Some(highlighter) = text_lines.get_highlighter_mut() {
                            highlighter.dec_focus();
                        }
                    }
                    let _ = tbox.cursor_up();
                }
                return Ok(AppFlow::FullScreenRender);
            }
            Action::MoveCurrentMenuFocusDown => {
                let result = self.get_window_by_id_mut(&WindowId::Menu);
                let win = res!(result);
                if let Some(tbox) = win.state.text_boxes.get_text_box_mut() {
                    {
                        let result = tbox.tview.atext.get_text_lines_mut();
                        let mut text_lines = res!(result);
                        if let Some(highlighter) = text_lines.get_highlighter_mut() {
                            highlighter.inc_focus();
                        }
                    }
                    tbox.cursor_down();
                }
                return Ok(AppFlow::FullScreenRender);
            }
            Action::ExecuteCurrentMenuAction => {
                let result = self.get_window_by_id_mut(&WindowId::Menu);
                let win = res!(result);
                let mut execute_action = None;
                if let Some(tbox) = win.state.text_boxes.get_text_box() {
                    let text_lines = res!(tbox.tview.atext.get_text_lines());
                    if let Some(highlighter) = text_lines.get_highlighter() {
                        if let Some(highlight) = highlighter.get_highlighted() {
                            if let Some(HighlightType::Menu(action)) = highlight.get_data() {
                                execute_action = Some(action.clone());
                            }
                        }
                    }
                }
                if let Some(action) = execute_action {
                    return self.act(&action, data);
                }
                return Ok(AppFlow::FullScreenRender);
            }
            Action::ExitApp => {
                return Ok(AppFlow::Quit);
            }
            // ┌─────────────────────────────┐
            // │ NAVIGATION MODE             │
            // └─────────────────────────────┘
            Action::MoveToNextWindow => {
                self.next_focus();
                return Ok(AppFlow::FullScreenRender);
            }
            Action::EnterWindow => {
                if self.key_state.is_empty() {
                    res!(self.set_mode_update_menu(WindowMode::Interaction, None));
                } else {
                    let buf = self.key_state.buffer().clone();
                    match self.set_focus_by_label(&buf) {
                        Ok(()) => {
                            res!(self.set_mode_update_menu(WindowMode::Interaction, None));
                        }
                        Err(_) => {}
                    }
                    self.key_state.clear();
                }
                return Ok(AppFlow::FullScreenRender);
            }
            Action::EnterWindowManagementMode => {
                res!(self.set_mode_update_menu(WindowMode::WindowManagement, None));
                let result = self.get_focal_window_mut();
                let win = res!(result);
                // If the window has a relative specification switch it to an
                // absolute one.
                let term = ok!(Self::must_get_term(data));
                if let RectView::InitiallyRelative(_rel_rect) = &mut win.view {
                    match win.view.relative_to(term) {
                        Some(view) => win.view = RectView::Float(view),
                        _ => {}
                    }
                }
                return Ok(AppFlow::FocalWindowRender);
            }
            Action::CreateNewWindow => {
                let id = res!(self.next_id());
                let top_left = Coord::from((
                    self.state.new_win_pos.0.next(),
                    self.state.new_win_pos.1.next(),
                ));
                let (cfg, view, state) = res!(self.style_lib.default_new_window(
                    &self.cfg,
                    &id,
                    top_left,
                ));
                res!(self.add_window(Some(id.clone()), cfg, view));
                res!(self.set_state(&id, state));
                res!(self.set_focus_by_id(&id));
                return Ok(AppFlow::FullScreenRender);
            }
            // ┌─────────────────────────────┐
            // │ WINDOW MANAGEMENT MODE      │
            // └─────────────────────────────┘
            Action::EnterBorderManagementMode => {
                let result = self.get_focal_window_mut();
                let win = res!(result);
                // Deforming the window requires that it be specified absolutely in the
                // terminal frame.
                if let RectView::Float(_abs_rect) = win.view {
                    win.state.lines.mode = Some(BorderManagementMode::Adjust);
                    res!(self.set_mode_update_menu(WindowMode::BorderManagement, None));
                }
                return Ok(AppFlow::FocalWindowRender);
            }
            Action::SlideWindowUp => {
                let result = self.get_focal_window_mut();
                let win = res!(result);
                match &mut win.view {
                    // Translate entire window upward, if possible.
                    RectView::Float(abs_rect) => {
                        if abs_rect.top() > Dim(0) {
                            abs_rect.dec_y(Dim(1));
                        }
                    }
                    _ => {}
                }
                return Ok(AppFlow::FullScreenRender);
            }
            Action::SlideWindowDown => {
                let result = self.get_focal_window_mut();
                let win = res!(result);
                match &mut win.view {
                    // Translate entire window downward, if possible.
                    RectView::Float(abs_rect) => {
                        let term = ok!(Self::must_get_term(data));
                        if abs_rect.bottom() < term.bottom() {
                            abs_rect.inc_y(Dim(1));
                        }
                    }
                    _ => {}
                }
                return Ok(AppFlow::FullScreenRender);
            }
            Action::SlideWindowRight => {
                let result = self.get_focal_window_mut();
                let win = res!(result);
                match &mut win.view {
                    // Translate entire window rightward, if possible.
                    RectView::Float(abs_rect) => {
                        let term = ok!(Self::must_get_term(data));
                        if abs_rect.right() < term.right() {
                            abs_rect.inc_x(Dim(1));
                        }
                    }
                    _ => {}
                }
                return Ok(AppFlow::FullScreenRender);
            }
            Action::SlideWindowLeft => {
                let result = self.get_focal_window_mut();
                let win = res!(result);
                match &mut win.view {
                    // Translate entire window leftward, if possible.
                    RectView::Float(abs_rect) => {
                        if abs_rect.left() > Dim(0) {
                            abs_rect.dec_x(Dim(1));
                        }
                    }
                    _ => {}
                }
                return Ok(AppFlow::FullScreenRender);
            }
            Action::CreateWindow => {
                return Ok(AppFlow::FullScreenRender);
            }
            Action::DeleteWindow => {
                return Ok(AppFlow::FullScreenRender);
            }
            Action::ReturnToWindowNavigationMode => {
                res!(self.set_mode_update_menu(WindowMode::Navigation, None));
                self.key_state.clear();
                return Ok(AppFlow::FocalWindowRender);
            }
            // ┌─────────────────────────────┐
            // │ BORDER MANAGEMENT MODE      │
            // └─────────────────────────────┘
            Action::MoveToNextBorder => {
                let result = self.get_focal_window_mut();
                let win = res!(result);
                match &mut win.state.lines.mode {
                    Some(BorderManagementMode::Adjust) => {
                        win.state.lines.line = win.state.lines.iter.next();
                    }
                    _ => {}
                }
                return Ok(AppFlow::FocalWindowRender);
            }
            Action::DragBorderUp => {
                let result = self.get_focal_window_mut();
                let win = res!(result);
                match win.state.lines.mode {
                    Some(BorderManagementMode::Adjust) => {
                        match win.state.lines.line {
                            // Move bottom line upward, if possible.
                            RectSide::Bottom => {
                                match &mut win.view {
                                    RectView::Float(abs_rect) => {
                                        if abs_rect.height() > win.cfg.min_size.y {
                                            abs_rect.dec_h(Dim(1));
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            // Move top line upward, if possible.
                            RectSide::Top => {
                                match &mut win.view {
                                    RectView::Float(abs_rect) => {
                                        if abs_rect.top() > Dim(0) {
                                            abs_rect.dec_y(Dim(1));
                                            abs_rect.inc_h(Dim(1));
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
                return Ok(AppFlow::FullScreenRender);
            }
            Action::DragBorderDown => {
                let result = self.get_focal_window_mut();
                let win = res!(result);
                match win.state.lines.mode {
                    Some(BorderManagementMode::Adjust) => {
                        match win.state.lines.line {
                            // Move bottom line downward, if possible.
                            RectSide::Bottom => {
                                match &mut win.view {
                                    RectView::Float(abs_rect) => {
                                        let term = ok!(Self::must_get_term(data));
                                        if abs_rect.bottom() < term.bottom() {
                                            abs_rect.inc_h(Dim(1));
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            // Move top line downward, if possible.
                            RectSide::Top => {
                                match &mut win.view {
                                    RectView::Float(abs_rect) => {
                                        if abs_rect.height() > win.cfg.min_size.y {
                                            abs_rect.inc_y(Dim(1));
                                            abs_rect.dec_h(Dim(1));
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
                return Ok(AppFlow::FullScreenRender);
            }
            Action::DragBorderRight => {
                let result = self.get_focal_window_mut();
                let win = res!(result);
                match win.state.lines.mode {
                    Some(BorderManagementMode::Adjust) => {
                        match win.state.lines.line {
                            // Move right line rightward, if possible.
                            RectSide::Right => {
                                match &mut win.view {
                                    RectView::Float(abs_rect) => {
                                        let term = ok!(Self::must_get_term(data));
                                        if abs_rect.right() < term.right() {
                                            abs_rect.inc_w(Dim(1));
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            // Move left line rightward, if possible.
                            RectSide::Left => {
                                match &mut win.view {
                                    RectView::Float(abs_rect) => {
                                        if abs_rect.width() > win.cfg.min_size.x {
                                            abs_rect.inc_x(Dim(1));
                                            abs_rect.dec_w(Dim(1));
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
                return Ok(AppFlow::FullScreenRender);
            }
            Action::DragBorderLeft => {
                let result = self.get_focal_window_mut();
                let win = res!(result);
                match win.state.lines.mode {
                    Some(BorderManagementMode::Adjust) => {
                        match win.state.lines.line {
                            // Move right line leftward, if possible.
                            RectSide::Right => {
                                match &mut win.view {
                                    RectView::Float(abs_rect) => {
                                        if abs_rect.width() > win.cfg.min_size.x {
                                            abs_rect.dec_w(Dim(1));
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            // Move left line leftward, if possible.
                            RectSide::Left => {
                                match &mut win.view {
                                    RectView::Float(abs_rect) => {
                                        if abs_rect.left() > Dim(0) {
                                            abs_rect.dec_x(Dim(1));
                                            abs_rect.inc_w(Dim(1));
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
                return Ok(AppFlow::FullScreenRender);
            }
            Action::ReturnToWindowManagementMode => {
                let result = self.get_focal_window_mut();
                let win = res!(result);
                win.state.lines.mode = None;
                res!(self.set_mode_update_menu(WindowMode::WindowManagement, None));
                self.key_state.clear();
                return Ok(AppFlow::FocalWindowRender);
            }
            // ┌─────────────────────────────┐
            // │ INTERACTION MODE            │
            // └─────────────────────────────┘
            Action::MoveToNextTab => {
                let tab_string = self.cfg.tab_string.clone();
                let result = self.get_focal_window_mut();
                let win = res!(result);
                if let Some(tbox) = win.get_focal_text_box_mut() {
                    match &mut tbox.tview.vtyp {
                        TextViewType::Editable(editor) => match editor.mode {
                            EditorMode::Navigation => {
                                if let TextBoxesState::Tabbed(ttmgr) = &mut win.state.text_boxes {
                                    res!(ttmgr.next_focus());
                                }
                            }
                            EditorMode::Insert | EditorMode::Replace => {
                                let result = tbox.tview.atext.get_text_lines_mut();
                                let mut text_lines = res!(result);
                                text_lines.add_str(
                                    &mut editor.nav.cursor,
                                    tab_string.as_str(),
                                    editor.mode == EditorMode::Replace,
                                );
                            }
                            _ => {}
                        }
                        _ => {
                            if let TextBoxesState::Tabbed(ttmgr) = &mut win.state.text_boxes {
                                res!(ttmgr.next_focus());
                            }
                        }
                    }
                }
                return Ok(AppFlow::FocalWindowRender);
            }
            Action::MoveCursorUp => {
                let result = self.get_focal_window_text_box_mut();
                if let Some(tbox) = res!(result) {
                    {
                        let result = tbox.tview.atext.get_text_lines_mut();
                        let mut text_lines = res!(result);
                        if let Some(highlighter) = text_lines.get_highlighter_mut() {
                            highlighter.dec_focus();
                        }
                    }
                    if tbox.cursor_up() {
                        return Ok(AppFlow::FocalWindowRender);
                    } else {
                        return Ok(AppFlow::CursorOnly);
                    }
                } else {
                    return Ok(AppFlow::CursorOnly);
                }
            }   
            Action::MoveCursorDown => {
                let result = self.get_focal_window_text_box_mut();
                if let Some(tbox) = res!(result) {
                    {
                        let result = tbox.tview.atext.get_text_lines_mut();
                        let mut text_lines = res!(result);
                        if let Some(highlighter) = text_lines.get_highlighter_mut() {
                            highlighter.inc_focus();
                        }
                    }
                    if tbox.cursor_down() {
                        return Ok(AppFlow::FocalWindowRender);
                    } else {
                        return Ok(AppFlow::CursorOnly);
                    }
                } else {
                    return Ok(AppFlow::CursorOnly);
                }
            }   
            Action::MoveCursorRight => {
                let result = self.get_focal_window_text_box_mut();
                if let Some(tbox) = res!(result) {
                    if tbox.cursor_right() {
                        return Ok(AppFlow::FocalWindowRender);
                    } else {
                        return Ok(AppFlow::CursorOnly);
                    }
                } else {
                    return Ok(AppFlow::CursorOnly);
                }
            }
            Action::MoveCursorLeft => {
                let result = self.get_focal_window_text_box_mut();
                if let Some(tbox) = res!(result) {
                    if tbox.cursor_left() {
                        return Ok(AppFlow::FocalWindowRender);
                    } else {
                        return Ok(AppFlow::CursorOnly);
                    }
                } else {
                    return Ok(AppFlow::CursorOnly);
                }
            }
            Action::PanTextViewUp => {
                let result = self.get_focal_window_text_box_mut();
                if let Some(tbox) = res!(result) {
                    tbox.pan_up(Dim(2));
                }
                return Ok(AppFlow::FocalWindowRender);
            }
            Action::PanTextViewDown => {
                let result = self.get_focal_window_text_box_mut();
                if let Some(tbox) = res!(result) {
                    tbox.pan_down(Dim(2));
                }
                return Ok(AppFlow::FocalWindowRender);
            }
            Action::PanTextViewRight => {
                let result = self.get_focal_window_text_box_mut();
                if let Some(tbox) = res!(result) {
                    tbox.pan_right(Dim(3));
                }
                return Ok(AppFlow::FocalWindowRender);
            }
            Action::PanTextViewLeft => {
                let result = self.get_focal_window_text_box_mut();
                if let Some(tbox) = res!(result) {
                    tbox.pan_left(Dim(3));
                }
                return Ok(AppFlow::FocalWindowRender);
            }
            Action::EnterEditorInsertMode => {
                let result = self.get_focal_window_mut();
                let win = res!(result);
                if let Some(tbox) = win.get_focal_text_box_mut() {
                    if let Some(editor) = tbox.tview.vtyp.get_editor_mut() {
                        match editor.mode {
                            EditorMode::Navigation => editor.mode = EditorMode::Insert,
                            _ => {}
                        }
                    }
                }
                return Ok(AppFlow::FocalWindowRender);
            }
            Action::EnterEditorReplaceMode => {
                let result = self.get_focal_window_mut();
                let win = res!(result);
                if let Some(tbox) = win.get_focal_text_box_mut() {
                    if let Some(editor) = tbox.tview.vtyp.get_editor_mut() {
                        match editor.mode {
                            EditorMode::Navigation => editor.mode = EditorMode::Replace,
                            _ => {}
                        }
                    }
                }
                return Ok(AppFlow::FocalWindowRender);
            }
            Action::EnterEditorModifyMode => {
                let result = self.get_focal_window_mut();
                let win = res!(result);
                if let Some(tbox) = win.get_focal_text_box_mut() {
                    if let Some(editor) = tbox.tview.vtyp.get_editor_mut() {
                        match editor.mode {
                            EditorMode::Navigation => editor.mode = EditorMode::Modify,
                            _ => {}
                        }
                    }
                }
                return Ok(AppFlow::FocalWindowRender);
            }
            Action::ReturnToEditorNavigationMode => {
                let result = self.get_focal_window_text_box_mut();
                if let Some(tbox) = res!(result) {
                    match &mut tbox.tview.vtyp {
                        TextViewType::Editable(editor) => {
                            editor.mode = EditorMode::Navigation;
                        }
                        _ => {}
                    }
                }
                self.key_state.clear();
                return Ok(AppFlow::FocalWindowRender);
            }
            Action::CreateNewTextTab => {
                let id = self.get_focus_id().clone();
                match self.add_text_box(
                    &id,
                    res!(self.style_lib.tabbed_text_box(ContentType::Text)),
                ) {
                    Ok(()) => {}
                    Err(e) => error!(e),
                }
                return Ok(AppFlow::FocalWindowRender);
            }
            //_ => {}
        }
        //Ok(AppFlow::FocalWindowRender)
    }
}
