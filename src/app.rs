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
const TOOLBAR_H: f32 = 40.0;
const STATUS_H: f32 = 18.0;

/// One Claude user profile — maps a display name to a home directory.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ClaudeUser {
    /// Display name shown in the toolbar user-selector and settings list.
    pub name: String,
    /// Home directory for this user (HOME / USERPROFILE / CLAUDE_CONFIG_DIR base).
    pub home_dir: String,
}

/// Persisted user preferences, saved to `%APPDATA%/multi-cli/settings.json`.
/// One entry in the user-defined custom shell list.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct CustomShellConfig {
    /// Label shown in the toolbar dropdown.
    pub name: String,
    /// Executable path or command (empty = entry hidden from toolbar).
    pub cmd: String,
    /// Initial working directory (empty = OS default).
    pub dir: String,
    /// Command sent silently to the PTY at session start.
    pub startup_cmd: String,
}

fn default_token_5h_limit() -> u64 { 2_350_000 }
fn default_token_week_limit() -> u64 { 120_000_000 }

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct AppSettings {
    /// Terminal font size in pixels (range 8–24).
    pub font_size: f32,
    /// `line_h = font_size × line_height_scale`.
    pub line_height_scale: f32,
    /// Shell opened by Ctrl+N and the NEW button.
    pub default_shell: ShellKind,
    /// PTY column count for new sessions.
    pub pty_cols: u16,
    /// PTY row count for new sessions.
    pub pty_rows: u16,
    /// Sidebar panel width in pixels.
    pub sidebar_width: f32,
    /// Initial working directory for new Claude sessions (empty = OS default).
    pub claude_initial_dir: String,
    /// Pass --dangerously-skip-permissions when launching claude.
    #[serde(default)]
    pub claude_skip_permissions: bool,
    /// Add Telegram MCP plugin when launching claude.
    #[serde(default)]
    pub claude_telegram: bool,
    /// User-defined custom shell entries shown in the toolbar.
    #[serde(default)]
    pub custom_shells: Vec<CustomShellConfig>,
    /// Named Claude user profiles (HOME directory per user).
    #[serde(default)]
    pub claude_users: Vec<ClaudeUser>,
    /// Index into `claude_users` for the currently selected user.
    #[serde(default)]
    pub selected_claude_user: usize,
    /// Token budget for the rolling 5-hour window (0 = show raw count).
    #[serde(default = "default_token_5h_limit")]
    pub token_5h_limit: u64,
    /// Token budget for the rolling 7-day window (0 = show raw count).
    #[serde(default = "default_token_week_limit")]
    pub token_week_limit: u64,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            font_size: 12.0,
            line_height_scale: 14.0 / 12.0, // ≈ 1.167
            default_shell: ShellKind::Claude,
            pty_cols: 120,
            pty_rows: 40,
            sidebar_width: 160.0,
            claude_initial_dir: String::new(),
            claude_skip_permissions: false,
            claude_telegram: false,
            custom_shells: Vec::new(),
            claude_users: vec![ClaudeUser { name: "Default".to_string(), home_dir: String::new() }],
            selected_claude_user: 0,
            token_5h_limit: 2_350_000,
            token_week_limit: 120_000_000,
        }
    }
}

impl AppSettings {
    /// Pixel width of one character column at the current font size.
    pub fn char_w(&self) -> f32 { self.font_size * (7.4 / 12.0) }
    /// Pixel height of one terminal line at the current font size and spacing.
    pub fn line_h(&self) -> f32 { self.font_size * self.line_height_scale }
}

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
    text_selection: Option<TextSelection>,
    sel_dragging: bool,
    context_menu: Option<(Pos2, String)>, // (screen_pos, window_id)
    settings: AppSettings,
    show_settings: bool,
    dir_change_dialog: Option<DirChangeDialog>,
    pending_relaunch: Option<PendingRelaunch>,
    /// Last known working directory per session (Claude/Custom overwrite the OSC 2 title).
    session_dirs: HashMap<String, String>,
    /// Which claude_users index each session was spawned with (session_id → user_idx).
    session_user_idx: HashMap<String, usize>,
    /// Per-home-dir token caches; keyed by resolved home path.
    token_caches: HashMap<String, TokenCache>,
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

struct DirChangeDialog {
    session_id: String,
    new_dir: String,
}

struct PendingRelaunch {
    session_id: String,
    dir: String,
    claude_cmd: String,
    fire_at: std::time::Instant,
}

/// Cached Claude token-usage stats.
/// Token usage aggregated from local JSONL transcripts.
struct TokenCache {
    tokens_5h: u64,
    tokens_week: u64,
    /// Timestamp of oldest entry in 5h window — used to estimate reset time.
    oldest_5h: Option<u64>,
    /// Timestamp of oldest entry in 7d window — used to estimate reset time.
    oldest_week: Option<u64>,
    last_scan: std::time::Instant,
}

impl Default for TokenCache {
    fn default() -> Self {
        Self {
            tokens_5h: 0, tokens_week: 0,
            oldest_5h: None, oldest_week: None,
            last_scan: std::time::Instant::now()
                .checked_sub(std::time::Duration::from_secs(15))
                .unwrap_or_else(std::time::Instant::now),
        }
    }
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

struct TextSelection {
    window_id: String,
    anchor: (usize, usize), // (buf_row, col)
    focus: (usize, usize),
}

impl TextSelection {
    fn range(&self) -> ((usize, usize), (usize, usize)) {
        if self.anchor <= self.focus { (self.anchor, self.focus) } else { (self.focus, self.anchor) }
    }
    fn is_empty(&self) -> bool { self.anchor == self.focus }
}

impl MultiCliApp {
    pub fn new(cc: &eframe::CreationContext) -> Self {
        setup_cjk_font(&cc.egui_ctx);
        let settings = load_settings_from_disk();
        let mut app = Self {
            wm: WindowManager::new(),
            sessions: HashMap::new(),
            new_shell_kind: settings.default_shell.clone(),
            session_counter: 0,
            drag_state: None,
            resize_state: None,
            renaming_id: None,
            rename_buf: String::new(),
            confirm_close_id: None,
            last_save: std::time::Instant::now(),
            text_selection: None,
            sel_dragging: false,
            context_menu: None,
            settings,
            show_settings: false,
            dir_change_dialog: None,
            pending_relaunch: None,
            session_dirs: HashMap::new(),
            session_user_idx: HashMap::new(),
            token_caches: HashMap::new(),
        };
        app.load_state_or_default();
        // Guarantee the Default user is always first so old saves still have it.
        if app.settings.claude_users.is_empty() {
            app.settings.claude_users.insert(0, ClaudeUser {
                name: "Default".to_string(),
                home_dir: String::new(),
            });
        }
        app
    }

    fn save_settings(&self) {
        if let Some(path) = settings_path() {
            if let Ok(json) = serde_json::to_string_pretty(&self.settings) {
                let _ = std::fs::write(path, json);
            }
        }
    }

    fn spawn_shell(&mut self, kind: ShellKind) {
        let (name, initial_dir) = match &kind {
            ShellKind::Claude => {
                let dir = if self.settings.claude_initial_dir.is_empty() { None }
                          else { Some(self.settings.claude_initial_dir.clone()) };
                (None, dir)
            }
            ShellKind::Custom(exe) => {
                let cfg = self.settings.custom_shells.iter()
                    .find(|c| &c.cmd == exe)
                    .cloned();
                let name = cfg.as_ref().and_then(|c| if c.name.is_empty() { None } else { Some(c.name.clone()) });
                let dir  = cfg.as_ref().and_then(|c| if c.dir.is_empty()  { None } else { Some(c.dir.clone())  });
                (name, dir)
            }
            _ => (None, None),
        };
        self.spawn_shell_ex(kind, name, initial_dir, None, None);
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
        let startup_cmd = match &kind {
            ShellKind::Custom(exe) => {
                self.settings.custom_shells.iter()
                    .find(|c| &c.cmd == exe)
                    .and_then(|c| if c.startup_cmd.is_empty() { None } else { Some(c.startup_cmd.clone()) })
            }
            ShellKind::Claude => {
                let mut cmd = String::from("claude");
                if self.settings.claude_skip_permissions {
                    cmd.push_str(" --dangerously-skip-permissions");
                }
                if self.settings.claude_telegram {
                    cmd.push_str(" --channels plugin:telegram@claude-plugins-official");
                }
                Some(cmd)
            }
            _ => None,
        };
        let is_claude = matches!(&kind, ShellKind::Claude);
        let claude_user_idx = if is_claude {
            self.settings.selected_claude_user
                .min(self.settings.claude_users.len().saturating_sub(1))
        } else { 0 };
        let user_home = if is_claude && claude_user_idx > 0 {
            self.settings.claude_users.get(claude_user_idx)
                .filter(|u| !u.home_dir.is_empty())
                .map(|u| u.home_dir.clone())
        } else {
            None
        };
        let tracked_dir = initial_dir.clone();
        let session = ShellSession::new(
            kind,
            self.settings.pty_cols, self.settings.pty_rows, initial_dir,
            startup_cmd, user_home,
        );
        self.sessions.insert(id.clone(), session);
        if is_claude {
            self.session_user_idx.insert(id.clone(), claude_user_idx);
        }
        if let Some(dir) = tracked_dir {
            self.session_dirs.insert(id.clone(), dir);
        }
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
                .map(|s| match &s.kind {
                    ShellKind::Custom(exe) => format!("Custom:{}", exe),
                    k => k.label().to_string(),
                })
                .unwrap_or_else(|| "PowerShell".to_string());
            serde_json::json!({
                "name": w.title,
                "kind": kind_str,
                "user_idx": self.session_user_idx.get(&w.session_id).copied().unwrap_or(0),
                "pos_x": w.pos.x,
                "pos_y": w.pos.y,
                "width": w.size.x,
                "height": w.size.y,
                "minimized": w.minimized,
                "focused": w.focused,
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

                        let mut focused_idx: Option<usize> = None;
                        for (idx, w) in windows.iter().enumerate() {
                            if w["focused"].as_bool().unwrap_or(false) {
                                focused_idx = Some(idx);
                            }
                            let name = w["name"].as_str().unwrap_or("Shell").to_string();
                            let kind_raw = w["kind"].as_str().unwrap_or("PowerShell");
                            let kind = if let Some(exe) = kind_raw.strip_prefix("Custom:") {
                                ShellKind::Custom(exe.to_string())
                            } else {
                                match kind_raw {
                                    "CMD" | "Cmd" => ShellKind::Cmd,
                                    "Bash" => ShellKind::Bash,
                                    "Claude" => ShellKind::Claude,
                                    _ => ShellKind::PowerShell,
                                }
                            };
                            let pos_x = w["pos_x"].as_f64().unwrap_or(160.0) as f32;
                            let pos_y = w["pos_y"].as_f64().unwrap_or(40.0) as f32;
                            let width = w["width"].as_f64().unwrap_or(560.0) as f32;
                            let height = w["height"].as_f64().unwrap_or(340.0) as f32;
                            let last_dir: Option<String> = w["last_dir"].as_str()
                                .filter(|s| !s.is_empty())
                                .map(|s| s.to_string());
                            // Resolve saved user index before computing restore_dir so
                            // custom-user Claude sessions can use their own home directory.
                            let saved_user_idx = w["user_idx"].as_u64().unwrap_or(0) as usize;
                            let saved_user_idx_clamped = saved_user_idx
                                .min(self.settings.claude_users.len().saturating_sub(1));
                            let restore_dir = match &kind {
                                ShellKind::Claude => {
                                    if saved_user_idx_clamped > 0 {
                                        // Custom user: restore last dir, then fall back to
                                        // global initial dir. home_dir is only for env vars,
                                        // not the working directory.
                                        last_dir.or_else(|| {
                                            if self.settings.claude_initial_dir.is_empty() { None }
                                            else { Some(self.settings.claude_initial_dir.clone()) }
                                        })
                                    } else {
                                        // Default user: global initial dir
                                        if self.settings.claude_initial_dir.is_empty() { None }
                                        else { Some(self.settings.claude_initial_dir.clone()) }
                                    }
                                }
                                ShellKind::Custom(exe) => {
                                    self.settings.custom_shells.iter()
                                        .find(|c| &c.cmd == exe)
                                        .and_then(|c| if c.dir.is_empty() { None } else { Some(c.dir.clone()) })
                                }
                                _ => last_dir,
                            };
                            // Temporarily set selected_claude_user to the saved value so
                            // spawn_shell_ex sets HOME/USERPROFILE/CLAUDE_CONFIG_DIR correctly.
                            let prev_user = self.settings.selected_claude_user;
                            self.settings.selected_claude_user = saved_user_idx_clamped;
                            self.spawn_shell_ex(
                                kind,
                                Some(name),
                                restore_dir,
                                Some(egui::Pos2::new(pos_x, pos_y)),
                                Some(egui::Vec2::new(width, height)),
                            );
                            self.settings.selected_claude_user = prev_user;
                        }
                        if let Some(idx) = focused_idx {
                            if let Some(win) = self.wm.windows.get(idx) {
                                let id = win.id.clone();
                                self.wm.focus(&id);
                            }
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

    /// Draws the floating settings window. Returns `true` if the window should
    /// stay open, `false` if the user closed it (so the caller can save).
    fn draw_settings_window(&mut self, ctx: &egui::Context) -> bool {
        let mut open = true;
        let mut save_and_close = false;
        let mut reset = false;

        egui::Window::new("⚙  SETTINGS")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .fixed_size(Vec2::new(380.0, 0.0)) // height auto-fits
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(SURFACE)
                    .stroke(Stroke::new(1.0, BORDER))
                    .rounding(Rounding::same(6.0))
                    .inner_margin(egui::Margin::same(16.0)),
            )
            .show(ctx, |ui| {
                ui.style_mut().visuals.override_text_color = Some(TEXT);

                // ── APPEARANCE ────────────────────────────────────────────
                settings_section(ui, "APPEARANCE");
                egui::Grid::new("s_appearance")
                    .num_columns(2)
                    .spacing([16.0, 8.0])
                    .min_col_width(110.0)
                    .show(ui, |ui| {
                        ui.label(settings_label("Font Size"));
                        ui.add(
                            egui::Slider::new(&mut self.settings.font_size, 8.0..=24.0)
                                .step_by(0.5)
                                .suffix(" px")
                                .clamp_to_range(true),
                        );
                        ui.end_row();

                        ui.label(settings_label("Line Spacing"));
                        ui.add(
                            egui::Slider::new(&mut self.settings.line_height_scale, 1.0..=2.0)
                                .step_by(0.05)
                                .fixed_decimals(2)
                                .suffix("×")
                                .clamp_to_range(true),
                        );
                        ui.end_row();
                    });

                ui.add_space(14.0);

                // ── TERMINAL ──────────────────────────────────────────────
                settings_section(ui, "TERMINAL");
                egui::Grid::new("s_terminal")
                    .num_columns(2)
                    .spacing([16.0, 8.0])
                    .min_col_width(110.0)
                    .show(ui, |ui| {
                        ui.label(settings_label("Default Shell"));
                        let default_shell_label = match &self.settings.default_shell {
                            ShellKind::Custom(exe) => self.settings.custom_shells.iter()
                                .find(|c| &c.cmd == exe)
                                .and_then(|c| if c.name.is_empty() { None } else { Some(c.name.clone()) })
                                .unwrap_or_else(|| exe.clone()),
                            k => k.label().to_string(),
                        };
                        egui::ComboBox::from_id_source("settings_shell_combo")
                            .selected_text(default_shell_label)
                            .width(150.0)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.settings.default_shell,
                                    ShellKind::Claude, "Claude",
                                );
                                for c in &self.settings.custom_shells {
                                    if c.cmd.is_empty() { continue; }
                                    let label = if c.name.is_empty() { c.cmd.clone() } else { c.name.clone() };
                                    ui.selectable_value(
                                        &mut self.settings.default_shell,
                                        ShellKind::Custom(c.cmd.clone()), label,
                                    );
                                }
                                ui.selectable_value(
                                    &mut self.settings.default_shell,
                                    ShellKind::PowerShell, "PowerShell",
                                );
                                ui.selectable_value(
                                    &mut self.settings.default_shell,
                                    ShellKind::Cmd, "CMD",
                                );
                                ui.selectable_value(
                                    &mut self.settings.default_shell,
                                    ShellKind::Bash, "Bash",
                                );
                            });
                        ui.end_row();

                        ui.label(settings_label("PTY Columns"));
                        ui.add(
                            egui::Slider::new(&mut self.settings.pty_cols, 40u16..=300u16)
                                .clamp_to_range(true),
                        );
                        ui.end_row();

                        ui.label(settings_label("PTY Rows"));
                        ui.add(
                            egui::Slider::new(&mut self.settings.pty_rows, 10u16..=100u16)
                                .clamp_to_range(true),
                        );
                        ui.end_row();
                    });

                ui.add_space(14.0);

                // ── CLAUDE ────────────────────────────────────────────────
                settings_section(ui, "CLAUDE");
                egui::Grid::new("s_claude")
                    .num_columns(2)
                    .spacing([16.0, 8.0])
                    .min_col_width(110.0)
                    .show(ui, |ui| {
                        ui.label(settings_label("Directory"));
                        ui.add(
                            egui::TextEdit::singleline(&mut self.settings.claude_initial_dir)
                                .font(FontId::new(11.0, FontFamily::Monospace))
                                .desired_width(150.0)
                                .hint_text("default working dir"),
                        );
                        ui.end_row();

                        ui.label(settings_label("Skip Permissions"));
                        ui.checkbox(&mut self.settings.claude_skip_permissions, "");
                        ui.end_row();

                        ui.label(settings_label("Telegram"));
                        ui.checkbox(&mut self.settings.claude_telegram, "");
                        ui.end_row();

                        ui.label(settings_label("5h Token Limit"));
                        let mut h5_str = if self.settings.token_5h_limit == 0 {
                            String::new()
                        } else {
                            self.settings.token_5h_limit.to_string()
                        };
                        if ui.add(
                            egui::TextEdit::singleline(&mut h5_str)
                                .font(FontId::new(11.0, FontFamily::Monospace))
                                .desired_width(100.0)
                                .hint_text("0 = show raw"),
                        ).changed() {
                            self.settings.token_5h_limit = h5_str.trim().parse().unwrap_or(0);
                        }
                        ui.end_row();

                        ui.label(settings_label("7d Token Limit"));
                        let mut wk_str = if self.settings.token_week_limit == 0 {
                            String::new()
                        } else {
                            self.settings.token_week_limit.to_string()
                        };
                        if ui.add(
                            egui::TextEdit::singleline(&mut wk_str)
                                .font(FontId::new(11.0, FontFamily::Monospace))
                                .desired_width(100.0)
                                .hint_text("0 = show raw"),
                        ).changed() {
                            self.settings.token_week_limit = wk_str.trim().parse().unwrap_or(0);
                        }
                        ui.end_row();
                    });

                ui.add_space(10.0);

                // ── CLAUDE USERS ──────────────────────────────────────────
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("USERS")
                            .font(FontId::new(10.0, FontFamily::Monospace))
                            .color(TEXT_DIM),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(
                            egui::Button::new(
                                RichText::new("+ Add")
                                    .font(FontId::new(10.0, FontFamily::Monospace))
                                    .color(ACCENT),
                            )
                            .fill(Color32::TRANSPARENT)
                            .stroke(Stroke::new(0.6, ACCENT.linear_multiply(0.5)))
                            .min_size(Vec2::new(50.0, 18.0)),
                        ).clicked() {
                            self.settings.claude_users.push(ClaudeUser::default());
                        }
                    });
                });
                ui.label(
                    RichText::new("  Set HOME / USERPROFILE / CLAUDE_CONFIG_DIR per user")
                        .font(FontId::new(10.0, FontFamily::Monospace))
                        .color(TEXT_DIM),
                );
                ui.add_space(4.0);

                let mut remove_user_idx: Option<usize> = None;
                for (i, user) in self.settings.claude_users.iter_mut().enumerate() {
                    let is_default = i == 0;
                    egui::Frame::none()
                        .fill(SURFACE2)
                        .stroke(Stroke::new(0.5, BORDER))
                        .rounding(Rounding::same(3.0))
                        .inner_margin(egui::Margin::same(6.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(if is_default { "Default".to_string() } else { format!("User {}", i) })
                                        .font(FontId::new(10.0, FontFamily::Monospace))
                                        .color(TEXT_DIM),
                                );
                                if !is_default {
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if ui.add(
                                            egui::Button::new(
                                                RichText::new("✕")
                                                    .font(FontId::new(10.0, FontFamily::Monospace))
                                                    .color(RED),
                                            )
                                            .fill(Color32::TRANSPARENT)
                                            .frame(false),
                                        ).clicked() {
                                            remove_user_idx = Some(i);
                                        }
                                    });
                                }
                            });
                            if is_default {
                                // Default user: name is fixed; home_dir editable for token-scan only
                                // (HOME env var is never overridden for idx 0 when spawning PTY).
                                egui::Grid::new("s_user_default")
                                    .num_columns(2)
                                    .spacing([8.0, 4.0])
                                    .min_col_width(80.0)
                                    .show(ui, |ui| {
                                        ui.label(settings_label("Name"));
                                        ui.label(
                                            RichText::new("Default")
                                                .font(FontId::new(11.0, FontFamily::Monospace))
                                                .color(TEXT_DIM),
                                        );
                                        ui.end_row();
                                        ui.label(settings_label("Stats Home"));
                                        ui.add(
                                            egui::TextEdit::singleline(&mut user.home_dir)
                                                .font(FontId::new(11.0, FontFamily::Monospace))
                                                .desired_width(200.0)
                                                .hint_text(r"e.g. D:\home\bert  (token scan only)"),
                                        );
                                        ui.end_row();
                                    });
                            } else {
                                egui::Grid::new(format!("s_user_{}", i))
                                    .num_columns(2)
                                    .spacing([8.0, 4.0])
                                    .min_col_width(80.0)
                                    .show(ui, |ui| {
                                        ui.label(settings_label("Name"));
                                        ui.add(
                                            egui::TextEdit::singleline(&mut user.name)
                                                .font(FontId::new(11.0, FontFamily::Monospace))
                                                .desired_width(200.0)
                                                .hint_text("display name"),
                                        );
                                        ui.end_row();
                                        ui.label(settings_label("Home Dir"));
                                        ui.add(
                                            egui::TextEdit::singleline(&mut user.home_dir)
                                                .font(FontId::new(11.0, FontFamily::Monospace))
                                                .desired_width(200.0)
                                                .hint_text(r"e.g. D:\home\alice"),
                                        );
                                        ui.end_row();
                                    });
                            }
                        });
                    ui.add_space(4.0);
                }
                if let Some(i) = remove_user_idx {
                    self.settings.claude_users.remove(i);
                    if self.settings.selected_claude_user >= self.settings.claude_users.len()
                        && !self.settings.claude_users.is_empty()
                    {
                        self.settings.selected_claude_user = self.settings.claude_users.len() - 1;
                    }
                }

                ui.add_space(14.0);

                // ── CUSTOM SHELLS ─────────────────────────────────────────
                ui.horizontal(|ui| {
                    settings_section(ui, "CUSTOM SHELLS");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(
                            egui::Button::new(
                                RichText::new("+ Add")
                                    .font(FontId::new(10.0, FontFamily::Monospace))
                                    .color(ACCENT),
                            )
                            .fill(Color32::TRANSPARENT)
                            .stroke(Stroke::new(0.6, ACCENT.linear_multiply(0.5)))
                            .min_size(Vec2::new(50.0, 18.0)),
                        ).clicked() {
                            self.settings.custom_shells.push(CustomShellConfig::default());
                        }
                    });
                });
                ui.label(
                    RichText::new("  Shown in toolbar when Command is non-empty")
                        .font(FontId::new(10.0, FontFamily::Monospace))
                        .color(TEXT_DIM),
                );
                ui.add_space(4.0);

                let mut remove_idx: Option<usize> = None;
                for (i, entry) in self.settings.custom_shells.iter_mut().enumerate() {
                    egui::Frame::none()
                        .fill(SURFACE2)
                        .stroke(Stroke::new(0.5, BORDER))
                        .rounding(Rounding::same(3.0))
                        .inner_margin(egui::Margin::same(6.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!("Entry {}", i + 1))
                                        .font(FontId::new(10.0, FontFamily::Monospace))
                                        .color(TEXT_DIM),
                                );
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.add(
                                        egui::Button::new(
                                            RichText::new("✕")
                                                .font(FontId::new(10.0, FontFamily::Monospace))
                                                .color(RED),
                                        )
                                        .fill(Color32::TRANSPARENT)
                                        .frame(false),
                                    ).clicked() {
                                        remove_idx = Some(i);
                                    }
                                });
                            });
                            egui::Grid::new(format!("s_custom_{}", i))
                                .num_columns(2)
                                .spacing([8.0, 4.0])
                                .min_col_width(80.0)
                                .show(ui, |ui| {
                                    ui.label(settings_label("Name"));
                                    ui.add(
                                        egui::TextEdit::singleline(&mut entry.name)
                                            .font(FontId::new(11.0, FontFamily::Monospace))
                                            .desired_width(200.0)
                                            .hint_text("display name"),
                                    );
                                    ui.end_row();
                                    ui.label(settings_label("Command"));
                                    ui.add(
                                        egui::TextEdit::singleline(&mut entry.cmd)
                                            .font(FontId::new(11.0, FontFamily::Monospace))
                                            .desired_width(200.0)
                                            .hint_text("e.g. wsl.exe"),
                                    );
                                    ui.end_row();
                                    ui.label(settings_label("Directory"));
                                    ui.add(
                                        egui::TextEdit::singleline(&mut entry.dir)
                                            .font(FontId::new(11.0, FontFamily::Monospace))
                                            .desired_width(200.0)
                                            .hint_text("initial working dir"),
                                    );
                                    ui.end_row();
                                    ui.label(settings_label("Startup Cmd"));
                                    ui.add(
                                        egui::TextEdit::singleline(&mut entry.startup_cmd)
                                            .font(FontId::new(11.0, FontFamily::Monospace))
                                            .desired_width(200.0)
                                            .hint_text("run silently on open"),
                                    );
                                    ui.end_row();
                                });
                        });
                    ui.add_space(4.0);
                }
                if let Some(i) = remove_idx {
                    self.settings.custom_shells.remove(i);
                }

                ui.add_space(4.0);
                ui.label(
                    RichText::new("  PTY size changes apply to new sessions only")
                        .font(FontId::new(10.0, FontFamily::Monospace))
                        .color(TEXT_DIM),
                );

                ui.add_space(14.0);

                // ── LAYOUT ────────────────────────────────────────────────
                settings_section(ui, "LAYOUT");
                egui::Grid::new("s_layout")
                    .num_columns(2)
                    .spacing([16.0, 8.0])
                    .min_col_width(110.0)
                    .show(ui, |ui| {
                        ui.label(settings_label("Sidebar Width"));
                        ui.add(
                            egui::Slider::new(&mut self.settings.sidebar_width, 80.0..=300.0)
                                .suffix(" px")
                                .clamp_to_range(true),
                        );
                        ui.end_row();
                    });

                ui.add_space(16.0);
                ui.separator();
                ui.add_space(10.0);

                // ── Action buttons ────────────────────────────────────────
                ui.horizontal(|ui| {
                    if ui.add(
                        egui::Button::new(
                            RichText::new("RESET DEFAULTS")
                                .font(FontId::new(11.0, FontFamily::Monospace))
                                .color(YELLOW),
                        )
                        .fill(Color32::TRANSPARENT)
                        .stroke(Stroke::new(0.8, YELLOW.linear_multiply(0.5)))
                        .min_size(Vec2::new(140.0, 28.0)),
                    ).clicked() {
                        reset = true;
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(
                            egui::Button::new(
                                RichText::new("SAVE & CLOSE")
                                    .font(FontId::new(11.0, FontFamily::Monospace))
                                    .color(Color32::WHITE),
                            )
                            .fill(ACCENT_DIM)
                            .min_size(Vec2::new(130.0, 28.0)),
                        ).clicked() {
                            save_and_close = true;
                        }
                    });
                });
            });

        if reset {
            self.settings = AppSettings::default();
        }
        if save_and_close {
            self.save_settings();
            return false;
        }
        // Sync toolbar dropdown with any default_shell change
        self.new_shell_kind = self.settings.default_shell.clone();
        open
    }
}

/// Parse an ISO-8601 UTC timestamp string into Unix seconds.
/// Handles `"YYYY-MM-DDTHH:MM:SS..."` (trailing fractional seconds / offset ignored).
fn parse_iso_to_unix(s: &str) -> u64 {
    let b = s.as_bytes();
    if b.len() < 19 { return 0; }
    fn d(b: &[u8]) -> u64 { b.iter().fold(0u64, |a, &c| a * 10 + (c.wrapping_sub(b'0')) as u64) }
    let (y, mo, day) = (d(&b[0..4]), d(&b[5..7]), d(&b[8..10]));
    let (h, mi, sec) = (d(&b[11..13]), d(&b[14..16]), d(&b[17..19]));
    // Howard Hinnant's civil-to-days → Unix epoch
    let m  = if mo > 2 { mo } else { mo + 12 };
    let y2 = if mo > 2 { y  } else { y - 1   };
    let era = y2 / 400;
    let yoe = y2 - era * 400;
    let doy = (153 * (m - 3) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = (era * 146097 + doe) as i64 - 719_468;
    if days < 0 { return 0; }
    days as u64 * 86_400 + h * 3_600 + mi * 60 + sec
}

/// Format a token count compactly: `"1.2M"`, `"123k"`, or `"99"`.
fn fmt_tok(n: u64) -> String {
    if n >= 1_000_000 { format!("{:.1}M", n as f64 / 1_000_000.0) }
    else if n >= 1_000 { format!("{}k", n / 1_000) }
    else               { n.to_string() }
}

/// Format a duration in seconds as `"2d03h"`, `"4h32m"`, `"12m05s"`, or `"0s"`.
fn fmt_dur(secs: u64) -> String {
    if secs >= 86_400 { format!("{}d{:02}h", secs / 86_400, (secs % 86_400) / 3_600) }
    else if secs >= 3_600 { format!("{}h{:02}m", secs / 3_600, (secs % 3_600) / 60) }
    else if secs >= 60    { format!("{}m{:02}s", secs / 60, secs % 60) }
    else                  { format!("{}s", secs) }
}


/// Scan `{home}/.claude/projects/**/*.jsonl` and aggregate token usage for
/// the rolling 5-hour and 7-day windows.
///
/// Follows the ccusage-rs approach:
///   - walkdir recursive scan
///   - only `type == "assistant"` lines
///   - deduplicate by `message_id` (tool-call rounds repeat the same usage block)
///   - sum input + output + cache_creation + cache_read tokens
fn compute_token_stats(home: &str) -> TokenCache {
    use std::collections::HashSet;
    use std::io::{BufRead, BufReader};
    use std::time::{SystemTime, UNIX_EPOCH};
    use walkdir::WalkDir;

    let now_s = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let cut5h = now_s.saturating_sub(5 * 3_600);
    let cut7d = now_s.saturating_sub(7 * 24 * 3_600);

    let mut tok5 = 0u64;
    let mut tok7 = 0u64;
    let mut old5: Option<u64> = None;
    let mut old7: Option<u64> = None;
    let mut seen: HashSet<String> = HashSet::new();

    let proj_dir = format!("{}/.claude/projects", home);

    for entry in WalkDir::new(&proj_dir).follow_links(false).into_iter().flatten() {
        if !entry.file_type().is_file() { continue; }
        if entry.path().extension().and_then(|e| e.to_str()) != Some("jsonl") { continue; }
        let Ok(file) = std::fs::File::open(entry.path()) else { continue };
        for line in BufReader::new(file).lines().flatten() {
            if !line.contains("\"assistant\"") || !line.contains("\"usage\"") { continue; }
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
            // Only assistant messages carry billable usage
            if v.get("type").and_then(|x| x.as_str()) != Some("assistant") { continue; }
            let ts_s = v.get("timestamp").and_then(|x| x.as_str())
                        .map(parse_iso_to_unix).unwrap_or(0);
            if ts_s == 0 || ts_s < cut7d { continue; }
            // Deduplicate by message_id (parallel tool calls repeat the same block)
            let msg_id = v.get("message").and_then(|m| m.get("id"))
                          .and_then(|x| x.as_str()).unwrap_or("").to_string();
            if !msg_id.is_empty() && !seen.insert(msg_id) { continue; }
            let Some(usage) = v.get("message").and_then(|m| m.get("usage")) else { continue };
            // Count input + output + cache_write only.
            // cache_read_input_tokens are cheap (10% cost) and NOT counted toward
            // Claude's rate-limit windows (matches what /usage reports).
            let tok =
                usage.get("input_tokens").and_then(|x| x.as_u64()).unwrap_or(0)
                + usage.get("output_tokens").and_then(|x| x.as_u64()).unwrap_or(0)
                + usage.get("cache_creation_input_tokens").and_then(|x| x.as_u64()).unwrap_or(0);
            if tok == 0 { continue; }
            tok7 += tok;
            old7 = Some(old7.map(|o| o.min(ts_s)).unwrap_or(ts_s));
            if ts_s >= cut5h {
                tok5 += tok;
                old5 = Some(old5.map(|o| o.min(ts_s)).unwrap_or(ts_s));
            }
        }
    }

    TokenCache {
        tokens_5h: tok5, tokens_week: tok7, oldest_5h: old5, oldest_week: old7,
        last_scan: std::time::Instant::now(),
    }
}

fn extract_selection_text(
    lines: &[Vec<crate::terminal_buffer::TerminalCell>],
    start: (usize, usize),
    end: (usize, usize),
) -> String {
    let mut out = String::new();
    for row in start.0..=end.0 {
        let Some(line) = lines.get(row) else { break };
        let c0 = if row == start.0 { start.1 } else { 0 };
        let c1 = if row == end.0 { end.1 } else { line.len() };
        for col in c0..c1.min(line.len()) {
            out.push(line[col].ch);
        }
        if row < end.0 { out.push('\n'); }
    }
    out.lines().map(|l| l.trim_end()).collect::<Vec<_>>().join("\n")
}

impl eframe::App for MultiCliApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(std::time::Duration::from_millis(33));

        // Derive rendering metrics from settings each frame
        let char_w   = self.settings.char_w();
        let line_h   = self.settings.line_h();
        let sidebar_w = self.settings.sidebar_width;
        let font_sz  = self.settings.font_size;

        // Auto-save every 60 seconds
        if self.last_save.elapsed() >= std::time::Duration::from_secs(60) {
            self.save_state();
            self.last_save = std::time::Instant::now();
        }

        // Refresh token stats every 10 seconds — one cache entry per distinct home dir
        {
            let sys_home = std::env::var("USERPROFILE")
                .or_else(|_| std::env::var("HOME"))
                .unwrap_or_default();
            // Collect all unique home dirs referenced by active Claude sessions
            let homes: std::collections::HashSet<String> = self.wm.windows.iter()
                .filter_map(|w| self.session_user_idx.get(&w.session_id).copied())
                .map(|idx| {
                    let idx = idx.min(self.settings.claude_users.len().saturating_sub(1));
                    let cfg = self.settings.claude_users.get(idx)
                        .map(|u| u.home_dir.as_str()).unwrap_or("");
                    if cfg.is_empty() { sys_home.clone() } else { cfg.to_string() }
                })
                .filter(|h| !h.is_empty())
                .collect();
            for home in homes {
                let stale = self.token_caches.get(&home)
                    .map(|c| c.last_scan.elapsed() >= std::time::Duration::from_secs(10))
                    .unwrap_or(true);
                if stale {
                    self.token_caches.insert(home.clone(), compute_token_stats(&home));
                }
            }
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
                    let combo_label = match &self.new_shell_kind {
                        ShellKind::Custom(exe) => {
                            self.settings.custom_shells.iter()
                                .find(|c| &c.cmd == exe)
                                .and_then(|c| if c.name.is_empty() { None } else { Some(c.name.clone()) })
                                .unwrap_or_else(|| exe.clone())
                        }
                        other => other.label().to_string(),
                    };
                    let custom_entries: Vec<(String, String)> = self.settings.custom_shells.iter()
                        .filter(|c| !c.cmd.is_empty())
                        .map(|c| (c.cmd.clone(), if c.name.is_empty() { c.cmd.clone() } else { c.name.clone() }))
                        .collect();
                    egui::ComboBox::from_id_source("shell_kind")
                        .selected_text(combo_label)
                        .width(110.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.new_shell_kind, ShellKind::Claude, "Claude");
                            for (cmd, label) in custom_entries {
                                ui.selectable_value(
                                    &mut self.new_shell_kind,
                                    ShellKind::Custom(cmd),
                                    label,
                                );
                            }
                            ui.selectable_value(&mut self.new_shell_kind, ShellKind::PowerShell, "PowerShell");
                            ui.selectable_value(&mut self.new_shell_kind, ShellKind::Cmd, "CMD");
                            ui.selectable_value(&mut self.new_shell_kind, ShellKind::Bash, "Bash");
                        });

                    // User selector: only visible when 2 or more Claude users are configured
                    // (index 0 is always the implicit Default; custom users start at index 1)
                    if self.settings.claude_users.len() >= 2 {
                        ui.add_space(4.0);
                        if self.settings.selected_claude_user >= self.settings.claude_users.len() {
                            self.settings.selected_claude_user = 0;
                        }
                        let user_label = if self.settings.selected_claude_user == 0 {
                            "Default".to_string()
                        } else {
                            self.settings.claude_users
                                .get(self.settings.selected_claude_user)
                                .map(|u| if u.name.is_empty() {
                                    format!("User {}", self.settings.selected_claude_user)
                                } else {
                                    u.name.clone()
                                })
                                .unwrap_or_else(|| "Default".to_string())
                        };
                        egui::ComboBox::from_id_source("claude_user_combo")
                            .selected_text(format!("👤 {}", user_label))
                            .width(120.0)
                            .show_ui(ui, |ui| {
                                // Default is always first
                                ui.selectable_value(
                                    &mut self.settings.selected_claude_user,
                                    0,
                                    "Default",
                                );
                                // Custom users follow (index 1+)
                                for i in 1..self.settings.claude_users.len() {
                                    let label = self.settings.claude_users.get(i)
                                        .map(|u| if u.name.is_empty() {
                                            format!("User {}", i)
                                        } else {
                                            u.name.clone()
                                        })
                                        .unwrap_or_else(|| format!("User {}", i));
                                    ui.selectable_value(
                                        &mut self.settings.selected_claude_user,
                                        i,
                                        label,
                                    );
                                }
                            });
                    }

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

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let settings_color = if self.show_settings { ACCENT } else { TEXT_DIM };
                        if toolbar_btn(ui, "⚙ SETTINGS", settings_color).clicked() {
                            self.show_settings = !self.show_settings;
                        }
                    });
                });
            });

        // --- Sidebar ---
        egui::SidePanel::left("sidebar")
            .exact_width(sidebar_w)
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
                                    .desired_width(sidebar_w - 8.0),
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
                            let btn_w = sidebar_w - 30.0; // leave room for ✕
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
                let pointer_pressed = ctx.input(|i| i.pointer.primary_pressed());
                let pointer_released = ctx.input(|i| i.pointer.primary_released());
                let secondary_pressed = ctx.input(|i| i.pointer.secondary_pressed());
                let pointer_dbl = ctx.input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary));

                if pointer_released {
                    self.drag_state = None;
                    self.resize_state = None;
                    self.sel_dragging = false;
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
                let mut new_selection: Option<TextSelection> = None;
                let mut sel_focus_update: Option<(String, usize, usize)> = None;
                let mut start_sel_drag = false;
                let mut new_context_menu: Option<(Pos2, String)> = None;
                let mut dir_change_request: Option<(String, String)> = None; // (session_id, current_cwd)

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
                    let mut visible_start: usize = 0;
                    if let Some(session) = self.sessions.get(&session_id) {
                        let buf = session.buffer.lock().unwrap();
                        let visible_rows = ((size.y - STATUS_H) / line_h) as usize;
                        let lines = buf.visible_lines();
                        // In alternate screen (TUI), show from scroll_offset (always 0).
                        // In normal mode, follow the cursor.
                        let start = if buf.alternate_screen {
                            buf.scroll_offset
                        } else {
                            (buf.cursor_row + 1).saturating_sub(visible_rows)
                        };
                        visible_start = start;

                        // Clip all terminal drawing to the content rect
                        let term_painter = painter.with_clip_rect(term_rect);
                        for (row_idx, row) in lines[start..].iter().enumerate() {
                            let y = term_rect.min.y + row_idx as f32 * line_h + 2.0;
                            let buf_row = start + row_idx;

                            // Selection highlight (drawn before characters)
                            if let Some(sel) = &self.text_selection {
                                if sel.window_id == win_id && !sel.is_empty() {
                                    let ((sr, sc), (er, ec)) = sel.range();
                                    if buf_row >= sr && buf_row <= er {
                                        let c0 = if buf_row == sr { sc } else { 0 };
                                        let c1 = if buf_row == er { ec } else { row.len().max(1) };
                                        let x0 = term_rect.min.x + 4.0 + c0 as f32 * char_w;
                                        let x1 = (term_rect.min.x + 4.0 + c1 as f32 * char_w)
                                            .min(term_rect.max.x);
                                        term_painter.rect_filled(
                                            Rect::from_min_max(
                                                Pos2::new(x0, y - 2.0),
                                                Pos2::new(x1, y + line_h - 2.0),
                                            ),
                                            Rounding::ZERO,
                                            Color32::from_rgba_unmultiplied(80, 200, 160, 80),
                                        );
                                    }
                                }
                            }

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
                                        FontId::new(font_sz, FontFamily::Monospace),
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
                        let shell_kind = self.sessions.get(&session_id).map(|s| s.kind.clone());
                        let is_managed = matches!(shell_kind.as_ref(), Some(ShellKind::Claude) | Some(ShellKind::Custom(_)));
                        let is_claude = matches!(shell_kind.as_ref(), Some(ShellKind::Claude));
                        let cwd = if is_managed {
                            // Claude CLI overwrites title to "Claude Code"; use our tracked dir.
                            self.session_dirs.get(&session_id)
                                .cloned()
                                .unwrap_or_else(|| if t.is_empty() { "~".to_string() } else { t })
                        } else {
                            if t.is_empty() { "~".to_string() } else { t }
                        };

                        // Left: current working directory
                        painter.text(
                            Pos2::new(status_rect.min.x + 6.0, status_rect.center().y),
                            egui::Align2::LEFT_CENTER,
                            &cwd,
                            FontId::new(10.0, FontFamily::Monospace),
                            TEXT_DIM,
                        );

                        // Right (Claude only): user | 5h token% | reset | 7d token% | reset
                        if is_claude {
                            // Resolve which user/home this session belongs to
                            let sys_home = std::env::var("USERPROFILE")
                                .or_else(|_| std::env::var("HOME"))
                                .unwrap_or_default();
                            let user_idx = self.session_user_idx.get(&session_id).copied()
                                .unwrap_or(0)
                                .min(self.settings.claude_users.len().saturating_sub(1));
                            let user_home = {
                                let cfg = self.settings.claude_users.get(user_idx)
                                    .map(|u| u.home_dir.as_str()).unwrap_or("");
                                if cfg.is_empty() { sys_home } else { cfg.to_string() }
                            };
                            let user_name = if user_idx == 0 {
                                "Default".to_string()
                            } else {
                                self.settings.claude_users.get(user_idx)
                                    .map(|u| if u.name.is_empty() { format!("User {}", user_idx) } else { u.name.clone() })
                                    .unwrap_or_else(|| "Default".to_string())
                            };
                            let cache = self.token_caches.get(&user_home);
                            let now_s = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default().as_secs();
                            // 5h usage % from JSONL counts / configured limit
                            let h5_usage = if self.settings.token_5h_limit > 0 {
                                let pct = (cache.map(|c| c.tokens_5h).unwrap_or(0) * 100)
                                    / self.settings.token_5h_limit.max(1);
                                format!("{}%", pct)
                            } else {
                                fmt_tok(cache.map(|c| c.tokens_5h).unwrap_or(0))
                            };
                            // 5h reset: rolling window from oldest entry
                            let h5_reset = cache.and_then(|c| c.oldest_5h)
                                .map(|o| fmt_dur((o + 5 * 3_600).saturating_sub(now_s)))
                                .unwrap_or_else(|| "--".to_string());
                            // 7d usage % from JSONL counts / configured limit
                            let wk_usage = if self.settings.token_week_limit > 0 {
                                let pct = (cache.map(|c| c.tokens_week).unwrap_or(0) * 100)
                                    / self.settings.token_week_limit.max(1);
                                format!("{}%", pct)
                            } else {
                                fmt_tok(cache.map(|c| c.tokens_week).unwrap_or(0))
                            };
                            // 7d reset: rolling window from oldest entry
                            let wk_reset = cache.and_then(|c| c.oldest_week)
                                .map(|o| fmt_dur((o + 7 * 24 * 3_600).saturating_sub(now_s)))
                                .unwrap_or_else(|| "--".to_string());
                            // Format: user | 5h:XX% | 3h41m | 7d:XX% | 6d17h
                            let right_text = format!(
                                "{} \u{2502} 5h:{} \u{2502} {} \u{2502} 7d:{} \u{2502} {}",
                                user_name, h5_usage, h5_reset, wk_usage, wk_reset
                            );
                            painter.text(
                                Pos2::new(status_rect.max.x - 6.0, status_rect.center().y),
                                egui::Align2::RIGHT_CENTER,
                                &right_text,
                                FontId::new(10.0, FontFamily::Monospace),
                                TEXT_DIM,
                            );
                        }

                        // Double-click on status bar → dir change dialog
                        if pointer_dbl {
                            if let Some(mpos) = pointer_pos {
                                if status_rect.contains(mpos) && is_managed {
                                    dir_change_request = Some((session_id.clone(), cwd));
                                }
                            }
                        }
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

                        // Cursor icon — higher-z windows override lower ones.
                        // win_rect.contains resets cursor when interior covers the mouse.
                        if self.drag_state.is_none() && self.resize_state.is_none() {
                            if edges.any() {
                                ctx.output_mut(|o| o.cursor_icon = edges.cursor());
                            } else if win_rect.contains(mpos) {
                                ctx.output_mut(|o| o.cursor_icon = egui::CursorIcon::Default);
                            }
                        }

                        if pointer_down && self.drag_state.is_none() && self.resize_state.is_none() {
                            if win_rect.expand(ez).contains(mpos) {
                                // This window claims the mouse — override any interaction
                                // that was set by a lower-z window in a previous iteration.
                                focus_request = Some(win_id.clone());
                                start_resize = None;
                                start_drag = None;

                                // Close button → hide window (session stays alive)
                                if (mpos - close_btn_center).length() < 7.0 {
                                    close_request = Some(win_id.clone());
                                }

                                // Edge resize (takes priority over titlebar drag)
                                if edges.any() {
                                    start_resize = Some((win_id.clone(), edges, win_rect.min, size, mpos));
                                } else if titlebar_rect.contains(mpos)
                                    && (mpos - close_btn_center).length() > 8.0
                                    && !self.sel_dragging
                                {
                                    // Titlebar drag
                                    let off = mpos - win_rect.min;
                                    start_drag = Some((win_id.clone(), off));
                                }
                            }
                        }

                        // Selection: start on primary press inside terminal area
                        if pointer_pressed && term_rect.contains(mpos)
                            && self.drag_state.is_none()
                            && self.resize_state.is_none()
                            && self.context_menu.is_none()
                        {
                            let col = ((mpos.x - term_rect.min.x - 4.0).max(0.0) / char_w) as usize;
                            let vis_r = ((mpos.y - term_rect.min.y - 2.0).max(0.0) / line_h) as usize;
                            new_selection = Some(TextSelection {
                                window_id: win_id.clone(),
                                anchor: (visible_start + vis_r, col),
                                focus:  (visible_start + vis_r, col),
                            });
                            start_sel_drag = true;
                        }

                        // Selection: extend focus while drag is active
                        if self.sel_dragging && pointer_down {
                            if self.text_selection.as_ref()
                                .map(|s| s.window_id.as_str()) == Some(win_id.as_str())
                            {
                                let col = ((mpos.x - term_rect.min.x - 4.0).max(0.0) / char_w) as usize;
                                let vis_r = ((mpos.y - term_rect.min.y - 2.0).max(0.0) / line_h) as usize;
                                sel_focus_update = Some((win_id.clone(), visible_start + vis_r, col));
                            }
                        }

                        // Right-click: open context menu
                        if secondary_pressed && win_rect.contains(mpos) {
                            new_context_menu = Some((mpos, win_id.clone()));
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
                if let Some(sel) = new_selection {
                    self.text_selection = Some(sel);
                }
                if let Some((wid, row, col)) = sel_focus_update {
                    if let Some(sel) = self.text_selection.as_mut() {
                        if sel.window_id == wid { sel.focus = (row, col); }
                    }
                }
                if start_sel_drag { self.sel_dragging = true; }
                if let Some(cm) = new_context_menu { self.context_menu = Some(cm); }
                if let Some((sid, cwd)) = dir_change_request {
                    self.dir_change_dialog = Some(DirChangeDialog { session_id: sid, new_dir: cwd });
                }

                // Keyboard input → direct PTY passthrough (shell-style)
                // Skip when the rename input, settings, or dir dialog is active to avoid dual input
                if self.renaming_id.is_none() && !self.show_settings && self.dir_change_dialog.is_none() {
                if let Some(fid) = self.wm.focused_id.clone() {
                    let win_info = self.wm.windows.iter().find(|w| w.id == fid).map(|w| {
                        (w.session_id.clone(), w.pos, w.size)
                    });

                    if let Some((sid, win_pos, win_size)) = win_info {
                        if let Some(session) = self.sessions.get(&sid) {
                            // Tell egui where the cursor is so the IME candidate window follows it
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

        // --- Settings window ---
        if self.show_settings {
            let was_open = true;
            let still_open = self.draw_settings_window(ctx);
            if was_open && !still_open {
                self.save_settings();
            }
            self.show_settings = still_open;
        }

        // --- Pending Claude relaunch (after /exit delay) ---
        if let Some(ref relaunch) = self.pending_relaunch {
            if std::time::Instant::now() >= relaunch.fire_at {
                let sid = relaunch.session_id.clone();
                let dir = relaunch.dir.clone();
                let claude_cmd = relaunch.claude_cmd.clone();
                if let Some(session) = self.sessions.get(&sid) {
                    let ps_cmd = format!(
                        "Set-Location '{}'; {}\r",
                        dir.replace('\'', "''"),
                        claude_cmd,
                    );
                    session.write_input(ps_cmd.as_bytes());
                }
                self.session_dirs.insert(sid, dir);
                self.pending_relaunch = None;
                ctx.request_repaint();
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }

        // --- Dir change dialog ---
        let mut dir_confirmed = false;
        let mut dir_cancelled = false;
        if self.dir_change_dialog.is_some() {
            egui::Window::new("Change Directory")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .frame(
                    egui::Frame::window(&ctx.style())
                        .fill(SURFACE2)
                        .stroke(Stroke::new(1.0, BORDER))
                        .inner_margin(egui::Margin::same(16.0)),
                )
                .show(ctx, |ui| {
                    ui.style_mut().visuals.override_text_color = Some(TEXT);
                    ui.label(settings_label("New Directory:"));
                    ui.add_space(6.0);
                    if let Some(dialog) = self.dir_change_dialog.as_mut() {
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut dialog.new_dir)
                                .font(FontId::new(11.0, FontFamily::Monospace))
                                .desired_width(300.0),
                        );
                        resp.request_focus();
                        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            dir_confirmed = true;
                        }
                    }
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.add(
                            egui::Button::new(
                                RichText::new("Cancel").font(FontId::new(11.0, FontFamily::Monospace)).color(TEXT_DIM)
                            ).fill(Color32::TRANSPARENT).stroke(Stroke::new(0.8, BORDER))
                        ).clicked() {
                            dir_cancelled = true;
                        }
                        ui.add_space(8.0);
                        if ui.add(
                            egui::Button::new(
                                RichText::new("Confirm").font(FontId::new(11.0, FontFamily::Monospace)).color(ACCENT)
                            ).fill(ACCENT.linear_multiply(0.15)).stroke(Stroke::new(0.8, ACCENT.linear_multiply(0.5)))
                        ).clicked() {
                            dir_confirmed = true;
                        }
                    });
                });
        }
        if dir_cancelled { self.dir_change_dialog = None; }
        if dir_confirmed {
            if let Some(dialog) = self.dir_change_dialog.take() {
                let kind = self.sessions.get(&dialog.session_id).map(|s| s.kind.clone());
                if matches!(kind.as_ref(), Some(ShellKind::Claude)) {
                    let mut claude_cmd = String::from("claude");
                    if self.settings.claude_skip_permissions {
                        claude_cmd.push_str(" --dangerously-skip-permissions");
                    }
                    if self.settings.claude_telegram {
                        claude_cmd.push_str(" --channels plugin:telegram@claude-plugins-official");
                    }
                    if let Some(session) = self.sessions.get(&dialog.session_id) {
                        session.write_input(b"/exit\r");
                    }
                    // Normalize drive-root paths: "G:" → "G:\" so Set-Location works correctly
                    let new_dir = {
                        let d = dialog.new_dir.trim_end_matches(['/', '\\']).to_string();
                        if d.len() == 2 && d.as_bytes()[1] == b':' { format!("{}\\", d) } else { d }
                    };
                    self.pending_relaunch = Some(PendingRelaunch {
                        session_id: dialog.session_id,
                        dir: new_dir,
                        claude_cmd,
                        fire_at: std::time::Instant::now() + std::time::Duration::from_millis(1000),
                    });
                }
            }
        }

        // --- Context menu (right-click copy/paste) ---
        let mut close_ctx_menu = false;
        if let Some((menu_pos, ref menu_win_id)) = self.context_menu.clone() {
            let sid = self.wm.windows.iter()
                .find(|w| &w.id == menu_win_id)
                .map(|w| w.session_id.clone());

            let inner = egui::Area::new(egui::Id::new("ctx_menu"))
                .fixed_pos(menu_pos)
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    egui::Frame::none()
                        .fill(SURFACE2)
                        .stroke(Stroke::new(1.0, BORDER))
                        .rounding(Rounding::same(4.0))
                        .inner_margin(egui::Margin::same(4.0))
                        .show(ui, |ui| {
                            ui.set_min_width(90.0);

                            let has_sel = self.text_selection.as_ref()
                                .map(|s| &s.window_id == menu_win_id && !s.is_empty())
                                .unwrap_or(false);

                            let copy_label = RichText::new("  Copy")
                                .font(FontId::new(12.0, FontFamily::Monospace))
                                .color(if has_sel { TEXT } else { TEXT_DIM });
                            if ui.add(egui::Button::new(copy_label)
                                .fill(Color32::TRANSPARENT)
                                .frame(false)
                                .min_size(Vec2::new(90.0, 22.0))
                            ).clicked() && has_sel {
                                if let Some(sel) = &self.text_selection {
                                    if let Some(w) = self.wm.windows.iter().find(|w| &w.id == &sel.window_id) {
                                        if let Some(session) = self.sessions.get(&w.session_id) {
                                            if let Ok(buf) = session.buffer.lock() {
                                                let lines = buf.visible_lines();
                                                let (sr, er) = sel.range();
                                                let text = extract_selection_text(&lines, sr, er);
                                                if let Ok(mut cb) = arboard::Clipboard::new() {
                                                    let _ = cb.set_text(text);
                                                }
                                            }
                                        }
                                    }
                                }
                                close_ctx_menu = true;
                            }

                            let paste_label = RichText::new("  Paste")
                                .font(FontId::new(12.0, FontFamily::Monospace))
                                .color(TEXT);
                            if ui.add(egui::Button::new(paste_label)
                                .fill(Color32::TRANSPARENT)
                                .frame(false)
                                .min_size(Vec2::new(90.0, 22.0))
                            ).clicked() {
                                if let Some(ref session_id) = sid {
                                    if let Some(session) = self.sessions.get(session_id) {
                                        if let Ok(mut cb) = arboard::Clipboard::new() {
                                            if let Ok(text) = cb.get_text() {
                                                session.write_input(text.as_bytes());
                                            }
                                        }
                                    }
                                }
                                close_ctx_menu = true;
                            }
                        });
                });

            // Close menu when clicking outside
            let menu_rect = inner.response.rect;
            let clicked_outside = ctx.input(|i| {
                (i.pointer.primary_pressed() || i.pointer.secondary_pressed())
                    && i.pointer.interact_pos().map_or(true, |p| !menu_rect.contains(p))
            });
            if clicked_outside { close_ctx_menu = true; }
        }
        if close_ctx_menu { self.context_menu = None; }

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
        // Don't intercept keys while the session rename input, settings, or dir dialog is active
        if self.renaming_id.is_some() || self.show_settings || self.dir_change_dialog.is_some() { return; }
        // Only intercept when a terminal window is focused
        let sid = self.wm.focused_id.as_ref()
            .and_then(|fid| self.wm.windows.iter().find(|w| &w.id == fid))
            .map(|w| w.session_id.clone());
        let Some(sid) = sid else { return };

        let mut to_send: Vec<Vec<u8>> = Vec::new();
        let mut to_remove: Vec<usize> = Vec::new();

        for (i, event) in raw_input.events.iter().enumerate() {
            let bytes: Option<&'static [u8]> = match event {
                egui::Event::Key { key, pressed: true, modifiers, .. } => key_to_bytes(key, modifiers),
                // egui converts Ctrl+C → Event::Copy before Key events on some platforms;
                // in a terminal Ctrl+C must always send ETX (interrupt), never clipboard copy.
                egui::Event::Copy => Some(b"\x03"),
                _ => None,
            };
            if let Some(b) = bytes {
                to_send.push(b.to_vec());
                to_remove.push(i);
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

fn settings_section(ui: &mut egui::Ui, title: &str) {
    ui.label(
        RichText::new(title)
            .font(FontId::new(10.0, FontFamily::Monospace))
            .color(ACCENT),
    );
    ui.add_space(6.0);
}

fn settings_label(text: &str) -> RichText {
    RichText::new(text)
        .font(FontId::new(11.0, FontFamily::Monospace))
        .color(TEXT_DIM)
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

fn settings_path() -> Option<std::path::PathBuf> {
    let dir = std::path::PathBuf::from(std::env::var("APPDATA").ok()?).join("multi-cli");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("settings.json"))
}

fn load_settings_from_disk() -> AppSettings {
    if let Some(path) = settings_path() {
        if let Ok(data) = std::fs::read_to_string(&path) {
            if let Ok(mut s) = serde_json::from_str::<AppSettings>(&data) {
                // Migrate zero limits from old saves — 0 means "not configured yet".
                if s.token_5h_limit == 0 { s.token_5h_limit = default_token_5h_limit(); }
                if s.token_week_limit == 0 { s.token_week_limit = default_token_week_limit(); }
                return s;
            }
        }
    }
    AppSettings::default()
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
