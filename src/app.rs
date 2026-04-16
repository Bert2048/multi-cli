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
const STATUS_H: f32 = 18.0;

/// Root application state owned by `eframe`, updated every frame.
///
/// `wm` owns the window list and layout; `sessions` owns the live PTY sessions.
/// The two are linked by `ShellWindow::session_id == ShellSession::id`.
pub struct MultiCliApp {
    pub wm: WindowManager,
    pub sessions: HashMap<String, ShellSession>,
    new_shell_kind: ShellKind,
    session_counter: usize,
    drag_state: Option<DragState>,
    resize_state: Option<ResizeState>,
    renaming_id: Option<String>,
    rename_buf: String,
    confirm_close_id: Option<String>,
    last_save: std::time::Instant,
}

struct DragState {
    window_id: String,
    offset: Vec2,
}

#[derive(Clone, Copy)]
struct ResizeEdges {
    left: bool,
    right: bool,
    top: bool,
    bottom: bool,
}

impl ResizeEdges {
    fn any(self) -> bool { self.left || self.right || self.top || self.bottom }
    fn cursor(self) -> egui::CursorIcon {
        use egui::CursorIcon::*;
        match (self.left || self.right, self.top || self.bottom,
               (self.left && self.top) || (self.right && self.bottom),
               (self.right && self.top) || (self.left && self.bottom)) {
            (_, _, true, _)  => ResizeNwSe,
            (_, _, _, true)  => ResizeNeSw,
            (true, false, _, _) => ResizeHorizontal,
            (false, true, _, _) => ResizeVertical,
            _ => Default,
        }
    }
}

struct ResizeState {
    window_id: String,
    edges: ResizeEdges,
    start_win_pos: Pos2,
    start_win_size: Vec2,
    start_mouse: Pos2,
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
            renaming_id: None,
            rename_buf: String::new(),
            confirm_close_id: None,
            last_save: std::time::Instant::now(),
        };
        app.load_state_or_default();
        app
    }

    fn spawn_shell(&mut self, kind: ShellKind) {
        self.spawn_shell_ex(kind, None, None, None, None);
    }

    fn spawn_shell_ex(
        &mut self,
        kind: ShellKind,
        name: Option<String>,
        initial_dir: Option<String>,
        pos: Option<egui::Pos2>,
        size: Option<egui::Vec2>,
    ) {
        self.session_counter += 1;
        let id = uuid::Uuid::new_v4().to_string();
        let name = name.unwrap_or_else(|| format!("{} {}", kind.label(), self.session_counter));
        let session = ShellSession::new(id.clone(), name.clone(), kind, 120, 40, initial_dir);
        self.sessions.insert(id.clone(), session);
        let win_id = self.wm.add_window(id, name);
        if let (Some(p), Some(s)) = (pos, size) {
            if let Some(w) = self.wm.windows.iter_mut().find(|w| w.id == win_id) {
                w.pos = p;
                w.size = s;
            }
        }
    }

    fn save_state(&self) {
        let windows: Vec<serde_json::Value> = self.wm.windows.iter().map(|w| {
            let last_dir = self.sessions.get(&w.session_id)
                .and_then(|s| s.buffer.lock().ok())
                .map(|b| b.screen_title())
                .unwrap_or_default();
            let kind_str = self.sessions.get(&w.session_id)
                .map(|s| s.kind.label().to_string())
                .unwrap_or_else(|| "PowerShell".to_string());
            serde_json::json!({
                "name": w.title,
                "kind": kind_str,
                "pos_x": w.pos.x,
                "pos_y": w.pos.y,
                "width": w.size.x,
                "height": w.size.y,
                "minimized": w.minimized,
                "last_dir": last_dir,
            })
        }).collect();
        let state = serde_json::json!({ "windows": windows });
        if let Some(path) = state_path() {
            let _ = std::fs::write(path, serde_json::to_string_pretty(&state).unwrap_or_default());
        }
    }

    fn load_state(&mut self) {
        if let Some(path) = state_path() {
            if let Ok(data) = std::fs::read_to_string(&path) {
                if let Ok(state) = serde_json::from_str::<serde_json::Value>(&data) {
                    if let Some(windows) = state["windows"].as_array() {
                        // close existing sessions
                        self.sessions.clear();
                        self.wm.windows.clear();
                        self.wm.focused_id = None;

                        for w in windows {
                            let name = w["name"].as_str().unwrap_or("Shell").to_string();
                            let kind = match w["kind"].as_str().unwrap_or("PowerShell") {
                                "CMD" | "Cmd" => ShellKind::Cmd,
                                "Bash" => ShellKind::Bash,
                                _ => ShellKind::PowerShell,
                            };
                            let pos_x = w["pos_x"].as_f64().unwrap_or(160.0) as f32;
                            let pos_y = w["pos_y"].as_f64().unwrap_or(40.0) as f32;
                            let width = w["width"].as_f64().unwrap_or(560.0) as f32;
                            let height = w["height"].as_f64().unwrap_or(340.0) as f32;
                            let last_dir: Option<String> = w["last_dir"].as_str()
                                .filter(|s| !s.is_empty())
                                .map(|s| s.to_string());
                            self.spawn_shell_ex(
                                kind,
                                Some(name),
                                last_dir,
                                Some(egui::Pos2::new(pos_x, pos_y)),
                                Some(egui::Vec2::new(width, height)),
                            );
                        }
                        return;
                    }
                }
            }
        }
        // fallback
        self.spawn_shell(ShellKind::PowerShell);
    }

    fn load_state_or_default(&mut self) {
        if let Some(path) = state_path() {
            if path.exists() {
                self.load_state();
                return;
            }
        }
        self.spawn_shell(ShellKind::PowerShell);
    }
}

impl eframe::App for MultiCliApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(std::time::Duration::from_millis(33));

        // Auto-save every 60 seconds
        if self.last_save.elapsed() >= std::time::Duration::from_secs(60) {
            self.save_state();
            self.last_save = std::time::Instant::now();
        }

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

                    ui.add_space(8.0);

                    if toolbar_btn(ui, "SAVE", TEXT_DIM).clicked() {
                        self.save_state();
                    }
                    if toolbar_btn(ui, "LOAD", TEXT_DIM).clicked() {
                        self.load_state();
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

                let mut rename_commit: Option<(String, String)> = None;

                for (win_id, title, focused) in &window_entries {
                    let is_focused = *focused;

                    if self.renaming_id.as_deref() == Some(win_id.as_str()) {
                        // Editing mode: show text input
                        ui.horizontal(|ui| {
                            let resp = ui.add(
                                egui::TextEdit::singleline(&mut self.rename_buf)
                                    .font(FontId::new(11.0, FontFamily::Monospace))
                                    .desired_width(SIDEBAR_W - 8.0),
                            );
                            if ui.input(|i| i.key_pressed(Key::Escape)) {
                                self.renaming_id = None;
                            } else if resp.lost_focus() || ui.input(|i| i.key_pressed(Key::Enter)) {
                                rename_commit = Some((win_id.clone(), self.rename_buf.clone()));
                            }
                            resp.request_focus();
                        });
                    } else {
                        ui.horizontal(|ui| {
                            let btn_w = SIDEBAR_W - 30.0; // leave room for ✕
                            let resp = ui.add(
                                egui::Button::new(
                                    RichText::new(format!("{} {}", if is_focused { "▶" } else { " " }, title))
                                        .font(FontId::new(11.0, FontFamily::Monospace))
                                        .color(if is_focused { ACCENT } else { TEXT }),
                                )
                                .fill(if is_focused { ACCENT_DIM } else { Color32::TRANSPARENT })
                                .frame(true)
                                .min_size(Vec2::new(btn_w, 26.0)),
                            );
                            if resp.clicked() {
                                self.wm.focus(win_id);
                            }
                            if resp.double_clicked() {
                                self.renaming_id = Some(win_id.clone());
                                self.rename_buf = title.clone();
                            }

                            let close_resp = ui.add(
                                egui::Button::new(
                                    RichText::new("✕")
                                        .font(FontId::new(10.0, FontFamily::Monospace))
                                        .color(RED),
                                )
                                .fill(Color32::TRANSPARENT)
                                .frame(true)
                                .min_size(Vec2::new(20.0, 26.0)),
                            );
                            if close_resp.clicked() {
                                self.confirm_close_id = Some(win_id.clone());
                            }
                        });
                    }
                }

                // Apply rename commit outside the borrow
                if let Some((wid, new_name)) = rename_commit {
                    if let Some(w) = self.wm.windows.iter_mut().find(|w| w.id == wid) {
                        w.title = new_name;
                    }
                    self.renaming_id = None;
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
                    if let Some(mpos) = pointer_pos {
                        let wid = rs.window_id.clone();
                        let e = rs.edges;
                        let sp = rs.start_win_pos;
                        let ss = rs.start_win_size;
                        let sm = rs.start_mouse;
                        if let Some(w) = self.wm.windows.iter_mut().find(|w| w.id == wid) {
                            let d = mpos - sm;
                            if e.right  { w.size.x = (ss.x + d.x).max(280.0); }
                            if e.bottom { w.size.y = (ss.y + d.y).max(160.0); }
                            if e.left {
                                let new_w = (ss.x - d.x).max(280.0);
                                w.pos.x = sp.x + (ss.x - new_w);
                                w.size.x = new_w;
                            }
                            if e.top {
                                let new_h = (ss.y - d.y).max(160.0);
                                w.pos.y = sp.y + (ss.y - new_h);
                                w.size.y = new_h;
                            }
                        }
                    }
                }

                // Render windows in z-order
                let sorted = self.wm.sorted_windows();
                let mut focus_request: Option<String> = None;
                let mut close_request: Option<String> = None;
                let mut start_drag: Option<(String, Vec2)> = None;
                let mut start_resize: Option<(String, ResizeEdges, Pos2, Vec2, Pos2)> = None;
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

                    // Outer glow (drawn before fill so the fill masks the interior)
                    if focused {
                        let glow = egui::epaint::Shadow {
                            offset: egui::Vec2::ZERO,
                            blur: 9.0,
                            spread: 1.5,
                            color: Color32::from_rgba_unmultiplied(80, 200, 160, 70),
                        };
                        painter.add(egui::Shape::Mesh(glow.tessellate(win_rect, Rounding::same(4.0))));
                    }

                    // Window background (masks shadow interior, leaving only outer halo)
                    painter.rect_filled(win_rect, Rounding::same(4.0), SURFACE);

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

                    // Titlebar buttons — only close (hides window, does not kill session)
                    let close_btn_center = Pos2::new(pos.x + size.x - 14.0, pos.y + TITLEBAR_H / 2.0);

                    painter.circle_filled(close_btn_center, 5.5, RED);

                    // Terminal output area (height reduced by STATUS_H)
                    let term_rect = Rect::from_min_size(
                        Pos2::new(pos.x, pos.y + TITLEBAR_H),
                        Vec2::new(size.x, size.y - STATUS_H),
                    );

                    // Render terminal content
                    if let Some(session) = self.sessions.get(&session_id) {
                        let buf = session.buffer.lock().unwrap();
                        let line_h = 14.0;
                        let char_w = 7.4; // narrow (ASCII) cell width
                        let visible_rows = ((size.y - STATUS_H) / line_h) as usize;
                        let lines = buf.visible_lines();
                        // In alternate screen (TUI), show from scroll_offset (always 0).
                        // In normal mode, follow the cursor.
                        let start = if buf.alternate_screen {
                            buf.scroll_offset
                        } else {
                            (buf.cursor_row + 1).saturating_sub(visible_rows)
                        };

                        // Clip all terminal drawing to the content rect
                        let term_painter = painter.with_clip_rect(term_rect);
                        for (row_idx, row) in lines[start..].iter().enumerate() {
                            let y = term_rect.min.y + row_idx as f32 * line_h + 2.0;
                            for (col_idx, cell) in row.iter().enumerate() {
                                let col_x = term_rect.min.x + 4.0 + col_idx as f32 * char_w;
                                if col_x >= term_rect.max.x - char_w {
                                    break;
                                }
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
                            }
                        }

                        // Cursor
                        let cursor_vis_row = buf.cursor_row.saturating_sub(start);
                        let cx = term_rect.min.x + 4.0 + buf.cursor_col as f32 * char_w;
                        let cy = term_rect.min.y + cursor_vis_row as f32 * line_h + 2.0;
                        if focused {
                            let cursor_ch = lines
                                .get(buf.cursor_row)
                                .and_then(|r| r.get(buf.cursor_col))
                                .map(|c| c.ch)
                                .unwrap_or(' ');
                            let cursor_w = if is_wide_char(cursor_ch) {
                                char_w * 2.0
                            } else {
                                char_w
                            };
                            // Semi-transparent block highlight
                            painter.rect_filled(
                                Rect::from_min_size(
                                    Pos2::new(cx, cy),
                                    Vec2::new(cursor_w, line_h - 1.0),
                                ),
                                Rounding::ZERO,
                                ACCENT.linear_multiply(0.25),
                            );
                            // Solid left bar
                            painter.rect_filled(
                                Rect::from_min_size(Pos2::new(cx, cy), Vec2::new(2.0, line_h - 1.0)),
                                Rounding::ZERO,
                                ACCENT,
                            );
                        }

                        // Status bar
                        let status_rect = Rect::from_min_size(
                            Pos2::new(pos.x, pos.y + TITLEBAR_H + size.y - STATUS_H),
                            Vec2::new(size.x, STATUS_H),
                        );
                        painter.rect_filled(status_rect, Rounding::ZERO, SURFACE2);
                        painter.line_segment(
                            [status_rect.left_top(), status_rect.right_top()],
                            Stroke::new(0.5, BORDER),
                        );
                        let t = buf.screen_title();
                        let cwd = if t.is_empty() { "~".to_string() } else { t };
                        painter.text(
                            Pos2::new(status_rect.min.x + 6.0, status_rect.center().y),
                            egui::Align2::LEFT_CENTER,
                            cwd,
                            FontId::new(10.0, FontFamily::Monospace),
                            TEXT_DIM,
                        );
                    }

                    // Crisp border drawn last — on top of all content
                    painter.rect_stroke(
                        win_rect,
                        Rounding::same(4.0),
                        Stroke::new(
                            if focused { 1.5 } else { 0.8 },
                            if focused { ACCENT } else { BORDER },
                        ),
                    );

                    // Interaction sensing
                    if let Some(mpos) = pointer_pos {
                        // Compute which edges the mouse is near
                        let ez = 6.0f32;
                        let in_x = mpos.x >= win_rect.min.x - ez && mpos.x <= win_rect.max.x + ez;
                        let in_y = mpos.y >= win_rect.min.y - ez && mpos.y <= win_rect.max.y + ez;
                        let edges = ResizeEdges {
                            left:   (mpos.x - win_rect.min.x).abs() < ez && in_y,
                            right:  (mpos.x - win_rect.max.x).abs() < ez && in_y,
                            top:    (mpos.y - win_rect.min.y).abs() < ez && in_x,
                            bottom: (mpos.y - win_rect.max.y).abs() < ez && in_x,
                        };

                        // Cursor icon when hovering an edge
                        if edges.any() && self.drag_state.is_none() {
                            ctx.output_mut(|o| o.cursor_icon = edges.cursor());
                        }

                        if pointer_down && self.drag_state.is_none() && self.resize_state.is_none() {
                            // Click anywhere on window → focus
                            if win_rect.expand(ez).contains(mpos) {
                                focus_request = Some(win_id.clone());
                            }

                            // Close button → hide window (session stays alive)
                            if (mpos - close_btn_center).length() < 7.0 {
                                close_request = Some(win_id.clone());
                            }

                            // Edge resize (takes priority over titlebar drag)
                            if edges.any() {
                                start_resize = Some((win_id.clone(), edges, win_rect.min, size, mpos));
                            } else if titlebar_rect.contains(mpos)
                                && (mpos - close_btn_center).length() > 8.0
                            {
                                // Titlebar drag
                                let off = mpos - win_rect.min;
                                start_drag = Some((win_id.clone(), off));
                            }
                        }
                    }
                }

                // Apply collected events
                if let Some(id) = focus_request {
                    self.wm.focus(&id);
                }
                if let Some(id) = close_request {
                    // Hide window but keep session alive; click in sidebar to restore
                    if let Some(w) = self.wm.windows.iter_mut().find(|w| w.id == id) {
                        w.minimized = true;
                        w.focused = false;
                    }
                    if self.wm.focused_id.as_deref() == Some(&id) {
                        self.wm.focused_id = None;
                    }
                }
                if let Some((id, off)) = start_drag {
                    self.drag_state = Some(DragState { window_id: id, offset: off });
                }
                if let Some((id, edges, win_pos, win_size, mouse_pos)) = start_resize {
                    self.resize_state = Some(ResizeState {
                        window_id: id,
                        edges,
                        start_win_pos: win_pos,
                        start_win_size: win_size,
                        start_mouse: mouse_pos,
                    });
                }

                // Keyboard input → direct PTY passthrough (shell-style)
                // Skip when the rename input is active to avoid dual input
                if self.renaming_id.is_none() {
                if let Some(fid) = self.wm.focused_id.clone() {
                    let win_info = self.wm.windows.iter().find(|w| w.id == fid).map(|w| {
                        (w.session_id.clone(), w.pos, w.size)
                    });

                    if let Some((sid, win_pos, win_size)) = win_info {
                        if let Some(session) = self.sessions.get(&sid) {
                            // Tell egui where the cursor is so the IME candidate window follows it
                            let line_h = 14.0;
                            let char_w = 7.4;
                            let visible_rows = ((win_size.y - STATUS_H) / line_h) as usize;
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

                            // Key events are handled in raw_input_hook before egui
                            // sees them; here we only need text and IME events.
                            ctx.input(|i| {
                                for e in &i.events {
                                    match e {
                                        // Regular printable text — filter control chars to
                                        // avoid double-sending alongside raw_input_hook.
                                        egui::Event::Text(t) => {
                                            let text: String =
                                                t.chars().filter(|c| !c.is_control()).collect();
                                            if !text.is_empty() {
                                                session.write_input(text.as_bytes());
                                            }
                                        }
                                        // IME composition confirmed
                                        egui::Event::CompositionEnd(t) => {
                                            session.write_input(t.as_bytes());
                                        }
                                        // Clipboard paste (Ctrl+V / Shift+Insert)
                                        egui::Event::Paste(t) => {
                                            session.write_input(t.as_bytes());
                                        }
                                        _ => {}
                                    }
                                }
                            });
                        }
                    }
                }
                } // end if renaming_id.is_none()
            });

        // --- Confirm-close dialog ---
        if let Some(ref close_id) = self.confirm_close_id.clone() {
            let win_title = self.wm.windows.iter()
                .find(|w| &w.id == close_id)
                .map(|w| w.title.clone())
                .unwrap_or_default();

            let mut confirmed = false;
            let mut cancelled = false;

            egui::Window::new("Close Session")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .fixed_size(Vec2::new(280.0, 100.0))
                .show(ctx, |ui| {
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(format!("Close \"{}\"?", win_title))
                            .font(FontId::new(12.0, FontFamily::Monospace))
                            .color(TEXT),
                    );
                    ui.label(
                        RichText::new("The session will be terminated.")
                            .font(FontId::new(10.0, FontFamily::Monospace))
                            .color(TEXT_DIM),
                    );
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.add(
                            egui::Button::new(
                                RichText::new("Close")
                                    .font(FontId::new(11.0, FontFamily::Monospace))
                                    .color(Color32::WHITE),
                            )
                            .fill(RED)
                            .min_size(Vec2::new(80.0, 26.0)),
                        ).clicked() {
                            confirmed = true;
                        }
                        ui.add_space(8.0);
                        if ui.add(
                            egui::Button::new(
                                RichText::new("Cancel")
                                    .font(FontId::new(11.0, FontFamily::Monospace))
                                    .color(TEXT),
                            )
                            .fill(Color32::TRANSPARENT)
                            .stroke(Stroke::new(0.8, BORDER))
                            .min_size(Vec2::new(80.0, 26.0)),
                        ).clicked() {
                            cancelled = true;
                        }
                    });
                });

            if confirmed {
                // Find the session_id linked to this window
                let session_id = self.wm.windows.iter()
                    .find(|w| &w.id == close_id)
                    .map(|w| w.session_id.clone());
                // Remove window from wm
                self.wm.windows.retain(|w| &w.id != close_id);
                if self.wm.focused_id.as_deref() == Some(close_id.as_str()) {
                    self.wm.focused_id = None;
                }
                // Drop the session (kills the PTY thread)
                if let Some(sid) = session_id {
                    self.sessions.remove(&sid);
                }
                self.confirm_close_id = None;
            } else if cancelled {
                self.confirm_close_id = None;
            }
        }
    }

    /// Called by eframe BEFORE egui processes events — intercept key events here
    /// so egui never sees Ctrl+C as "copy", Ctrl+Z as "undo", etc.
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        // Don't intercept keys while the session rename input is active
        if self.renaming_id.is_some() { return; }
        // Only intercept when a terminal window is focused
        let sid = self.wm.focused_id.as_ref()
            .and_then(|fid| self.wm.windows.iter().find(|w| &w.id == fid))
            .map(|w| w.session_id.clone());
        let Some(sid) = sid else { return };

        let mut to_send: Vec<Vec<u8>> = Vec::new();
        let mut to_remove: Vec<usize> = Vec::new();

        for (i, event) in raw_input.events.iter().enumerate() {
            if let egui::Event::Key { key, pressed: true, modifiers, .. } = event {
                if let Some(bytes) = key_to_bytes(key, modifiers) {
                    to_send.push(bytes.to_vec());
                    // Remove so egui cannot also process the event
                    to_remove.push(i);
                }
                // Keys without a mapping (e.g. Ctrl+N for "new window") are left
                // in raw_input so update() can handle them normally.
            }
        }

        for i in to_remove.into_iter().rev() {
            raw_input.events.remove(i);
        }

        if let Some(session) = self.sessions.get(&sid) {
            for bytes in to_send {
                session.write_input(&bytes);
            }
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_state();
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

/// Map an egui key + modifiers to the terminal byte sequence.
/// Returns None for keys that should not be forwarded to the PTY
/// (e.g. Ctrl+N which is the global "new window" shortcut).
fn key_to_bytes(key: &Key, modifiers: &egui::Modifiers) -> Option<&'static [u8]> {
    match key {
        // ── Navigation ──────────────────────────────────────────────────────
        Key::Enter      => Some(b"\r"),
        Key::Backspace  => Some(b"\x7f"),
        Key::Tab        => Some(b"\t"),
        Key::Escape     => Some(b"\x1b"),
        Key::ArrowUp    => Some(b"\x1b[A"),
        Key::ArrowDown  => Some(b"\x1b[B"),
        Key::ArrowRight => Some(b"\x1b[C"),
        Key::ArrowLeft  => Some(b"\x1b[D"),
        Key::Home       => Some(b"\x1b[H"),
        Key::End        => Some(b"\x1b[F"),
        Key::Insert     => Some(b"\x1b[2~"),
        Key::Delete     => Some(b"\x1b[3~"),
        Key::PageUp     => Some(b"\x1b[5~"),
        Key::PageDown   => Some(b"\x1b[6~"),
        // ── Function keys ───────────────────────────────────────────────────
        Key::F1  => Some(b"\x1bOP"),
        Key::F2  => Some(b"\x1bOQ"),
        Key::F3  => Some(b"\x1bOR"),
        Key::F4  => Some(b"\x1bOS"),
        Key::F5  => Some(b"\x1b[15~"),
        Key::F6  => Some(b"\x1b[17~"),
        Key::F7  => Some(b"\x1b[18~"),
        Key::F8  => Some(b"\x1b[19~"),
        Key::F9  => Some(b"\x1b[20~"),
        Key::F10 => Some(b"\x1b[21~"),
        Key::F11 => Some(b"\x1b[23~"),
        Key::F12 => Some(b"\x1b[24~"),
        // ── Ctrl + letter ───────────────────────────────────────────────────
        Key::A if modifiers.ctrl => Some(b"\x01"), // line start
        Key::B if modifiers.ctrl => Some(b"\x02"), // backward char
        Key::C if modifiers.ctrl => Some(b"\x03"), // interrupt
        Key::D if modifiers.ctrl => Some(b"\x04"), // EOF / logout
        Key::E if modifiers.ctrl => Some(b"\x05"), // line end
        Key::F if modifiers.ctrl => Some(b"\x06"), // forward char
        Key::K if modifiers.ctrl => Some(b"\x0b"), // kill to end
        Key::L if modifiers.ctrl => Some(b"\x0c"), // clear screen
        Key::P if modifiers.ctrl => Some(b"\x10"), // prev history
        Key::Q if modifiers.ctrl => Some(b"\x11"), // XON / resume
        Key::R if modifiers.ctrl => Some(b"\x12"), // reverse search
        Key::S if modifiers.ctrl => Some(b"\x13"), // XOFF / pause
        Key::T if modifiers.ctrl => Some(b"\x14"), // transpose chars
        Key::U if modifiers.ctrl => Some(b"\x15"), // kill to start
        Key::W if modifiers.ctrl => Some(b"\x17"), // delete word
        Key::Y if modifiers.ctrl => Some(b"\x19"), // yank
        Key::Z if modifiers.ctrl => Some(b"\x1a"), // suspend (SIGTSTP)
        _ => None,
    }
}

/// Returns the path to `%APPDATA%/multi-cli/state.json`, creating the directory
/// if it does not exist.  Returns `None` if `APPDATA` is unset or the directory
/// could not be created.
fn state_path() -> Option<std::path::PathBuf> {
    let dir = std::path::PathBuf::from(std::env::var("APPDATA").ok()?).join("multi-cli");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("state.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_mod() -> egui::Modifiers { egui::Modifiers::default() }
    fn ctrl()   -> egui::Modifiers { egui::Modifiers { ctrl: true, ..Default::default() } }

    // ── key_to_bytes — navigation ────────────────────────────────────────────

    #[test]
    fn navigation_keys_produce_correct_escape_sequences() {
        assert_eq!(key_to_bytes(&Key::Enter,      &no_mod()), Some(b"\r"      as &[u8]));
        assert_eq!(key_to_bytes(&Key::Backspace,  &no_mod()), Some(b"\x7f"    as &[u8]));
        assert_eq!(key_to_bytes(&Key::Tab,        &no_mod()), Some(b"\t"      as &[u8]));
        assert_eq!(key_to_bytes(&Key::Escape,     &no_mod()), Some(b"\x1b"    as &[u8]));
        assert_eq!(key_to_bytes(&Key::ArrowUp,    &no_mod()), Some(b"\x1b[A"  as &[u8]));
        assert_eq!(key_to_bytes(&Key::ArrowDown,  &no_mod()), Some(b"\x1b[B"  as &[u8]));
        assert_eq!(key_to_bytes(&Key::ArrowRight, &no_mod()), Some(b"\x1b[C"  as &[u8]));
        assert_eq!(key_to_bytes(&Key::ArrowLeft,  &no_mod()), Some(b"\x1b[D"  as &[u8]));
        assert_eq!(key_to_bytes(&Key::Home,       &no_mod()), Some(b"\x1b[H"  as &[u8]));
        assert_eq!(key_to_bytes(&Key::End,        &no_mod()), Some(b"\x1b[F"  as &[u8]));
        assert_eq!(key_to_bytes(&Key::Insert,     &no_mod()), Some(b"\x1b[2~" as &[u8]));
        assert_eq!(key_to_bytes(&Key::Delete,     &no_mod()), Some(b"\x1b[3~" as &[u8]));
        assert_eq!(key_to_bytes(&Key::PageUp,     &no_mod()), Some(b"\x1b[5~" as &[u8]));
        assert_eq!(key_to_bytes(&Key::PageDown,   &no_mod()), Some(b"\x1b[6~" as &[u8]));
    }

    // ── key_to_bytes — function keys ─────────────────────────────────────────

    #[test]
    fn function_keys_produce_correct_escape_sequences() {
        assert_eq!(key_to_bytes(&Key::F1,  &no_mod()), Some(b"\x1bOP"   as &[u8]));
        assert_eq!(key_to_bytes(&Key::F2,  &no_mod()), Some(b"\x1bOQ"   as &[u8]));
        assert_eq!(key_to_bytes(&Key::F3,  &no_mod()), Some(b"\x1bOR"   as &[u8]));
        assert_eq!(key_to_bytes(&Key::F4,  &no_mod()), Some(b"\x1bOS"   as &[u8]));
        assert_eq!(key_to_bytes(&Key::F5,  &no_mod()), Some(b"\x1b[15~" as &[u8]));
        assert_eq!(key_to_bytes(&Key::F12, &no_mod()), Some(b"\x1b[24~" as &[u8]));
    }

    // ── key_to_bytes — Ctrl+letter ───────────────────────────────────────────

    #[test]
    fn ctrl_letter_shortcuts_produce_correct_control_chars() {
        assert_eq!(key_to_bytes(&Key::A, &ctrl()), Some(b"\x01" as &[u8])); // line start
        assert_eq!(key_to_bytes(&Key::B, &ctrl()), Some(b"\x02" as &[u8])); // backward char
        assert_eq!(key_to_bytes(&Key::C, &ctrl()), Some(b"\x03" as &[u8])); // interrupt
        assert_eq!(key_to_bytes(&Key::D, &ctrl()), Some(b"\x04" as &[u8])); // EOF
        assert_eq!(key_to_bytes(&Key::E, &ctrl()), Some(b"\x05" as &[u8])); // line end
        assert_eq!(key_to_bytes(&Key::K, &ctrl()), Some(b"\x0b" as &[u8])); // kill to end
        assert_eq!(key_to_bytes(&Key::L, &ctrl()), Some(b"\x0c" as &[u8])); // clear screen
        assert_eq!(key_to_bytes(&Key::R, &ctrl()), Some(b"\x12" as &[u8])); // reverse search
        assert_eq!(key_to_bytes(&Key::U, &ctrl()), Some(b"\x15" as &[u8])); // kill to start
        assert_eq!(key_to_bytes(&Key::W, &ctrl()), Some(b"\x17" as &[u8])); // delete word
        assert_eq!(key_to_bytes(&Key::Z, &ctrl()), Some(b"\x1a" as &[u8])); // suspend
    }

    // ── key_to_bytes — unmapped keys return None ─────────────────────────────

    #[test]
    fn ctrl_n_not_forwarded_to_pty() {
        // Ctrl+N is the global "new window" shortcut and must NOT reach the shell
        assert_eq!(key_to_bytes(&Key::N, &ctrl()), None);
    }

    #[test]
    fn plain_letter_without_modifier_returns_none() {
        assert_eq!(key_to_bytes(&Key::A, &no_mod()), None);
        assert_eq!(key_to_bytes(&Key::Z, &no_mod()), None);
    }
}
