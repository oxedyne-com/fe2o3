use oxedyne_fe2o3_core::prelude::*;

#[derive(Clone, Copy)]
pub struct BoxStyle {
    horizontal:     char,
    vertical:       char,
    top_left:       char,
    top_right:      char,
    bottom_left:    char,
    bottom_right:   char,
    cross:          char,
    top_join:       char,
    bottom_join:    char,
    left_join:      char,
    right_join:     char,
}

impl BoxStyle {
    pub const SINGLE: Self = Self {
        horizontal:     '─',
        vertical:       '│',
        top_left:       '┌',
        top_right:      '┐',
        bottom_left:    '└',
        bottom_right:   '┘',
        cross:          '┼',
        top_join:       '┬',
        bottom_join:    '┴',
        left_join:      '├',
        right_join:     '┤',
    };
    
    pub const DOUBLE: Self = Self {
        horizontal:     '═',
        vertical:       '║',
        top_left:       '╔',
        top_right:      '╗',
        bottom_left:    '╚',
        bottom_right:   '╝',
        cross:          '╬',
        top_join:       '╦',
        bottom_join:    '╩',
        left_join:      '╠',
        right_join:     '╣',
    };
    
    pub const ROUNDED: Self = Self {
        horizontal:     '─',
        vertical:       '│',
        top_left:       '╭',
        top_right:      '╮',
        bottom_left:    '╰',
        bottom_right:   '╯',
        cross:          '┼',
        top_join:       '┬',
        bottom_join:    '┴',
        left_join:      '├',
        right_join:     '┤',
    };
}

#[derive(Clone, Copy, PartialEq)]
pub enum Alignment {
    Left,
    Right,
    Centre,
    Center,
}

#[derive(Clone)]
enum AlignmentRule {
    Column(usize, Alignment),
    Row(usize, Alignment),
    All(Alignment),
}

pub struct Tabular {
    rows:               Vec<Vec<String>>,
    separator:          &'static str,
    box_style:          Option<BoxStyle>,
    alignment_rules:    Vec<AlignmentRule>,
}

impl Tabular {
    pub fn new() -> Self {
        Self {
            rows:               vec![],
            separator:          "  ",
            box_style:          None,
            alignment_rules:    vec![],
        }
    }

    pub fn separator(mut self, sep: &'static str) -> Self {
        self.separator = sep;
        self
    }

    pub fn boxed(mut self, style: BoxStyle) -> Self {
        self.box_style = Some(style);
        self
    }

    pub fn align_col(mut self, col: usize, alignment: Alignment) -> Self {
        self.alignment_rules.push(AlignmentRule::Column(col, alignment));
        self
    }
    
    pub fn align_cols(mut self, alignments: &[(usize, Alignment)]) -> Self {
        for &(col, align) in alignments {
            self.alignment_rules.push(AlignmentRule::Column(col, align));
        }
        self
    }
    
    pub fn align_row(mut self, row: usize, alignment: Alignment) -> Self {
        self.alignment_rules.push(AlignmentRule::Row(row, alignment));
        self
    }
    
    pub fn align_rows(mut self, alignments: &[(usize, Alignment)]) -> Self {
        for &(row, align) in alignments {
            self.alignment_rules.push(AlignmentRule::Row(row, align));
        }
        self
    }
    
    pub fn align_all(mut self, alignment: Alignment) -> Self {
        self.alignment_rules.push(AlignmentRule::All(alignment));
        self
    }

    fn get_alignment(&self, row: usize, col: usize) -> Alignment {
        let mut alignment = Alignment::Left; // default
    
        // Apply rules in order - later rules override earlier ones.
        for rule in &self.alignment_rules {
            match rule {
                AlignmentRule::All(align) => alignment = *align,
                AlignmentRule::Column(c, align) if *c == col => alignment = *align,
                AlignmentRule::Row(r, align) if *r == row => alignment = *align,
                _ => {}
            }
        }
    
        alignment
    }

    pub fn row<const N: usize>(mut self, cells: [&str; N]) -> Self {
        self.rows.push(cells.iter().map(|&s| s.to_string()).collect());
        self
    }

    pub fn row_owned(mut self, cells: Vec<String>) -> Self {
        self.rows.push(cells);
        self
    }

    pub fn rows_from_iter(mut self, iter: impl Iterator<Item = Vec<String>>) -> Self {
        self.rows.extend(iter);
        self
    }

    fn format_cell(
        &self,
        cell:   &str,
        width:  usize,
        row:    usize,
        col:    usize,
    )
        -> String
    {
        let alignment = self.get_alignment(row, col);
        match alignment {
            Alignment::Left => format!("{:width$}", cell, width = width),
            Alignment::Right => format!("{:>width$}", cell, width = width),
            Alignment::Centre | Alignment::Center => format!("{:^width$}", cell, width = width),
        }
    }

    fn print_horizontal_line(
        &self,
        widths: &[usize],
        style:  &BoxStyle,
        left:   char,
        mid:    char,
        right:  char,
    ) {
        print!("{}", left);
        for (i, &width) in widths.iter().enumerate() {
            print!("{}", style.horizontal.to_string().repeat(width + 2));
            if i < widths.len() - 1 {
                print!("{}", mid);
            }
        }
        println!("{}", right);
    }

    pub fn print(&self) {

        if self.rows.is_empty() { return; }

        // Calculate max width for each column.
        let widths: Vec<usize> = (0..self.rows[0].len())
            .map(|col| self.rows.iter()
                .map(|row| row.get(col).map_or(0, |s| s.len()))
                .max()
                .unwrap_or(0))
            .collect();

        match self.box_style {
            Some(style) => {
                // Top border.
                self.print_horizontal_line(
                    &widths,
                    &style,
                    style.top_left,
                    style.top_join,
                    style.top_right,
                );

                // Rows with borders.
                for (row_idx, row) in self.rows.iter().enumerate() {
                    print!("{}", style.vertical);
                    for (col_idx, (cell, &width)) in row.iter().zip(&widths).enumerate() {
                        let formatted = self.format_cell(cell, width, row_idx, col_idx);
                        print!(" {} ", formatted);
                        if col_idx < row.len() - 1 {
                            print!("{}", style.vertical);
                        }
                    }
                    println!("{}", style.vertical);

                    // Separator after header row.
                    if row_idx == 0 && self.rows.len() > 1 {
                        self.print_horizontal_line(
                            &widths,
                            &style,
                            style.left_join,
                            style.cross,
                            style.right_join,
                        );
                    }
                }

                // Bottom border.
                self.print_horizontal_line(
                    &widths,
                    &style,
                    style.bottom_left,
                    style.bottom_join,
                    style.bottom_right,
                );
            }
            None => {
                // Original behavior without boxing.
                for (row_idx, row) in self.rows.iter().enumerate() {
                    let formatted: Vec<String> = row.iter()
                        .zip(&widths)
                        .enumerate()
                        .map(|(col_idx, (cell, &width))| self.format_cell(
                            cell,
                            width,
                            row_idx,
                            col_idx,
                        ))
                        .collect();
                    println!("{}", formatted.join(self.separator));
                }
            }
        }
    }
}

//// Usage examples:
//
//// Right-align numeric columns
//Tabular::new()
//    .boxed(BoxStyle::SINGLE)
//    .align_col(1, Alignment::Right)  // Revenue column
//    .align_col(2, Alignment::Right)  // Expenses column
//    .align_col(3, Alignment::Right)  // Profit column
//    .row(["Month", "Revenue", "Expenses", "Profit"])
//    .row(["January", "$100,000", "$80,000", "$20,000"])
//    .row(["February", "$1,234,567", "$987,654", "$246,913"])
//    .print();
//
//// Or set multiple alignments at once
//Tabular::new()
//    .boxed(BoxStyle::SINGLE)
//    .align_cols(&[
//        (1, Alignment::Right),
//        (2, Alignment::Right),
//        (3, Alignment::Right),
//    ])
//    .row(["Month", "Revenue", "Expenses", "Profit"])
//    .row(["January", "$100,000", "$80,000", "$20,000"])
//    .print();
//
//// For your monthly table, right-align all numeric columns:
//Tabular::new()
//    .align_cols(&[
//        (3, Alignment::Right),  // Perth
//        (4, Alignment::Right),  // Rest AU
//        (5, Alignment::Right),  // Rest World
//        (6, Alignment::Right),  // Total Users
//        (7, Alignment::Right),  // Overlay Rev
//        (8, Alignment::Right),  // Overlay Bal
//        (9, Alignment::Right),  // RPS A
//        (10, Alignment::Right), // RPS B
//        (11, Alignment::Right), // RPS C
//        (12, Alignment::Right), // Expenses
//    ])
//    .rows_from_iter(rows.into_iter())
//    .print();
