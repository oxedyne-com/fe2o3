#![forbid(unsafe_code)]
pub mod app;
pub mod lib_tui;

use crate::{
    app::{
        cfg::AppConfig,
        constant,
        log::AppLoggerConsole,
    },
    lib_tui::{
        action::ActionData,
        draw::window::WindowId,
        event::AppFlow,
        render::{
            CrosstermRenderer,
            DebugCrosstermRenderer,
            Drawable,
            Drawer,
            Renderer,
            Sink,
            When,
        },
        text::typ::{
            HighlightType,
            TextType,
        },
        window::{
            WindowManager,
            WindowManagerConfig,
        },
    },
};

use oxedize_fe2o3_core::{
    prelude::*,
    log::{
        bot::FileConfig,
        console::LoggerConsole,
    },
};
use oxedize_fe2o3_geom::{
    rect::AbsRect,
};
use oxedize_fe2o3_text::{
    Text,
    lines::TextLines,
};

use std::{
    sync::{
        Arc,
        RwLock,
    },
};

use crossterm::event::{
    Event,
    KeyEvent,
};


#[derive(Clone, Debug, Default)]
pub struct App<'a> {
    pub cfg:        AppConfig,
    pub log:        Arc<RwLock<TextLines<TextType, HighlightType>>>,
    pub mouse:      (u16, u16),
    pub win_mgr:    WindowManager<'a>,
}

impl App<'_> {

    pub fn new(cfg: Option<AppConfig>) -> Outcome<Self> {
        let cfg = match cfg {
            Some(cfg) => cfg,
            None => AppConfig::default(),
        };
        Ok(Self {
            cfg:        cfg.clone(),
            log:        Arc::new(RwLock::new(TextLines::new(vec![Text::new("Log line 1.", None)], None))),
            win_mgr:    res!(WindowManager::new(WindowManagerConfig::from(cfg))),
            ..Default::default()
        })
    }

    pub fn handle_key_press(
        &mut self,
        key:    KeyEvent,
        data:   Option<ActionData>,
    )
        -> Outcome<AppFlow>
    {
        self.win_mgr.native_key_handler(key, data)
    }
}

fn setup_log(log_level: LogLevel) -> Outcome<()> {
    let mut log_cfg = get_log_config!();
    log_cfg.level = log_level;
    log_cfg.console = None;
    let file_cfg = FileConfig::new(
        res!(std::env::current_dir()),
        "ironic".to_string(),
        "log".to_string(),
        0,
        Some(1_048_576),
    );
    let log_path = file_cfg.path();
    log_cfg.file = Some(file_cfg);
    set_log_config!(log_cfg);
    info!("Logging at {:?}", log_path);
    Ok(())
}

fn main() -> Outcome<()> {

    const NORMAL_LOG_THRESHOLD: LogLevel = LogLevel::Debug;

    let log_level = res!(LogLevel::from_str("debug"));
    res!(setup_log(log_level));

    let mut app = res!(App::new(None));

    loop {
        match if log_level <= NORMAL_LOG_THRESHOLD {
            let mut app_logger = AppLoggerConsole::new(app.log.clone());
            let simplex_thread = app_logger.go();
            set_log_out!(simplex_thread);
            run(&mut app, CrosstermRenderer::new(std::io::stdout()))
        } else {
            run(&mut app, DebugCrosstermRenderer::new(std::io::stdout()))
        } {
            Ok(()) => {
                break;
            }
            Err(e) => {
                error!(e);
                //if log_level > NORMAL_LOG_THRESHOLD {
                    break;
                //}
            }
        }
    }

    log_finish_wait!();
    Ok(())
}

fn run<S: Sink, R: Renderer<S>>(
    app:        &mut App,
    renderer:   R,
)
    -> Outcome<()>
{
    trace!("Enter run");
    res!(app.make_windows(res!(renderer.size())));
    res!(app.win_mgr.set_focus_by_id(&WindowId::Help));

    let mut drawer = Drawer::new(renderer, app.win_mgr.style_lib.clone(), true);
    res!(drawer.on());

    let mut refresh_render = true;

    //info!("Window list:");
    //for (index, name) in app.win_mgr.list_windows() {
    //    info!("{}{}: {}", if index == app.win_mgr.focus { ">" } else { " " }, index, name);
    //}

    loop {

        let term = AbsRect::from(res!(drawer.rend.size()));
        let data = ActionData {
            term: Some(term),
            ..Default::default()
        };

        // Render, if we need to.
        if refresh_render {
            res!(drawer.rend.clear());
            for window in app.win_mgr.iter_mut() {
                if !window.focus {
                    res!(window.render(&mut drawer, When::Later));
                }
            }
            // Display the focal window last.
            let result = app.win_mgr.get_focal_window_mut();
            let focal_window = res!(result);
            res!(focal_window.render(&mut drawer, When::Later));

            res!(app.win_mgr.draw_cursor(&mut drawer, When::Later));
            res!(drawer.rend.flush());
            refresh_render = false;
        }

        if res!(crossterm::event::poll(std::time::Duration::from_millis(
            constant::MAIN_LOOP_WAIT_MILLIS
        ))) {
            match res!(crossterm::event::read()) {
                Event::Mouse(mouse_event) => {
                    app.mouse = (mouse_event.column, mouse_event.row);
                    //res!(app.append_log(&fmt!("Mouse: ({}, {})", app.mouse.0, app.mouse.1)));
                    //app.windows[0].rect.top_left = Coord::new(app.mouse);
                }
                Event::Key(key_event) => {
                    match app.handle_key_press(key_event, Some(data)) {
                        Ok(AppFlow::Quit) => {
                            res!(drawer.off());
                            return Ok(());
                        }
                        Err(e) => {
                            res!(drawer.off());
                            return Err(e);
                        }
                        Ok(AppFlow::CursorOnly) => {
                            // Just update cursor position without full redraw.
                            let result = app.win_mgr.get_focal_window_text_box_mut();
                            if let Some(tbox) = res!(result) {
                                tbox.tview.update_cursor();
                                res!(drawer.rend.set_cursor(tbox.tview.term_cursor, When::Later));
                                if let Some(cursor_style) = tbox.cfg.cursor_style {
                                    res!(drawer.rend.set_cursor_style(cursor_style, When::Later));
                                }
                                res!(app.win_mgr.draw_cursor(&mut drawer, When::Later));
                                res!(drawer.rend.flush());
                            }
                        }
                        Ok(AppFlow::FocalWindowRender) => {
                            let result = app.win_mgr.get_focal_window_mut();
                            let focal_window = res!(result);

                            let term = AbsRect::from(res!(drawer.rend.size()));
                            if let Some(win_rect) = focal_window.view.relative_to(term) {
                                // win_view is within the terminal view.
                                res!(drawer.rend.clear_rect(win_rect));
                            };

                            res!(focal_window.render(&mut drawer, When::Later));
                            res!(app.win_mgr.draw_cursor(&mut drawer, When::Later));
                            res!(drawer.rend.flush());
                        }
                        Ok(AppFlow::FullScreenRender) => {
                            // This code is pulled out into the top of the loop so as to provide
                            // initialisation.
                            refresh_render = true;
                        }
                        _ => (),
                    }
                }
                _ => (),
            }
        }
    }
}
