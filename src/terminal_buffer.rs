//! VT100/ANSI terminal emulation buffer backed by the `vt100` crate.
//! Handles escape sequences, colour mapping, wide-character detection,
//! and OSC 2 (title / CWD) tracking.

use vt100::Color;

/// A single terminal cell: a character with foreground/background colours and text attributes.
#[derive(Clone, Debug)]
pub struct TerminalCell {
    /// The Unicode scalar rendered in this cell.
    pub ch: char,
    /// Foreground colour as an sRGB triple `[r, g, b]`.
    pub fg: [u8; 3],
    /// Background colour as an sRGB triple `[r, g, b]`.
    pub bg: [u8; 3],
    /// Whether the cell is rendered in bold.
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

const DEFAULT_FG: [u8; 3] = [204, 204, 204];
const DEFAULT_BG: [u8; 3] = [18, 18, 18];

/// In-process VT100 terminal emulator. Wraps a [`vt100::Parser`] and exposes
/// a flat cell grid, cursor position, and the OSC 2 screen title.
/// Terminal size is fixed at construction; PTY resize is not yet wired.
pub struct TerminalBuffer {
    parser: vt100::Parser,
    /// Terminal width in columns.
    pub cols: usize,
    /// Terminal height in rows.
    pub rows: usize,
    /// Current cursor row (0-based), synced after every [`Self::feed`] call.
    pub cursor_row: usize,
    /// Current cursor column (0-based), synced after every [`Self::feed`] call.
    pub cursor_col: usize,
    /// Always 0; kept for rendering compatibility.
    pub scroll_offset: usize,
    /// `true` while an alternate-screen application (vim, htop…) is active.
    /// The renderer disables scroll-follow when this flag is set.
    pub alternate_screen: bool,
    /// Current working directory, populated from OSC 7 sequences that the shell
    /// prompt emits (`ESC ] 7 ; file://host/path BEL`).
    pub current_dir: String,
}

impl TerminalBuffer {
    /// Create a new buffer of `cols × rows` cells with no scrollback history.
    pub fn new(cols: usize, rows: usize) -> Self {
        Self {
            parser: vt100::Parser::new(rows as u16, cols as u16, 0),
            cols,
            rows,
            cursor_row: 0,
            cursor_col: 0,
            scroll_offset: 0,
            alternate_screen: false,
            current_dir: String::new(),
        }
    }

    /// Process raw PTY output bytes, updating the parser, cursor position, and
    /// the `alternate_screen` flag.  Also scans for OSC 7 current-directory
    /// notifications before handing data to the vt100 parser.
    pub fn feed(&mut self, data: &[u8]) {
        if let Some(dir) = extract_osc7(data) {
            self.current_dir = dir;
        }
        self.parser.process(data);
        let screen = self.parser.screen();
        let (r, c) = screen.cursor_position();
        self.cursor_row = r as usize;
        self.cursor_col = c as usize;
        self.alternate_screen = screen.alternate_screen();
    }

    /// Erase the display and move the cursor to the origin (row 0, col 0).
    pub fn clear_screen(&mut self) {
        self.parser.process(b"\x1b[2J\x1b[H");
        self.cursor_row = 0;
        self.cursor_col = 0;
    }

    /// Returns a snapshot of all screen rows as `TerminalCell` grids.
    /// Wide-char continuation cells have `ch = ' '`; the renderer uses
    /// column-index–based X positions so no `'\0'` placeholder is needed.
    pub fn visible_lines(&self) -> Vec<Vec<TerminalCell>> {
        let screen = self.parser.screen();
        (0..self.rows as u16)
            .map(|row| {
                (0..self.cols as u16)
                    .map(|col| match screen.cell(row, col) {
                        Some(cell) => {
                            let contents = cell.contents();
                            let ch = contents.chars().next().unwrap_or(' ');
                            TerminalCell {
                                ch,
                                fg: vt100_color(cell.fgcolor(), true),
                                bg: vt100_color(cell.bgcolor(), false),
                                bold: cell.bold(),
                            }
                        }
                        None => TerminalCell::default(),
                    })
                    .collect()
            })
            .collect()
    }

    /// Returns the screen title set via OSC 2 (used to pass current working directory).
    pub fn screen_title(&self) -> String {
        self.parser.screen().title().to_string()
    }

    /// Return the screen as a newline-joined string with trailing whitespace
    /// trimmed per row (useful for testing and clipboard copy).
    pub fn get_text(&self) -> String {
        let screen = self.parser.screen();
        (0..self.rows as u16)
            .map(|row| {
                let s: String = (0..self.cols as u16)
                    .map(|col| {
                        screen
                            .cell(row, col)
                            .map(|c| c.contents().chars().next().unwrap_or(' '))
                            .unwrap_or(' ')
                    })
                    .collect();
                s.trim_end().to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn vt100_color(color: Color, is_fg: bool) -> [u8; 3] {
    match color {
        Color::Default => {
            if is_fg {
                DEFAULT_FG
            } else {
                DEFAULT_BG
            }
        }
        Color::Idx(n) => color_256(n),
        Color::Rgb(r, g, b) => [r, g, b],
    }
}

fn color_256(n: u8) -> [u8; 3] {
    match n {
        0 => [0, 0, 0],
        1 => [205, 49, 49],
        2 => [13, 188, 121],
        3 => [229, 229, 16],
        4 => [36, 114, 200],
        5 => [188, 63, 188],
        6 => [17, 168, 205],
        7 => [229, 229, 229],
        8 => [102, 102, 102],
        9 => [241, 76, 76],
        10 => [35, 209, 139],
        11 => [245, 245, 67],
        12 => [59, 142, 234],
        13 => [214, 112, 214],
        14 => [41, 184, 219],
        15 => [255, 255, 255],
        16..=231 => {
            let idx = n - 16;
            let b = idx % 6;
            let g = (idx / 6) % 6;
            let r = idx / 36;
            let f = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            [f(r), f(g), f(b)]
        }
        232..=255 => {
            let v = 8 + (n - 232) * 10;
            [v, v, v]
        }
    }
}

/// Returns `true` for Unicode scalars that occupy two terminal columns
/// (CJK unified ideographs, Hangul syllables, fullwidth forms, etc.).
pub fn is_wide_char(c: char) -> bool {
    let u = c as u32;
    matches!(
        u,
        0x1100..=0x115F
            | 0x2E80..=0x303E
            | 0x3041..=0x33FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xA000..=0xA4CF
            | 0xAC00..=0xD7AF
            | 0xF900..=0xFAFF
            | 0xFE10..=0xFE1F
            | 0xFE30..=0xFE4F
            | 0xFF00..=0xFF60
            | 0xFFE0..=0xFFE6
            | 0x20000..=0x2A6DF
            | 0x2A700..=0x2CEAF
            | 0x2CEB0..=0x2EBEF
            | 0x30000..=0x3134F
    )
}

// ── OSC 7 current-directory helpers ──────────────────────────────────────────

/// Scan raw PTY bytes for the first OSC 7 sequence and return the decoded path.
/// Shells emit: `ESC ] 7 ; file://hostname/path BEL`
///          or: `ESC ] 7 ; file://hostname/path ESC \`
fn extract_osc7(data: &[u8]) -> Option<String> {
    let mut i = 0;
    while i + 3 < data.len() {
        if data[i] == 0x1b && data[i + 1] == b']' && data[i + 2] == b'7' && data[i + 3] == b';' {
            let start = i + 4;
            let mut j = start;
            while j < data.len() {
                if data[j] == 0x07 {
                    return std::str::from_utf8(&data[start..j]).ok().map(parse_osc7_url);
                }
                if data[j] == 0x1b && j + 1 < data.len() && data[j + 1] == b'\\' {
                    return std::str::from_utf8(&data[start..j]).ok().map(parse_osc7_url);
                }
                j += 1;
            }
        }
        i += 1;
    }
    None
}

/// Convert a `file://` URL to a display path.
///
/// * `file://HOSTNAME/C:/Users/…` → `C:/Users/…`  (Windows drive letter)
/// * `file://hostname/path`       → `/path`         (Unix)
fn parse_osc7_url(url: &str) -> String {
    let path = if let Some(rest) = url.strip_prefix("file://") {
        match rest.find('/') {
            Some(pos) => {
                let after_host = &rest[pos..]; // starts with '/'
                // Windows: "/C:/…" → "C:/…"
                if after_host.len() >= 3
                    && after_host.as_bytes()[1].is_ascii_alphabetic()
                    && after_host.as_bytes()[2] == b':'
                {
                    &after_host[1..]
                } else {
                    after_host
                }
            }
            None => rest,
        }
    } else {
        url
    };
    percent_decode(path)
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_nibble(bytes[i + 1]), hex_nibble(bytes[i + 2])) {
                out.push(char::from(h << 4 | l));
                i += 3;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── TerminalCell ─────────────────────────────────────────────────────────

    #[test]
    fn cell_default_is_space_with_standard_colours() {
        let cell = TerminalCell::default();
        assert_eq!(cell.ch, ' ');
        assert_eq!(cell.fg, DEFAULT_FG);
        assert_eq!(cell.bg, DEFAULT_BG);
        assert!(!cell.bold);
    }

    // ── TerminalBuffer construction ──────────────────────────────────────────

    #[test]
    fn buffer_new_initial_state() {
        let buf = TerminalBuffer::new(80, 24);
        assert_eq!(buf.cols, 80);
        assert_eq!(buf.rows, 24);
        assert_eq!(buf.cursor_row, 0);
        assert_eq!(buf.cursor_col, 0);
        assert_eq!(buf.scroll_offset, 0);
        assert!(!buf.alternate_screen);
    }

    // ── feed / cursor ────────────────────────────────────────────────────────

    #[test]
    fn feed_plain_text_advances_cursor_col() {
        let mut buf = TerminalBuffer::new(80, 24);
        buf.feed(b"Hello");
        assert_eq!(buf.cursor_col, 5);
        assert_eq!(buf.cursor_row, 0);
    }

    #[test]
    fn feed_crlf_advances_cursor_row() {
        let mut buf = TerminalBuffer::new(80, 24);
        buf.feed(b"Line1\r\nLine2");
        assert_eq!(buf.cursor_row, 1);
    }

    // ── visible_lines ────────────────────────────────────────────────────────

    #[test]
    fn visible_lines_returns_correct_grid_dimensions() {
        let buf = TerminalBuffer::new(80, 24);
        let lines = buf.visible_lines();
        assert_eq!(lines.len(), 24);
        assert_eq!(lines[0].len(), 80);
    }

    #[test]
    fn visible_lines_empty_buffer_all_spaces() {
        let buf = TerminalBuffer::new(80, 24);
        for row in buf.visible_lines() {
            for cell in row {
                assert_eq!(cell.ch, ' ');
            }
        }
    }

    #[test]
    fn visible_lines_reflects_fed_text() {
        let mut buf = TerminalBuffer::new(80, 24);
        buf.feed(b"Hi");
        let lines = buf.visible_lines();
        assert_eq!(lines[0][0].ch, 'H');
        assert_eq!(lines[0][1].ch, 'i');
    }

    // ── clear_screen ─────────────────────────────────────────────────────────

    #[test]
    fn clear_screen_resets_cursor_to_origin() {
        let mut buf = TerminalBuffer::new(80, 24);
        buf.feed(b"Hello\r\nWorld");
        buf.clear_screen();
        assert_eq!(buf.cursor_row, 0);
        assert_eq!(buf.cursor_col, 0);
    }

    // ── screen_title (OSC 2) ─────────────────────────────────────────────────

    #[test]
    fn screen_title_empty_initially() {
        assert_eq!(TerminalBuffer::new(80, 24).screen_title(), "");
    }

    #[test]
    fn screen_title_set_via_osc2() {
        let mut buf = TerminalBuffer::new(80, 24);
        buf.feed(b"\x1b]2;/home/user\x07");
        assert_eq!(buf.screen_title(), "/home/user");
    }

    #[test]
    fn screen_title_updated_by_second_osc2() {
        let mut buf = TerminalBuffer::new(80, 24);
        buf.feed(b"\x1b]2;/first\x07");
        buf.feed(b"\x1b]2;/second\x07");
        assert_eq!(buf.screen_title(), "/second");
    }

    // ── get_text ─────────────────────────────────────────────────────────────

    #[test]
    fn get_text_contains_fed_content() {
        let mut buf = TerminalBuffer::new(80, 24);
        buf.feed(b"TestContent");
        assert!(buf.get_text().contains("TestContent"));
    }

    #[test]
    fn get_text_trims_trailing_spaces_per_row() {
        let buf = TerminalBuffer::new(80, 24);
        for line in buf.get_text().lines() {
            assert!(!line.ends_with(' '), "trailing space in: {:?}", line);
        }
    }

    // ── alternate_screen ─────────────────────────────────────────────────────

    #[test]
    fn alternate_screen_flag_toggled_by_escape_sequences() {
        let mut buf = TerminalBuffer::new(80, 24);
        assert!(!buf.alternate_screen);
        buf.feed(b"\x1b[?1049h"); // enter alternate screen
        assert!(buf.alternate_screen);
        buf.feed(b"\x1b[?1049l"); // exit alternate screen
        assert!(!buf.alternate_screen);
    }

    // ── is_wide_char ─────────────────────────────────────────────────────────

    #[test]
    fn is_wide_char_false_for_ascii() {
        for c in ('A'..='Z').chain('0'..='9') {
            assert!(!is_wide_char(c), "unexpected wide: {c:?}");
        }
        assert!(!is_wide_char(' '));
        assert!(!is_wide_char('!'));
    }

    #[test]
    fn is_wide_char_true_for_cjk_unified_ideographs() {
        assert!(is_wide_char('中')); // U+4E2D
        assert!(is_wide_char('日')); // U+65E5
        assert!(is_wide_char('文')); // U+6587
    }

    #[test]
    fn is_wide_char_true_for_hangul_syllables() {
        assert!(is_wide_char('한')); // U+D55C
        assert!(is_wide_char('글')); // U+AE00
    }

    #[test]
    fn is_wide_char_true_for_hiragana_and_katakana() {
        assert!(is_wide_char('あ')); // U+3042
        assert!(is_wide_char('ア')); // U+30A2
    }

    #[test]
    fn is_wide_char_true_for_fullwidth_latin() {
        assert!(is_wide_char('Ａ')); // U+FF21
        assert!(is_wide_char('０')); // U+FF10
    }

    // ── color_256 (private, accessible via use super::*) ─────────────────────

    #[test]
    fn color_256_standard_16_colours() {
        assert_eq!(color_256(0),  [0,   0,   0  ]); // black
        assert_eq!(color_256(1),  [205, 49,  49 ]); // red
        assert_eq!(color_256(2),  [13,  188, 121]); // green
        assert_eq!(color_256(7),  [229, 229, 229]); // white
        assert_eq!(color_256(8),  [102, 102, 102]); // bright black
        assert_eq!(color_256(15), [255, 255, 255]); // bright white
    }

    #[test]
    fn color_256_six_cube_corners() {
        assert_eq!(color_256(16),  [0,   0,   0  ]); // cube origin: r=g=b=0
        assert_eq!(color_256(21),  [0,   0,   255]); // max blue: r=0,g=0,b=5
        assert_eq!(color_256(231), [255, 255, 255]); // cube max: r=g=b=5
    }

    #[test]
    fn color_256_grayscale_ramp() {
        assert_eq!(color_256(232), [8,   8,   8  ]); // v = 8 + 0*10
        assert_eq!(color_256(244), [128, 128, 128]); // v = 8 + 12*10
        assert_eq!(color_256(255), [238, 238, 238]); // v = 8 + 23*10
    }

    // ── vt100_color (private) ─────────────────────────────────────────────────

    #[test]
    fn vt100_color_default_returns_theme_colours() {
        assert_eq!(vt100_color(vt100::Color::Default, true),  DEFAULT_FG);
        assert_eq!(vt100_color(vt100::Color::Default, false), DEFAULT_BG);
    }

    #[test]
    fn vt100_color_rgb_passes_through() {
        assert_eq!(vt100_color(vt100::Color::Rgb(255, 128, 0), true), [255, 128, 0]);
    }

    #[test]
    fn vt100_color_idx_delegates_to_color_256() {
        assert_eq!(vt100_color(vt100::Color::Idx(0),   true),  color_256(0));
        assert_eq!(vt100_color(vt100::Color::Idx(7),   true),  color_256(7));
        assert_eq!(vt100_color(vt100::Color::Idx(255), false), color_256(255));
    }

    // ── ANSI colour sequences → cell colours (integration) ───────────────────

    #[test]
    fn ansi_256_fg_colour_applied_to_cell() {
        let mut buf = TerminalBuffer::new(80, 24);
        buf.feed(b"\x1b[38;5;1mX"); // fg = colour index 1 = [205,49,49]
        let cell = &buf.visible_lines()[0][0];
        assert_eq!(cell.ch, 'X');
        assert_eq!(cell.fg, [205, 49, 49]);
    }

    #[test]
    fn ansi_rgb_fg_colour_applied_to_cell() {
        let mut buf = TerminalBuffer::new(80, 24);
        buf.feed(b"\x1b[38;2;255;128;0mY"); // fg = RGB(255,128,0)
        let cell = &buf.visible_lines()[0][0];
        assert_eq!(cell.ch, 'Y');
        assert_eq!(cell.fg, [255, 128, 0]);
    }

    #[test]
    fn ansi_bold_attribute_set_on_cell() {
        let mut buf = TerminalBuffer::new(80, 24);
        buf.feed(b"\x1b[1mB"); // SGR 1 = bold
        let cell = &buf.visible_lines()[0][0];
        assert_eq!(cell.ch, 'B');
        assert!(cell.bold);
    }
}
