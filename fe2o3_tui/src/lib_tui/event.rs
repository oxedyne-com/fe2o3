use crate::lib_tui::{
    action::{
        Action,
        ActionData,
    },
    text::{
        edit::EditorMode,
        typ::TextViewType,
    },
    window::{
        WindowManager,
        WindowMode,
    },
};

use oxedize_fe2o3_core::prelude::*;

use crossterm::{
    event::{
        KeyCode,
        KeyEvent,
        KeyModifiers,
    },
};


#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppFlow {
    Quit,
    Continue,
    CursorOnly,
    FocalWindowRender,
    FullScreenRender,
}

#[derive(Clone, Debug, Default)]
pub struct KeyState {
    buf: String,
}

impl KeyState {

    pub fn buffer(&self) -> &String {
        &self.buf
    }

    pub fn clear(&mut self) {
        self.buf = String::new();
    }

    pub fn is_empty(&self) -> bool {
        self.buf.len() == 0
    }

    pub fn push(&mut self, c: char) {
        self.buf.push(c);
    }
}

impl WindowManager<'_> {

    pub fn native_key_handler(
        &mut self,
        key:    KeyEvent,
        data:   Option<ActionData>,
    )
        -> Outcome<AppFlow>
    {
        match key {
            // ┌─────────────────────────────┐
            // │ ALL MODES                   │
            // └─────────────────────────────┘
            // Ctrl-C: Exit app.
            KeyEvent { code: KeyCode::Char('c'), modifiers: KeyModifiers::CONTROL, .. } => {
                return self.act(&Action::ExitApp, data);
            }
            // Esc: Exiting modes.
            KeyEvent { code: KeyCode::Esc, modifiers: KeyModifiers::NONE, .. } => {
                match self.mode {
                    WindowMode::Interaction => {
                        // If the user is in editor navigation mode, Esc drops them out of editing
                        // and back to window navigation mode.  If the user is in another editor
                        // mode, they are dropped back to editor navigation mode.
                        let result = self.get_focal_window_text_box_mut();
                        if let Some(tbox) = res!(result) {
                            match &mut tbox.tview.vtyp {
                                TextViewType::Editable(editor) => {
                                    match editor.mode {
                                        EditorMode::Navigation => {
                                            res!(self.set_mode_update_menu(WindowMode::Navigation, None));
                                        }
                                        _ => {
                                            editor.mode = EditorMode::Navigation;
                                        }
                                    }
                                }
                                _ => {
                                    res!(self.set_mode_update_menu(WindowMode::Navigation, None));
                                }
                            }
                        }
                        self.key_state.clear();
                    }
                    WindowMode::WindowManagement => {
                        return self.act(&Action::ReturnToWindowNavigationMode, data);
                    }
                    WindowMode::BorderManagement => {
                        return self.act(&Action::ReturnToWindowManagementMode, data);
                    }
                    _ => {}
                }
            }
            // Shift + PageUp: Move menu selection up one line in current tab of menu window.
            KeyEvent { code: KeyCode::PageUp, modifiers: KeyModifiers::SHIFT, .. } => {
                return self.act(&Action::MoveCurrentMenuFocusUp, data);
            }
            // Shift + PageDown: Move menu selection down one line in current tab of menu window.
            KeyEvent { code: KeyCode::PageDown, modifiers: KeyModifiers::SHIFT, .. } => {
                return self.act(&Action::MoveCurrentMenuFocusDown, data);
            }
            // Home: Perform currently highlighted menu action.
            KeyEvent { code: KeyCode::Home, modifiers: KeyModifiers::NONE, .. } => {
                return self.act(&Action::ExecuteCurrentMenuAction, data);
            }
            // ┌─────────────────────────────┐
            // │ NAVIGATION MODE             │
            // └─────────────────────────────┘
            // Tab: Rotate the focus amongst windows.
            KeyEvent { code: KeyCode::Tab, modifiers: KeyModifiers::NONE, .. }
                if self.mode == WindowMode::Navigation =>
            {
                return self.act(&Action::MoveToNextWindow, data);
            }
            // Enter:
            // - Transition from window navigation to interaction mode (enter the window).
            // - If the key buffer is not empty, try and jump straight into the appropriate window.
            KeyEvent { code: KeyCode::Enter, modifiers: KeyModifiers::NONE, .. }
                if self.mode == WindowMode::Navigation =>
            {
                return self.act(&Action::EnterWindow, data);
            }
            // Collect keys into the key buffer, to form a label which we can navigate to.
            KeyEvent { code: KeyCode::Char(c), modifiers: KeyModifiers::NONE, .. }
                if self.mode == WindowMode::Navigation
                    && c != ' ' =>
            {
                self.key_state.push(c);
            }
            // Space: Transition from window navigation to management mode.
            KeyEvent { code: KeyCode::Char(c), modifiers: KeyModifiers::NONE, .. }
                if self.mode == WindowMode::Navigation =>
            {
                match c {
                    ' ' => {
                        return self.act(&Action::EnterWindowManagementMode, data);
                    }
                    _ => {}
                }
            }
            // ┌─────────────────────────────┐
            // │ WINDOW MANAGEMENT MODE      │
            // └─────────────────────────────┘
            // Space: Transition to border management mode.
            KeyEvent { code: KeyCode::Char(c), modifiers: KeyModifiers::NONE, .. }
                if self.mode == WindowMode::WindowManagement =>
            {
                match c {
                    ' ' => {
                        return self.act(&Action::EnterBorderManagementMode, data);
                    }
                    _ => {}
                }
            }
            // Up: Translate window in window management mode.
            KeyEvent { code: KeyCode::Up, modifiers: KeyModifiers::NONE, .. }
                if self.mode == WindowMode::WindowManagement =>
            {
                return self.act(&Action::SlideWindowUp, data);
            }
            // Down: Translate window in window management mode.
            KeyEvent { code: KeyCode::Down, modifiers: KeyModifiers::NONE, .. }
                if self.mode == WindowMode::WindowManagement =>
            {
                return self.act(&Action::SlideWindowDown, data);
            }
            // Right: Translate window in window management mode.
            KeyEvent { code: KeyCode::Right, modifiers: KeyModifiers::NONE, .. }
                if self.mode == WindowMode::WindowManagement =>
            {
                return self.act(&Action::SlideWindowRight, data);
            }
            // Left: Translate window in window management mode.
            KeyEvent { code: KeyCode::Left, modifiers: KeyModifiers::NONE, .. }
                if self.mode == WindowMode::WindowManagement =>
            {
                return self.act(&Action::SlideWindowLeft, data);
            }
            // ┌─────────────────────────────┐
            // │ BORDER MANAGEMENT MODE      │
            // └─────────────────────────────┘
            // Space: Select next border.
            KeyEvent { code: KeyCode::Char(c), modifiers: KeyModifiers::NONE, .. }
                if self.mode == WindowMode::BorderManagement =>
            {
                match c {
                    ' ' => {
                        return self.act(&Action::MoveToNextBorder, data);
                    }
                    _ => {}
                }
            }
            // Up: Resize in vertical direction in border management mode.
            KeyEvent { code: KeyCode::Up, modifiers: KeyModifiers::NONE, .. }
                if self.mode == WindowMode::BorderManagement =>
            {
                return self.act(&Action::DragBorderUp, data);
            }
            // Down: Resize in vertical direction in border management mode.
            KeyEvent { code: KeyCode::Down, modifiers: KeyModifiers::NONE, .. }
                if self.mode == WindowMode::BorderManagement =>
            {
                return self.act(&Action::DragBorderDown, data);
            }
            // Right: Resize in horizontal direction in border management mode.
            KeyEvent { code: KeyCode::Right, modifiers: KeyModifiers::NONE, .. }
                if self.mode == WindowMode::BorderManagement =>
            {
                return self.act(&Action::DragBorderRight, data);
            }
            // Left: Resize in horizontal direction in border management mode.
            KeyEvent { code: KeyCode::Left, modifiers: KeyModifiers::NONE, .. }
                if self.mode == WindowMode::BorderManagement =>
            {
                return self.act(&Action::DragBorderLeft, data);
            }
            // ┌─────────────────────────────┐
            // │ INTERACTION MODE            │
            // └─────────────────────────────┘
            // Up: Move cursor up one line, if possible.
            KeyEvent { code: KeyCode::Up, modifiers: KeyModifiers::NONE, .. }
                if self.mode == WindowMode::Interaction =>
            {
                return self.act(&Action::MoveCursorUp, data);
            }
            // Down: Move cursor down one line, if possible.
            KeyEvent { code: KeyCode::Down, modifiers: KeyModifiers::NONE, .. }
                if self.mode == WindowMode::Interaction =>
            {
                return self.act(&Action::MoveCursorDown, data);
            }
            // Right: Move one character to the right, if possible.
            KeyEvent { code: KeyCode::Right, modifiers: KeyModifiers::NONE, .. }
                if self.mode == WindowMode::Interaction =>
            {
                return self.act(&Action::MoveCursorRight, data);
            }
            // Left: Move one character to the left, if possible.
            KeyEvent { code: KeyCode::Left, modifiers: KeyModifiers::NONE, .. }
                if self.mode == WindowMode::Interaction =>
            {
                return self.act(&Action::MoveCursorLeft, data);
            }
            // Shift Up: Pan text view up, if possible.
            KeyEvent { code: KeyCode::Up, modifiers: KeyModifiers::SHIFT, .. }
                if self.mode == WindowMode::Interaction =>
            {
                return self.act(&Action::PanTextViewUp, data);
            }
            // Shift Down: Pan text view down, if possible.
            KeyEvent { code: KeyCode::Down, modifiers: KeyModifiers::SHIFT, .. }
                if self.mode == WindowMode::Interaction =>
            {
                return self.act(&Action::PanTextViewDown, data);
            }
            // Shift Right: Pan text view right, if possible.
            KeyEvent { code: KeyCode::Right, modifiers: KeyModifiers::SHIFT, .. }
                if self.mode == WindowMode::Interaction =>
            {
                return self.act(&Action::PanTextViewRight, data);
            }
            // Shift Left: Pan text view left, if possible.
            KeyEvent { code: KeyCode::Left, modifiers: KeyModifiers::SHIFT, .. }
                if self.mode == WindowMode::Interaction =>
            {
                return self.act(&Action::PanTextViewLeft, data);
            }
            // Char:
            // - Transition from intra-window navigation to other editor modes.
            // - Character insertion in insert or replace editor modes.
            KeyEvent { code: KeyCode::Char(c), modifiers: KeyModifiers::NONE | KeyModifiers::SHIFT, .. }
                if self.mode == WindowMode::Interaction =>
            {
                let result = self.get_focal_window_mut();
                let win = res!(result);
                if let Some(tbox) = win.get_focal_text_box_mut() {
                    if let Some(editor) = tbox.tview.vtyp.get_editor_mut() {
                        match editor.mode {
                            EditorMode::Navigation => {
                                match c {
                                    'i' => editor.mode = EditorMode::Insert,
                                    'r' => editor.mode = EditorMode::Replace,
                                    'm' => editor.mode = EditorMode::Modify,
                                    _ => {}
                                }
                            }
                            EditorMode::Insert | EditorMode::Replace => {
                                let result = tbox.tview.atext.get_text_lines_mut();
                                let mut text_lines = res!(result);
                                text_lines.add_char(
                                    &mut editor.nav.cursor,
                                    c,
                                    editor.mode == EditorMode::Replace,
                                );
                            }
                            _ => {}
                        }
                    }
                }
            }
            // Shift + Tab doesn't work reliably.
            // - Rotate the focus amongst tabs.
            //KeyEvent { code: KeyCode::BackTab, modifiers: KeyModifiers::NONE, .. }
            //    if self.mode == WindowMode::Interaction =>
            //{
            //    debug!("Shift + Tab");
            //    let win = res!(self.get_focal_window_mut());
            //    if let Some(tbox) = win.get_focal_text_box() {
            //        match &tbox.tview.vtyp {
            //            TextViewType::Editable(editor) => match editor.mode {
            //                EditorMode::Navigation => {
            //                    win.state.text_boxes.next_focus();
            //                }
            //                _ => {}
            //            }
            //            _ => {
            //                win.state.text_boxes.next_focus();
            //            }
            //        }
            //    }
            //}
            // Tab:
            // - If navigating: rotate highlighting of line focus ranges in text.
            // - If editing: insert tab characters (defined via configuration) into current text.
            KeyEvent { code: KeyCode::Tab, modifiers: KeyModifiers::NONE, .. }
                if self.mode == WindowMode::Interaction =>
            {
                return self.act(&Action::MoveToNextTab, data);
            }
            // Backspace: Delete character to the left of the cursors, dragging the remainder of
            // the line from the right.
            KeyEvent { code: KeyCode::Backspace, modifiers: KeyModifiers::NONE, .. }
                if self.mode == WindowMode::Interaction =>
            {
                let result = self.get_focal_window_text_box_mut();
                if let Some(tbox) = res!(result) {
                    if let Some(editor) = tbox.tview.vtyp.get_editor_mut() {
                        match editor.mode {
                            EditorMode::Insert | EditorMode::Replace | EditorMode::Modify => {
                                let result = tbox.tview.atext.get_text_lines_mut();
                                let mut text_lines = res!(result);
                                text_lines.backspace(&mut editor.nav.cursor);
                            }
                            _ => {}
                        }
                    }
                }
            }
            // Delete: Delete character under the cursor, dragging the remainder of the line from
            // the right.
            KeyEvent { code: KeyCode::Delete, modifiers: KeyModifiers::NONE, .. }
                if self.mode == WindowMode::Interaction =>
            {
                let result = self.get_focal_window_text_box_mut();
                if let Some(tbox) = res!(result) {
                    if let Some(editor) = tbox.tview.vtyp.get_editor_mut() {
                        match editor.mode {
                            EditorMode::Insert | EditorMode::Replace | EditorMode::Modify => {
                                let result = tbox.tview.atext.get_text_lines_mut();
                                let mut text_lines = res!(result);
                                text_lines.delete_char(&mut editor.nav.cursor);
                            }
                            _ => {}
                        }
                    }
                }
            }
            // Enter: Insert a new line.
            KeyEvent { code: KeyCode::Enter, modifiers: KeyModifiers::NONE, .. }
                if self.mode == WindowMode::Interaction =>
            {
                let result = self.get_focal_window_text_box_mut();
                if let Some(tbox) = res!(result) {
                    if let Some(editor) = tbox.tview.vtyp.get_editor_mut() {
                        match editor.mode {
                            EditorMode::Insert | EditorMode::Replace | EditorMode::Modify => {
                                let result = tbox.tview.atext.get_text_lines_mut();
                                let mut text_lines = res!(result);
                                text_lines.enter_new_line(&mut editor.nav.cursor);
                            }
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(AppFlow::FullScreenRender)
    }
}
