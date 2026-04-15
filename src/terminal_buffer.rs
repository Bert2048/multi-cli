use vte::{Params, Parser, Perform};

#[derive(Clone, Debug)]
pub struct TerminalCell {
    pub ch: char,
    pub fg: [u8; 3],
    pub bg: [u8; 3],
    pub bold: bool,
}

impl Default for TerminalCell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: [204, 204, 204],
            bg: [18, 18, 18],
            bold: false,
        }
    }
}

pub struct TerminalBuffer {
    pub lines: Vec<Vec<TerminalCell>>,
    pub cursor_row: usize,
    pub cursor_col: usize,
    pub scroll_offset: usize,
    /// True while inside an alternate screen (e.g. TUI apps, `\x1b[?1049h`).
    /// In this mode scroll_offset is never incremented and H/f use absolute rows.
    pub alternate_screen: bool,
    pub cols: usize,
    pub rows: usize,
    fg: [u8; 3],
    bg: [u8; 3],
    bold: bool,
    parser: Parser,
}

impl TerminalBuffer {
    pub fn new(cols: usize, rows: usize) -> Self {
        let mut lines = Vec::new();
        for _ in 0..rows {
            lines.push(vec![TerminalCell::default(); cols]);
        }
        Self {
            lines,
            cursor_row: 0,
            cursor_col: 0,
            scroll_offset: 0,
            alternate_screen: false,
            cols,
            rows,
            fg: [204, 204, 204],
            bg: [18, 18, 18],
            bold: false,
            parser: Parser::new(),
        }
    }

    pub fn feed(&mut self, data: &[u8]) {
        let mut performer = Performer {
            lines: &mut self.lines,
            cursor_row: &mut self.cursor_row,
            cursor_col: &mut self.cursor_col,
            cols: self.cols,
            rows: self.rows,
            fg: &mut self.fg,
            bg: &mut self.bg,
            bold: &mut self.bold,
            scroll_offset: &mut self.scroll_offset,
            alternate_screen: &mut self.alternate_screen,
        };
        for &byte in data {
            self.parser.advance(&mut performer, byte);
        }
    }

    /// Clear all cells and reset cursor + scroll to origin.
    pub fn clear_screen(&mut self) {
        for row in self.lines.iter_mut() {
            for cell in row.iter_mut() {
                *cell = TerminalCell::default();
            }
        }
        self.scroll_offset = 0;
        self.cursor_row = 0;
        self.cursor_col = 0;
    }

    pub fn visible_lines(&self) -> &[Vec<TerminalCell>] {
        &self.lines
    }

    pub fn get_text(&self) -> String {
        self.lines
            .iter()
            .map(|row| {
                let s: String = row.iter().map(|c| c.ch).collect();
                s.trim_end().to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

struct Performer<'a> {
    lines: &'a mut Vec<Vec<TerminalCell>>,
    cursor_row: &'a mut usize,
    cursor_col: &'a mut usize,
    cols: usize,
    rows: usize,
    fg: &'a mut [u8; 3],
    bg: &'a mut [u8; 3],
    bold: &'a mut bool,
    scroll_offset: &'a mut usize,
    alternate_screen: &'a mut bool,
}

impl<'a> Performer<'a> {
    fn clear_screen(&mut self) {
        for row in self.lines.iter_mut() {
            for cell in row.iter_mut() {
                *cell = TerminalCell::default();
            }
        }
        *self.scroll_offset = 0;
        *self.cursor_row = 0;
        *self.cursor_col = 0;
    }

    fn ensure_row(&mut self, row: usize) {
        while self.lines.len() <= row {
            self.lines.push(vec![TerminalCell::default(); self.cols]);
        }
    }

    fn write_char(&mut self, ch: char) {
        let row = *self.cursor_row;
        let col = *self.cursor_col;
        self.ensure_row(row);
        let wide = is_wide_char(ch);
        if col < self.cols {
            self.lines[row][col] = TerminalCell {
                ch,
                fg: *self.fg,
                bg: *self.bg,
                bold: *self.bold,
            };
            // Mark the second column of a wide char as a NUL placeholder
            if wide && col + 1 < self.cols {
                self.lines[row][col + 1] = TerminalCell {
                    ch: '\0',
                    fg: *self.fg,
                    bg: *self.bg,
                    bold: *self.bold,
                };
            }
        }
        // Wide chars occupy 2 terminal columns
        *self.cursor_col += if wide { 2 } else { 1 };
        if *self.cursor_col >= self.cols {
            *self.cursor_col = 0;
            *self.cursor_row += 1;
            self.ensure_row(*self.cursor_row);
            // In alternate screen, clamp instead of scrolling
            if !*self.alternate_screen && *self.cursor_row >= self.rows + *self.scroll_offset {
                *self.scroll_offset += 1;
            }
        }
    }
}

impl<'a> Perform for Performer<'a> {
    fn print(&mut self, c: char) {
        self.write_char(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            b'\n' => {
                *self.cursor_row += 1;
                self.ensure_row(*self.cursor_row);
                // In alternate screen, clamp instead of scrolling
                if !*self.alternate_screen && *self.cursor_row >= self.rows + *self.scroll_offset {
                    *self.scroll_offset += 1;
                }
            }
            b'\r' => {
                *self.cursor_col = 0;
            }
            b'\x08' => {
                // backspace
                if *self.cursor_col > 0 {
                    *self.cursor_col -= 1;
                }
            }
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], _ignore: bool, action: char) {
        // Handle private-mode sequences: \x1b[?NNNh / \x1b[?NNNl
        if intermediates.contains(&b'?') {
            let mode = params.iter().next().and_then(|p| p.first()).copied().unwrap_or(0);
            match (action, mode) {
                // Enter alternate screen — treat as a full clear
                ('h', 1049) | ('h', 47) | ('h', 1047) => {
                    self.clear_screen();
                }
                // Exit alternate screen — same effect for now
                ('l', 1049) | ('l', 47) | ('l', 1047) => {
                    self.clear_screen();
                }
                _ => {}
            }
            return;
        }

        match action {
            'A' => {
                // cursor up
                let n = params.iter().next().and_then(|p| p.first()).copied().unwrap_or(1) as usize;
                *self.cursor_row = self.cursor_row.saturating_sub(n);
            }
            'B' => {
                // cursor down
                let n = params.iter().next().and_then(|p| p.first()).copied().unwrap_or(1) as usize;
                *self.cursor_row += n;
                self.ensure_row(*self.cursor_row);
            }
            'C' => {
                // cursor forward
                let n = params.iter().next().and_then(|p| p.first()).copied().unwrap_or(1) as usize;
                *self.cursor_col = (*self.cursor_col + n).min(self.cols - 1);
            }
            'D' => {
                // cursor back
                let n = params.iter().next().and_then(|p| p.first()).copied().unwrap_or(1) as usize;
                *self.cursor_col = self.cursor_col.saturating_sub(n);
            }
            // Cursor horizontal absolute (column n, 1-based)
            'G' => {
                let col = params.iter().next().and_then(|p| p.first()).copied().unwrap_or(1) as usize;
                *self.cursor_col = col.saturating_sub(1).min(self.cols - 1);
            }
            // Cursor vertical absolute (row n, 1-based)
            'd' => {
                let row = params.iter().next().and_then(|p| p.first()).copied().unwrap_or(1) as usize;
                *self.cursor_row = (row.saturating_sub(1) + *self.scroll_offset).min(self.rows + *self.scroll_offset - 1);
                self.ensure_row(*self.cursor_row);
            }
            // Delete n characters at cursor (shift rest left)
            'P' => {
                let n = params.iter().next().and_then(|p| p.first()).copied().unwrap_or(1) as usize;
                let row = *self.cursor_row;
                let col = *self.cursor_col;
                self.ensure_row(row);
                let len = self.lines[row].len();
                let end = (col + n).min(len);
                self.lines[row].drain(col..end);
                while self.lines[row].len() < self.cols {
                    self.lines[row].push(TerminalCell::default());
                }
            }
            // Insert n blank lines at cursor row
            'L' => {
                let n = params.iter().next().and_then(|p| p.first()).copied().unwrap_or(1) as usize;
                let row = *self.cursor_row;
                self.ensure_row(row);
                for _ in 0..n {
                    self.lines.insert(row, vec![TerminalCell::default(); self.cols]);
                }
            }
            // Delete n lines at cursor row
            'M' => {
                let n = params.iter().next().and_then(|p| p.first()).copied().unwrap_or(1) as usize;
                let row = *self.cursor_row;
                self.ensure_row(row);
                let end = (row + n).min(self.lines.len());
                self.lines.drain(row..end);
                self.ensure_row(row);
            }
            'H' | 'f' => {
                // Absolute cursor position — row/col are viewport-relative, no scroll_offset
                let mut iter = params.iter();
                let row = iter.next().and_then(|p| p.first()).copied().unwrap_or(1) as usize;
                let col = iter.next().and_then(|p| p.first()).copied().unwrap_or(1) as usize;
                *self.cursor_row = row.saturating_sub(1) + *self.scroll_offset;
                *self.cursor_col = col.saturating_sub(1);
                self.ensure_row(*self.cursor_row);
            }
            'J' => {
                let mode = params.iter().next().and_then(|p| p.first()).copied().unwrap_or(0);
                match mode {
                    // Erase from cursor to end of screen
                    0 => {
                        let row = *self.cursor_row;
                        let col = *self.cursor_col;
                        self.ensure_row(row);
                        for c in self.lines[row][col..].iter_mut() {
                            *c = TerminalCell::default();
                        }
                        for r in (row + 1)..self.lines.len() {
                            for c in self.lines[r].iter_mut() {
                                *c = TerminalCell::default();
                            }
                        }
                    }
                    // Erase entire screen + reset (used by TUI apps)
                    2 | 3 => self.clear_screen(),
                    _ => {}
                }
            }
            'K' => {
                // erase in line
                let mode = params.iter().next().and_then(|p| p.first()).copied().unwrap_or(0);
                let row = *self.cursor_row;
                self.ensure_row(row);
                match mode {
                    0 => {
                        // erase to end
                        let col = *self.cursor_col;
                        for c in self.lines[row][col..].iter_mut() {
                            *c = TerminalCell::default();
                        }
                    }
                    1 => {
                        // erase to start
                        let col = *self.cursor_col;
                        for c in self.lines[row][..=col].iter_mut() {
                            *c = TerminalCell::default();
                        }
                    }
                    2 => {
                        // erase whole line
                        for c in self.lines[row].iter_mut() {
                            *c = TerminalCell::default();
                        }
                    }
                    _ => {}
                }
            }
            'm' => {
                // SGR — flatten all sub-params into a flat list for look-ahead parsing
                let flat: Vec<u16> = params.iter()
                    .flat_map(|p| p.iter().copied())
                    .collect();
                let n = flat.len();
                let mut i = 0;
                while i < n {
                    match flat[i] {
                        0 => {
                            *self.fg = [204, 204, 204];
                            *self.bg = [18, 18, 18];
                            *self.bold = false;
                        }
                        1 => *self.bold = true,
                        22 => *self.bold = false,
                        // Foreground: 8-color
                        30 => *self.fg = [0, 0, 0],
                        31 => *self.fg = [205, 49, 49],
                        32 => *self.fg = [13, 188, 121],
                        33 => *self.fg = [229, 229, 16],
                        34 => *self.fg = [36, 114, 200],
                        35 => *self.fg = [188, 63, 188],
                        36 => *self.fg = [17, 168, 205],
                        37 => *self.fg = [229, 229, 229],
                        39 => *self.fg = [204, 204, 204],
                        // Foreground: bright
                        90 => *self.fg = [102, 102, 102],
                        91 => *self.fg = [241, 76, 76],
                        92 => *self.fg = [35, 209, 139],
                        93 => *self.fg = [245, 245, 67],
                        94 => *self.fg = [59, 142, 234],
                        95 => *self.fg = [214, 112, 214],
                        96 => *self.fg = [41, 184, 219],
                        97 => *self.fg = [229, 229, 229],
                        // Background: 8-color
                        40 => *self.bg = [0, 0, 0],
                        41 => *self.bg = [205, 49, 49],
                        42 => *self.bg = [13, 188, 121],
                        43 => *self.bg = [229, 229, 16],
                        44 => *self.bg = [36, 114, 200],
                        45 => *self.bg = [188, 63, 188],
                        46 => *self.bg = [17, 168, 205],
                        47 => *self.bg = [229, 229, 229],
                        49 => *self.bg = [18, 18, 18],
                        // Background: bright
                        100 => *self.bg = [102, 102, 102],
                        101 => *self.bg = [241, 76, 76],
                        102 => *self.bg = [35, 209, 139],
                        103 => *self.bg = [245, 245, 67],
                        104 => *self.bg = [59, 142, 234],
                        105 => *self.bg = [214, 112, 214],
                        106 => *self.bg = [41, 184, 219],
                        107 => *self.bg = [229, 229, 229],
                        // Foreground: extended (38;5;N or 38;2;R;G;B)
                        38 if i + 2 < n && flat[i + 1] == 5 => {
                            *self.fg = color_256(flat[i + 2] as u8);
                            i += 2;
                        }
                        38 if i + 4 < n && flat[i + 1] == 2 => {
                            *self.fg = [flat[i+2] as u8, flat[i+3] as u8, flat[i+4] as u8];
                            i += 4;
                        }
                        // Background: extended (48;5;N or 48;2;R;G;B)
                        48 if i + 2 < n && flat[i + 1] == 5 => {
                            *self.bg = color_256(flat[i + 2] as u8);
                            i += 2;
                        }
                        48 if i + 4 < n && flat[i + 1] == 2 => {
                            *self.bg = [flat[i+2] as u8, flat[i+3] as u8, flat[i+4] as u8];
                            i += 4;
                        }
                        _ => {}
                    }
                    i += 1;
                }
            }
            _ => {}
        }
    }

    fn hook(&mut self, _params: &Params, _intermediates: &[u8], _ignore: bool, _action: char) {}
    fn put(&mut self, _byte: u8) {}
    fn unhook(&mut self) {}
    fn osc_dispatch(&mut self, _params: &[&[u8]], _bell_terminated: bool) {}
    fn esc_dispatch(&mut self, _intermediates: &[u8], _ignore: bool, _byte: u8) {}
}

/// Map a 256-color index to RGB.
fn color_256(n: u8) -> [u8; 3] {
    match n {
        // 0-7: standard ANSI colors
        0 => [0, 0, 0],
        1 => [205, 49, 49],
        2 => [13, 188, 121],
        3 => [229, 229, 16],
        4 => [36, 114, 200],
        5 => [188, 63, 188],
        6 => [17, 168, 205],
        7 => [229, 229, 229],
        // 8-15: bright colors
        8  => [102, 102, 102],
        9  => [241, 76, 76],
        10 => [35, 209, 139],
        11 => [245, 245, 67],
        12 => [59, 142, 234],
        13 => [214, 112, 214],
        14 => [41, 184, 219],
        15 => [255, 255, 255],
        // 16-231: 6×6×6 color cube
        16..=231 => {
            let idx = n - 16;
            let b = idx % 6;
            let g = (idx / 6) % 6;
            let r = idx / 36;
            let f = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            [f(r), f(g), f(b)]
        }
        // 232-255: grayscale ramp
        232..=255 => {
            let v = 8 + (n - 232) * 10;
            [v, v, v]
        }
    }
}

/// Returns true for characters that occupy two terminal columns (CJK, fullwidth).
pub fn is_wide_char(c: char) -> bool {
    let u = c as u32;
    matches!(u,
        0x1100..=0x115F |
        0x2E80..=0x303E |
        0x3041..=0x33FF |
        0x3400..=0x4DBF |
        0x4E00..=0x9FFF |
        0xA000..=0xA4CF |
        0xAC00..=0xD7AF |
        0xF900..=0xFAFF |
        0xFE10..=0xFE1F |
        0xFE30..=0xFE4F |
        0xFF00..=0xFF60 |
        0xFFE0..=0xFFE6 |
        0x20000..=0x2A6DF |
        0x2A700..=0x2CEAF |
        0x2CEB0..=0x2EBEF |
        0x30000..=0x3134F
    )
}
