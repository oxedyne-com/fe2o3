use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_stds::chars::Term;

#[derive(Clone, Debug, Default)]
pub enum Dimension {
    Fixed(usize),
    #[default]
    ShrinkToFit,
}

#[derive(Clone, Debug, Default)]
pub struct BoxSides {
    pub left:   usize,
    pub top:    usize,
    pub right:  usize,
    pub bottom: usize,
}

#[derive(Clone, Debug)]
pub struct FrameLineConfig {
    pub effect_on:          String,
    pub effect_off:         String,
    pub top_left_char:      String,
    pub top_char:           String,
    pub top_right_char:     String,
    pub right_char:         String,
    pub bottom_right_char:  String,
    pub bottom_char:        String,
    pub bottom_left_char:   String,
    pub left_char:          String,
}

impl Default for FrameLineConfig {
    fn default() -> Self {
        Self::simple()
    }
}

impl FrameLineConfig {
    pub fn simple() -> Self {
        Self {
            effect_on:          fmt!(""),
            effect_off:         fmt!(""),
            top_left_char:      fmt!("┌"),
            top_char:           fmt!("─"),
            top_right_char:     fmt!("┐"),
            right_char:         fmt!("│"),
            bottom_right_char:  fmt!("┘"),
            bottom_char:        fmt!("─"),
            bottom_left_char:   fmt!("└"),
            left_char:          fmt!("│"),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct BoxConfig {
    pub width:  Dimension,
    pub height: Dimension,
    pub pad:    BoxSides,
    pub frame:  FrameLineConfig,
}

impl BoxConfig {
    pub fn width(&self, w: usize) -> usize {
        match self.width {
            Dimension::Fixed(w0) => w0,
            Dimension::ShrinkToFit => w,
        }
    }
    pub fn height(&self, h: usize) -> usize {
        match self.height{
            Dimension::Fixed(h0) => h0,
            Dimension::ShrinkToFit => h,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Box {
    pub cfg: BoxConfig,
}

/// ```ignore
/// let framer = visual::Box::default().with_frame_effect(
///     Term::BOLD.to_owned() + Term::SET_BRIGHT_FORE_RED
/// );
/// for line in framer.frame(vec![line], about.len() + 2) {
///     lines.push(line);    
/// }
/// ```
impl Box {

    pub fn with_frame_effect(mut self, effect: String) -> Self {
        self.cfg.frame.effect_on = effect;
        self.cfg.frame.effect_off = Term::RESET.to_string();
        self
    }

    pub fn measure_width(lines: &Vec<String>) -> usize {
        let mut max = 0;
        for line in lines {
            let len = line.chars().count();
            if len > max {
                max = len;
            }
        }
        max
    }

    /// Input should be padded by user so that each line is the same visible length.
    pub fn frame(
        &self,
        lines:          Vec<String>,
        visible_width:  usize, // Visible width of lines, excluding ANSI sequences.
    )
        -> Vec<String>
    {
        let width = visible_width
            - self.cfg.pad.left
            - self.cfg.pad.right;
        let mut result = Vec::new();
        // Top line.
        result.push(fmt!(
            "{}{}{}{}{}",
            self.cfg.frame.effect_on,
            self.cfg.frame.top_left_char,
            self.cfg.frame.top_char.repeat(width),
            self.cfg.frame.top_right_char,
            self.cfg.frame.effect_off,
        ));
        // Middle lines.
        for line in lines {
            result.push(fmt!(
                "{}{}{}{}{}{}{}",
                self.cfg.frame.effect_on,
                self.cfg.frame.left_char,
                self.cfg.frame.effect_off,
                line,
                self.cfg.frame.effect_on,
                self.cfg.frame.right_char,
                self.cfg.frame.effect_off,
            ));
        }
        // Bottom line.
        result.push(fmt!(
            "{}{}{}{}{}",
            self.cfg.frame.effect_on,
            self.cfg.frame.bottom_left_char,
            self.cfg.frame.bottom_char.repeat(width),
            self.cfg.frame.bottom_right_char,
            self.cfg.frame.effect_off,
        ));
        result
    }
}
