use std::collections::HashMap;

use egui::{
    Color32, FontFamily, FontId, Key, Pos2, Rect, RichText, Rounding, Stroke,
    Vec2,
};

use crate::shell_session::{ShellKind, ShellSession};
use crate::terminal_buffer::is_wide_char;
use crate::window_manager::{LayoutMode, WindowManager};

const BG: Color32 = Color32::from_rgb(13, 13, 17);
const SURFACE: Color32 = Color32::from_rgb(22, 22, 30);
const SURFACE2: Color32 = Color32::from_rgb(30, 30, 42);
const BORDER: Color32 = Color32::from_rgb(50, 50, 70);
const ACCENT: Color32 = Color32::from_rgb(80, 200, 160);
const ACCENT_DIM: Color32 = Color32::from_rgb(40, 110, 85);
const TEXT: Color32 = Color32::from_rgb(210, 210, 220);
const TEXT_DIM: Color32 = Color32::from_rgb(110, 110, 130);
const RED: Color32 = Color32::from_rgb(210, 70, 70);
const YELLOW: Color32 = Color32::from_rgb(230, 190, 60);
const TITLEBAR_H: f32 = 28.0;
const SIDEBAR_W: f32 = 160.0;
const TOOLBAR_H: f32 = 40.0;

pub struct MultiCliApp {
    pub wm: WindowManager,
    pub sessions: HashMap<String, ShellSession>,
    new_shell_kind: ShellKind,
    session_counter: usize,
    drag_state: Option<DragState>,
    resize_state: Option<ResizeState>,
}

struct DragState {
    window_id: String,
    offset: Vec2,
}

struct ResizeState {
    window_id: String,
    start_size: Vec2,
    start_pos: Pos2,
}

impl MultiCliApp {
    pub fn new(cc: &eframe::CreationContext) -> Self {
        setup_cjk_font(&cc.egui_ctx);
        let mut app = Self {
            wm: WindowManager::new(),
            sessions: HashMap::new(),
            new_shell_kind: ShellKind::PowerShell,
            session_counter: 0,
            drag_state: None,
            resize_state: None,
        };
        // spawn an initial shell
        app.spawn_shell(ShellKind::PowerShell);
        app
    }

    fn spawn_shell(&mut self, kind: ShellKind) {
        self.session_counter += 1;
        let id = uuid::Uuid::new_v4().to_string();
        let name = format!("{} {}", kind.label(), self.session_counter);
        let session = ShellSession::new(id.clone(), name.clone(), kind, 120, 40);
        self.sessions.insert(id.clone(), session);
        self.wm.add_window(id, name);
    }

}

impl eframe::App for MultiCliApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(std::time::Duration::from_millis(33));

        // Style
        let mut style = (*ctx.style()).clone();
        style.visuals.window_fill = BG;
        style.visuals.panel_fill = BG;
        style.visuals.override_text_color = Some(TEXT);
        ctx.set_style(style);

        // Global keyboard: Ctrl+N
        if ctx.input(|i| i.key_pressed(Key::N) && i.modifiers.ctrl) {
            self.spawn_shell(self.new_shell_kind.clone());
        }

        // --- Toolbar ---
        egui::TopBottomPanel::top("toolbar")
            .exact_height(TOOLBAR_H)
            .frame(egui::Frame::none().fill(SURFACE).inner_margin(egui::Margin::symmetric(8.0, 0.0)))
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(
                        RichText::new("⬡ MULTI-CLI")
                            .font(FontId::new(15.0, FontFamily::Monospace))
                            .color(ACCENT)
                            .strong(),
                    );
                    ui.add_space(16.0);

                    // shell kind selector
                    egui::ComboBox::from_id_source("shell_kind")
                        .selected_text(self.new_shell_kind.label())
                        .width(110.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.new_shell_kind, ShellKind::PowerShell, "PowerShell");
                            ui.selectable_value(&mut self.new_shell_kind, ShellKind::Cmd, "CMD");
                            ui.selectable_value(&mut self.new_shell_kind, ShellKind::Bash, "Bash");
                        });

                    if toolbar_btn(ui, "+ NEW", ACCENT).clicked() {
                        self.spawn_shell(self.new_shell_kind.clone());
                    }

                    ui.add_space(8.0);

                    if toolbar_btn(ui, "TILE", TEXT_DIM).clicked() {
                        self.wm.tile();
                    }
                    if toolbar_btn(ui, "CASCADE", TEXT_DIM).clicked() {
                        self.wm.cascade();
                    }
                    if toolbar_btn(ui, "FREE", TEXT_DIM).clicked() {
                        self.wm.free();
                    }

                    ui.add_space(8.0);

                    if toolbar_btn(ui, "MIN ALL", YELLOW).clicked() {
                        self.wm.minimize_all();
                    }
                    if toolbar_btn(ui, "RESTORE", TEXT_DIM).clicked() {
                        self.wm.restore_all();
                    }
                });
            });

        // --- Sidebar ---
        egui::SidePanel::left("sidebar")
            .exact_width(SIDEBAR_W)
            .resizable(false)
            .frame(egui::Frame::none().fill(SURFACE).inner_margin(egui::Margin::same(0.0)))
            .show(ctx, |ui| {
                ui.add_space(8.0);
                ui.label(
                    RichText::new("SESSIONS")
                        .font(FontId::new(10.0, FontFamily::Monospace))
                        .color(TEXT_DIM),
                );
                ui.add_space(4.0);

                let window_entries: Vec<(String, String, bool)> = self
                    .wm
                    .windows
                    .iter()
                    .map(|w| (w.id.clone(), w.title.clone(), w.focused))
                    .collect();

                for (win_id, title, focused) in window_entries {
                    let is_focused = focused;
                    let resp = ui.add(
                        egui::Button::new(
                            RichText::new(format!("{} {}", if is_focused { "▶" } else { " " }, title))
                                .font(FontId::new(11.0, FontFamily::Monospace))
                                .color(if is_focused { ACCENT } else { TEXT }),
                        )
                        .fill(if is_focused { ACCENT_DIM } else { Color32::TRANSPARENT })
                        .frame(true)
                        .min_size(Vec2::new(SIDEBAR_W - 12.0, 26.0)),
                    );
                    if resp.clicked() {
                        self.wm.focus(&win_id);
                    }
                }

                ui.add_space(8.0);
                ui.separator();

                // layout indicator
                ui.add_space(8.0);
                let mode_label = match self.wm.layout_mode {
                    LayoutMode::Free => "FREE",
                    LayoutMode::Tile => "TILE",
                    LayoutMode::Cascade => "CASCADE",
                };
                ui.label(
                    RichText::new(format!("Layout: {}", mode_label))
                        .font(FontId::new(10.0, FontFamily::Monospace))
                        .color(TEXT_DIM),
                );
            });

        // --- Main workspace ---
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(BG))
            .show(ctx, |ui| {
                let workspace = ui.available_rect_before_wrap();
                // update wm workspace rect
                self.wm.workspace_rect = workspace;

                let painter = ui.painter();

                // faint grid background
                let grid_spacing = 32.0f32;
                let x0 = workspace.min.x;
                let y0 = workspace.min.y;
                let mut x = x0;
                while x < workspace.max.x {
                    painter.line_segment(
                        [Pos2::new(x, workspace.min.y), Pos2::new(x, workspace.max.y)],
                        Stroke::new(0.5, Color32::from_rgba_unmultiplied(50, 50, 70, 30)),
                    );
                    x += grid_spacing;
                }
                let mut y = y0;
                while y < workspace.max.y {
                    painter.line_segment(
                        [Pos2::new(workspace.min.x, y), Pos2::new(workspace.max.x, y)],
                        Stroke::new(0.5, Color32::from_rgba_unmultiplied(50, 50, 70, 30)),
                    );
                    y += grid_spacing;
                }

                // Handle pointer events
                let pointer_pos = ctx.input(|i| i.pointer.interact_pos());
                let pointer_down = ctx.input(|i| i.pointer.primary_down());
                let pointer_released = ctx.input(|i| i.pointer.primary_released());

                if pointer_released {
                    self.drag_state = None;
                    self.resize_state = None;
                }

                // Apply drag
                if let Some(ds) = &self.drag_state {
                    if let Some(pos) = pointer_pos {
                        let wid = ds.window_id.clone();
                        let off = ds.offset;
                        if let Some(w) = self.wm.windows.iter_mut().find(|w| w.id == wid) {
                            let new_pos = pos - off;
                            w.pos = Pos2::new(
                                new_pos.x.max(workspace.min.x),
                                new_pos.y.max(workspace.min.y),
                            );
                        }
                    }
                }

                // Apply resize
                if let Some(rs) = &self.resize_state {
                    if let Some(pos) = pointer_pos {
                        let wid = rs.window_id.clone();
                        let start_pos = rs.start_pos;
                        let start_size = rs.start_size;
                        if let Some(w) = self.wm.windows.iter_mut().find(|w| w.id == wid) {
                            let delta = pos - start_pos;
                            w.size = Vec2::new(
                                (start_size.x + delta.x).max(280.0),
                                (start_size.y + delta.y).max(160.0),
                            );
                        }
                    }
                }

                // Render windows in z-order
                let sorted = self.wm.sorted_windows();
                let mut focus_request: Option<String> = None;
                let mut close_request: Option<String> = None;
                let mut start_drag: Option<(String, Vec2)> = None;
                let mut start_resize: Option<(String, Vec2, Pos2)> = None;
                let _input_submits: Vec<(String, String)> = Vec::new();

                for &wi in &sorted {
                    let win = &self.wm.windows[wi];
                    if win.minimized {
                        continue;
                    }

                    let win_id = win.id.clone();
                    let session_id = win.session_id.clone();
                    let title = win.title.clone();
                    let pos = win.pos;
                    let size = win.size;
                    let focused = win.focused;

                    let total_h = TITLEBAR_H + size.y;
                    let win_rect = Rect::from_min_size(pos, Vec2::new(size.x, total_h));

                    // Shadow
                    if focused {
                        painter.rect_filled(
                            win_rect.expand(4.0).translate(Vec2::new(3.0, 5.0)),
                            Rounding::same(6.0),
                            Color32::from_rgba_unmultiplied(0, 0, 0, 120),
                        );
                    }

                    // Window background
                    painter.rect_filled(win_rect, Rounding::same(4.0), SURFACE);
                    painter.rect_stroke(
                        win_rect,
                        Rounding::same(4.0),
                        Stroke::new(
                            if focused { 1.5 } else { 0.8 },
                            if focused { ACCENT } else { BORDER },
                        ),
                    );

                    // Title bar
                    let titlebar_rect = Rect::from_min_size(pos, Vec2::new(size.x, TITLEBAR_H));
                    painter.rect_filled(
                        titlebar_rect,
                        Rounding {
                            nw: 4.0, ne: 4.0, sw: 0.0, se: 0.0,
                        },
                        if focused { SURFACE2 } else { SURFACE },
                    );

                    // Title text
                    painter.text(
                        Pos2::new(pos.x + 10.0, pos.y + TITLEBAR_H / 2.0),
                        egui::Align2::LEFT_CENTER,
                        &title,
                        FontId::new(12.0, FontFamily::Monospace),
                        if focused { ACCENT } else { TEXT_DIM },
                    );

                    // Titlebar buttons
                    let close_btn_center = Pos2::new(pos.x + size.x - 14.0, pos.y + TITLEBAR_H / 2.0);
                    let min_btn_center = Pos2::new(pos.x + size.x - 34.0, pos.y + TITLEBAR_H / 2.0);

                    painter.circle_filled(close_btn_center, 5.5, RED);
                    painter.circle_filled(min_btn_center, 5.5, YELLOW);

                    // Terminal output area
                    let term_rect = Rect::from_min_size(
                        Pos2::new(pos.x, pos.y + TITLEBAR_H),
                        Vec2::new(size.x, size.y),
                    );

                    // Render terminal content
                    if let Some(session) = self.sessions.get(&session_id) {
                        let buf = session.buffer.lock().unwrap();
                        let line_h = 14.0;
                        let char_w = 7.4; // narrow (ASCII) cell width
                        let visible_rows = (size.y / line_h) as usize;
                        let lines = buf.visible_lines();
                        // Anchor to cursor so content is always visible
                        let start = (buf.cursor_row + 1).saturating_sub(visible_rows);

                        // Clip all terminal drawing to the content rect
                        let term_painter = painter.with_clip_rect(term_rect);
                        for (row_idx, row) in lines[start..].iter().enumerate() {
                            let y = term_rect.min.y + row_idx as f32 * line_h + 2.0;
                            let mut col_x = term_rect.min.x + 4.0;
                            for cell in row.iter() {
                                if col_x >= term_rect.max.x - char_w {
                                    break;
                                }
                                // '\0' is the right-half placeholder of a wide char — skip
                                if cell.ch == '\0' {
                                    continue;
                                }
                                let cell_w = if is_wide_char(cell.ch) { char_w * 2.0 } else { char_w };
                                if cell.ch != ' ' {
                                    let fg = Color32::from_rgb(cell.fg[0], cell.fg[1], cell.fg[2]);
                                    term_painter.text(
                                        Pos2::new(col_x, y),
                                        egui::Align2::LEFT_TOP,
                                        cell.ch.to_string(),
                                        FontId::new(12.0, FontFamily::Monospace),
                                        fg,
                                    );
                                }
                                col_x += cell_w;
                            }
                        }

                        // Cursor
                        let cursor_vis_row = buf.cursor_row.saturating_sub(start);
                        let cx = term_rect.min.x + 4.0 + buf.cursor_col as f32 * char_w;
                        let cy = term_rect.min.y + cursor_vis_row as f32 * line_h + 2.0;
                        if focused {
                            painter.rect_filled(
                                Rect::from_min_size(Pos2::new(cx, cy), Vec2::new(2.0, 12.0)),
                                Rounding::ZERO,
                                ACCENT,
                            );
                        }
                    }

                    // Resize grip
                    let grip_size = 14.0;
                    let grip_rect = Rect::from_min_size(
                        Pos2::new(win_rect.max.x - grip_size, win_rect.max.y - grip_size),
                        Vec2::splat(grip_size),
                    );
                    painter.text(
                        grip_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "⤡",
                        FontId::new(10.0, FontFamily::Monospace),
                        TEXT_DIM,
                    );

                    // Interaction sensing
                    if let Some(pos) = pointer_pos {
                        if pointer_down && self.drag_state.is_none() && self.resize_state.is_none() {
                            // Click anywhere on window → focus
                            if win_rect.contains(pos) {
                                focus_request = Some(win_id.clone());
                            }

                            // Close button
                            if (pos - close_btn_center).length() < 7.0 {
                                close_request = Some(win_id.clone());
                            }

                            // Minimize button
                            if (pos - min_btn_center).length() < 7.0 {
                                if let Some(w) = self.wm.windows.iter_mut().find(|w| w.id == win_id) {
                                    w.minimized = true;
                                }
                            }

                            // Titlebar drag
                            if titlebar_rect.contains(pos)
                                && (pos - close_btn_center).length() > 8.0
                                && (pos - min_btn_center).length() > 8.0
                            {
                                let off = pos - win_rect.min;
                                start_drag = Some((win_id.clone(), off));
                            }

                            // Resize grip
                            if grip_rect.contains(pos) {
                                start_resize = Some((win_id.clone(), size, pos));
                            }
                        }
                    }
                }

                // Apply collected events
                if let Some(id) = focus_request {
                    self.wm.focus(&id);
                }
                if let Some(id) = close_request {
                    let sid = self.wm.windows.iter().find(|w| w.id == id).map(|w| w.session_id.clone());
                    self.wm.close_window(&id);
                    if let Some(sid) = sid {
                        self.sessions.remove(&sid);
                    }
                }
                if let Some((id, off)) = start_drag {
                    self.drag_state = Some(DragState { window_id: id, offset: off });
                }
                if let Some((id, size, pos)) = start_resize {
                    self.resize_state = Some(ResizeState {
                        window_id: id,
                        start_size: size,
                        start_pos: pos,
                    });
                }

                // Keyboard input → direct PTY passthrough (shell-style)
                if let Some(fid) = self.wm.focused_id.clone() {
                    let win_info = self.wm.windows.iter().find(|w| w.id == fid).map(|w| {
                        (w.session_id.clone(), w.pos, w.size)
                    });

                    if let Some((sid, win_pos, win_size)) = win_info {
                        if let Some(session) = self.sessions.get(&sid) {
                            // Tell egui where the cursor is so the IME candidate window follows it
                            let line_h = 14.0;
                            let char_w = 7.4;
                            let visible_rows = (win_size.y / line_h) as usize;
                            let (cur_row, cur_col) = {
                                let buf = session.buffer.lock().unwrap();
                                (buf.cursor_row, buf.cursor_col)
                            };
                            let start_row = (cur_row + 1).saturating_sub(visible_rows);
                            let cx = win_pos.x + 4.0 + cur_col as f32 * char_w;
                            let cy = win_pos.y + TITLEBAR_H + (cur_row - start_row) as f32 * line_h + 2.0;
                            let cursor_rect = Rect::from_min_size(
                                Pos2::new(cx, cy),
                                Vec2::new(char_w, line_h),
                            );
                            ctx.output_mut(|o| {
                                o.ime = Some(egui::output::IMEOutput {
                                    rect: cursor_rect,
                                    cursor_rect,
                                });
                            });

                            ctx.input(|i| {
                                for e in &i.events {
                                    match e {
                                        // Regular ASCII / non-IME text
                                        egui::Event::Text(t) => {
                                            session.write_input(t.as_bytes());
                                        }
                                        // IME composition finalised → send UTF-8 to PTY
                                        egui::Event::CompositionEnd(t) => {
                                            session.write_input(t.as_bytes());
                                        }
                                        egui::Event::Key { key, pressed: true, modifiers, .. } => {
                                            let bytes: Option<&[u8]> = match key {
                                                Key::Enter => Some(b"\r"),
                                                Key::Backspace => Some(b"\x7f"),
                                                Key::Tab => Some(b"\t"),
                                                Key::Escape => Some(b"\x1b"),
                                                Key::ArrowUp => Some(b"\x1b[A"),
                                                Key::ArrowDown => Some(b"\x1b[B"),
                                                Key::ArrowRight => Some(b"\x1b[C"),
                                                Key::ArrowLeft => Some(b"\x1b[D"),
                                                Key::Home => Some(b"\x1b[H"),
                                                Key::End => Some(b"\x1b[F"),
                                                Key::Delete => Some(b"\x1b[3~"),
                                                Key::C if modifiers.ctrl => Some(b"\x03"),
                                                Key::D if modifiers.ctrl => Some(b"\x04"),
                                                Key::L if modifiers.ctrl => Some(b"\x0c"),
                                                Key::A if modifiers.ctrl => Some(b"\x01"),
                                                Key::E if modifiers.ctrl => Some(b"\x05"),
                                                Key::U if modifiers.ctrl => Some(b"\x15"),
                                                Key::K if modifiers.ctrl => Some(b"\x0b"),
                                                Key::W if modifiers.ctrl => Some(b"\x17"),
                                                _ => None,
                                            };
                                            if let Some(b) = bytes {
                                                session.write_input(b);
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            });
                        }
                    }
                }
            });
    }
}

fn setup_cjk_font(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let candidates = [
        "C:/Windows/Fonts/msyh.ttc",
        "C:/Windows/Fonts/simsun.ttc",
        "C:/Windows/Fonts/meiryo.ttc",
    ];
    for path in &candidates {
        if let Ok(data) = std::fs::read(path) {
            fonts.font_data.insert(
                "cjk".to_owned(),
                egui::FontData::from_owned(data),
            );
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .push("cjk".to_owned());
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .push("cjk".to_owned());
            break;
        }
    }
    ctx.set_fonts(fonts);
}

fn toolbar_btn(ui: &mut egui::Ui, label: &str, color: Color32) -> egui::Response {
    ui.add(
        egui::Button::new(
            RichText::new(label)
                .font(FontId::new(11.0, FontFamily::Monospace))
                .color(color),
        )
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::new(0.8, color.linear_multiply(0.5)))
        .min_size(Vec2::new(0.0, 24.0)),
    )
}
