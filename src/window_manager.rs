use egui::Pos2;

const TITLEBAR_H: f32 = 28.0;

/// Controls how terminal windows are arranged in the workspace.
#[derive(Debug, Clone, PartialEq)]
pub enum LayoutMode {
    /// User drags windows freely; no automatic repositioning.
    Free,
    /// Windows are placed in an even grid (`cols = ceil(sqrt(n))`).
    Tile,
    /// Windows are stacked with a 28 px cascading offset per slot.
    Cascade,
}

/// A floating terminal window in the workspace.
/// Holds UI geometry and links to its [`ShellSession`] via [`session_id`](Self::session_id).
#[derive(Debug, Clone)]
pub struct ShellWindow {
    pub id: String,
    pub session_id: String,
    pub title: String,
    pub pos: Pos2,
    pub size: egui::Vec2,
    pub minimized: bool,
    pub focused: bool,
    pub z_order: usize,
}

impl ShellWindow {
    pub fn new(id: String, session_id: String, title: String, pos: Pos2) -> Self {
        Self {
            id,
            session_id,
            title,
            pos,
            size: egui::Vec2::new(560.0, 340.0),
            minimized: false,
            focused: false,
            z_order: 0,
        }
    }
}

/// Central controller that owns the list of [`ShellWindow`]s and applies
/// layout algorithms. `workspace_rect` is updated every frame from the egui
/// `CentralPanel` so that layout calculations use current screen coordinates.
pub struct WindowManager {
    pub windows: Vec<ShellWindow>,
    pub layout_mode: LayoutMode,
    /// ID of the currently keyboard-focused window, if any.
    pub focused_id: Option<String>,
    /// Available workspace area (set each frame by the renderer).
    pub workspace_rect: egui::Rect,
    /// Monotonically increasing counter used to assign z-order.
    z_counter: usize,
}

impl WindowManager {
    pub fn new() -> Self {
        Self {
            windows: Vec::new(),
            layout_mode: LayoutMode::Free,
            focused_id: None,
            workspace_rect: egui::Rect::from_min_size(
                Pos2::new(160.0, 40.0),
                egui::Vec2::new(1120.0, 720.0),
            ),
            z_counter: 0,
        }
    }

    pub fn add_window(&mut self, session_id: String, title: String) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let n = self.windows.len();
        let pos = self.default_pos(n);
        self.windows.push(ShellWindow::new(id.clone(), session_id, title, pos));
        self.focus(&id);
        id
    }

    fn default_pos(&self, idx: usize) -> Pos2 {
        let ws = self.workspace_rect;
        Pos2::new(
            ws.min.x + 20.0 + (idx as f32 * 30.0),
            ws.min.y + 20.0 + (idx as f32 * 30.0),
        )
    }

    pub fn focus(&mut self, id: &str) {
        self.z_counter += 1;
        let z = self.z_counter;
        for w in self.windows.iter_mut() {
            w.focused = w.id == id;
            if w.id == id {
                w.z_order = z;
                w.minimized = false;
            }
        }
        self.focused_id = Some(id.to_string());
    }

    #[allow(dead_code)]
    pub fn close_window(&mut self, id: &str) {
        self.windows.retain(|w| w.id != id);
        if self.focused_id.as_deref() == Some(id) {
            self.focused_id = self.windows.last().map(|w| w.id.clone());
        }
    }

    pub fn minimize_all(&mut self) {
        for w in self.windows.iter_mut() {
            w.minimized = true;
        }
        self.focused_id = None;
    }

    pub fn restore_all(&mut self) {
        for w in self.windows.iter_mut() {
            w.minimized = false;
        }
    }

    pub fn tile(&mut self) {
        self.layout_mode = LayoutMode::Tile;
        self.apply_layout();
    }

    pub fn cascade(&mut self) {
        self.layout_mode = LayoutMode::Cascade;
        self.apply_layout();
    }

    pub fn apply_layout(&mut self) {
        let visible: Vec<usize> = self
            .windows
            .iter()
            .enumerate()
            .filter(|(_, w)| !w.minimized)
            .map(|(i, _)| i)
            .collect();

        let n = visible.len();
        if n == 0 {
            return;
        }

        let ws = self.workspace_rect;
        let padding = 6.0;

        match self.layout_mode {
            LayoutMode::Tile => {
                let cols = (n as f32).sqrt().ceil() as usize;
                let rows = (n + cols - 1) / cols;
                let w = (ws.width() - padding * (cols + 1) as f32) / cols as f32;
                // cell_h is total height per grid cell; subtract TITLEBAR_H for content size
                let cell_h = (ws.height() - padding * (rows + 1) as f32) / rows as f32;
                let content_h = (cell_h - TITLEBAR_H).max(80.0);

                for (slot, &wi) in visible.iter().enumerate() {
                    let col = slot % cols;
                    let row = slot / cols;
                    self.windows[wi].pos = Pos2::new(
                        ws.min.x + padding + col as f32 * (w + padding),
                        ws.min.y + padding + row as f32 * (cell_h + padding),
                    );
                    self.windows[wi].size = egui::Vec2::new(w, content_h);
                }
            }
            LayoutMode::Cascade => {
                let base_w = ws.width() * 0.55;
                let base_h = ws.height() * 0.6 - TITLEBAR_H;
                let step = 28.0;
                for (slot, &wi) in visible.iter().enumerate() {
                    self.windows[wi].pos = Pos2::new(
                        ws.min.x + 10.0 + step * slot as f32,
                        ws.min.y + 10.0 + step * slot as f32,
                    );
                    self.windows[wi].size = egui::Vec2::new(base_w, base_h);
                }
            }
            LayoutMode::Free => {}
        }
    }

    #[allow(dead_code)]
    pub fn focused_session_id(&self) -> Option<&str> {
        self.focused_id.as_ref().and_then(|fid| {
            self.windows
                .iter()
                .find(|w| &w.id == fid)
                .map(|w| w.session_id.as_str())
        })
    }

    /// Return window indices sorted by ascending z-order (back-to-front paint order).
    pub fn sorted_windows(&self) -> Vec<usize> {
        let mut indices: Vec<usize> = (0..self.windows.len()).collect();
        indices.sort_by_key(|&i| self.windows[i].z_order);
        indices
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A WindowManager with a predictable 1000×600 workspace for deterministic layout math.
    fn make_wm() -> WindowManager {
        let mut wm = WindowManager::new();
        wm.workspace_rect = egui::Rect::from_min_size(
            Pos2::new(0.0, 0.0),
            egui::Vec2::new(1000.0, 600.0),
        );
        wm
    }

    // ── initial state ────────────────────────────────────────────────────────

    #[test]
    fn new_window_manager_starts_empty() {
        let wm = WindowManager::new();
        assert!(wm.windows.is_empty());
        assert_eq!(wm.layout_mode, LayoutMode::Free);
        assert!(wm.focused_id.is_none());
    }

    // ── ShellWindow defaults ─────────────────────────────────────────────────

    #[test]
    fn shell_window_new_has_correct_defaults() {
        let w = ShellWindow::new("id1".into(), "sid1".into(), "Test".into(), Pos2::new(10.0, 20.0));
        assert_eq!(w.size.x, 560.0);
        assert_eq!(w.size.y, 340.0);
        assert!(!w.minimized);
        assert!(!w.focused);
        assert_eq!(w.z_order, 0);
    }

    // ── add_window ───────────────────────────────────────────────────────────

    #[test]
    fn add_window_creates_and_auto_focuses() {
        let mut wm = make_wm();
        let id = wm.add_window("s1".into(), "Shell 1".into());
        assert_eq!(wm.windows.len(), 1);
        assert_eq!(wm.focused_id.as_deref(), Some(id.as_str()));
        assert!(wm.windows[0].focused);
    }

    #[test]
    fn add_multiple_windows_last_one_is_focused() {
        let mut wm = make_wm();
        wm.add_window("s1".into(), "S1".into());
        let id2 = wm.add_window("s2".into(), "S2".into());
        assert_eq!(wm.focused_id.as_deref(), Some(id2.as_str()));
        assert!(!wm.windows[0].focused);
        assert!(wm.windows[1].focused);
    }

    // ── focus ────────────────────────────────────────────────────────────────

    #[test]
    fn focus_switches_correctly() {
        let mut wm = make_wm();
        let id1 = wm.add_window("s1".into(), "S1".into());
        let id2 = wm.add_window("s2".into(), "S2".into());
        wm.focus(&id1);
        assert!(wm.windows.iter().find(|w| w.id == id1).unwrap().focused);
        assert!(!wm.windows.iter().find(|w| w.id == id2).unwrap().focused);
        assert_eq!(wm.focused_id.as_deref(), Some(id1.as_str()));
    }

    #[test]
    fn focus_unminimizes_the_target_window() {
        let mut wm = make_wm();
        let id = wm.add_window("s1".into(), "S1".into());
        wm.minimize_all();
        assert!(wm.windows[0].minimized);
        wm.focus(&id);
        assert!(!wm.windows[0].minimized);
    }

    #[test]
    fn focus_raises_z_order_above_others() {
        let mut wm = make_wm();
        let id1 = wm.add_window("s1".into(), "S1".into());
        wm.add_window("s2".into(), "S2".into());
        wm.focus(&id1);
        let z1 = wm.windows.iter().find(|w| w.id == id1).unwrap().z_order;
        let z2 = wm.windows.iter().find(|w| w.id != id1).unwrap().z_order;
        assert!(z1 > z2, "focused window should have highest z_order");
    }

    // ── close_window ─────────────────────────────────────────────────────────

    #[test]
    fn close_window_removes_it() {
        let mut wm = make_wm();
        let id = wm.add_window("s1".into(), "S1".into());
        wm.close_window(&id);
        assert!(wm.windows.is_empty());
        assert!(wm.focused_id.is_none());
    }

    #[test]
    fn close_focused_window_moves_focus_to_last_remaining() {
        let mut wm = make_wm();
        let id1 = wm.add_window("s1".into(), "S1".into());
        let id2 = wm.add_window("s2".into(), "S2".into());
        wm.close_window(&id2);
        assert_eq!(wm.windows.len(), 1);
        assert_eq!(wm.focused_id.as_deref(), Some(id1.as_str()));
    }

    #[test]
    fn close_non_focused_window_preserves_focus() {
        let mut wm = make_wm();
        let id1 = wm.add_window("s1".into(), "S1".into());
        let id2 = wm.add_window("s2".into(), "S2".into());
        wm.focus(&id2);
        wm.close_window(&id1);
        assert_eq!(wm.focused_id.as_deref(), Some(id2.as_str()));
    }

    // ── minimize / restore ───────────────────────────────────────────────────

    #[test]
    fn minimize_all_hides_every_window_and_clears_focus() {
        let mut wm = make_wm();
        wm.add_window("s1".into(), "S1".into());
        wm.add_window("s2".into(), "S2".into());
        wm.minimize_all();
        assert!(wm.windows.iter().all(|w| w.minimized));
        assert!(wm.focused_id.is_none());
    }

    #[test]
    fn restore_all_shows_every_window() {
        let mut wm = make_wm();
        wm.add_window("s1".into(), "S1".into());
        wm.minimize_all();
        wm.restore_all();
        assert!(wm.windows.iter().all(|w| !w.minimized));
    }

    // ── layouts ──────────────────────────────────────────────────────────────

    #[test]
    fn tile_sets_mode_and_assigns_equal_sizes() {
        let mut wm = make_wm();
        for i in 0..4 { wm.add_window(format!("s{i}"), format!("S{i}")); }
        wm.tile();
        assert_eq!(wm.layout_mode, LayoutMode::Tile);
        let size0 = wm.windows[0].size;
        assert!(wm.windows.iter().all(|w| w.size == size0),
            "all tiled windows should share the same size");
    }

    #[test]
    fn tile_four_windows_forms_2x2_grid() {
        let mut wm = make_wm();
        for i in 0..4 { wm.add_window(format!("s{i}"), format!("S{i}")); }
        wm.tile();
        let xs: Vec<i32> = wm.windows.iter().map(|w| w.pos.x.round() as i32).collect();
        let ys: Vec<i32> = wm.windows.iter().map(|w| w.pos.y.round() as i32).collect();
        assert_eq!(xs[0], xs[2], "col-0 windows share x");
        assert_eq!(xs[1], xs[3], "col-1 windows share x");
        assert_eq!(ys[0], ys[1], "row-0 windows share y");
        assert_eq!(ys[2], ys[3], "row-1 windows share y");
    }

    #[test]
    fn tile_skips_minimized_windows() {
        let mut wm = make_wm();
        for i in 0..3 { wm.add_window(format!("s{i}"), format!("S{i}")); }
        wm.windows[2].minimized = true;
        let pos_before = wm.windows[2].pos;
        wm.tile();
        assert_eq!(wm.windows[2].pos, pos_before, "minimised window must not be repositioned");
    }

    #[test]
    fn cascade_sets_mode_and_offsets_each_window_28px() {
        let mut wm = make_wm();
        for i in 0..3 { wm.add_window(format!("s{i}"), format!("S{i}")); }
        wm.cascade();
        assert_eq!(wm.layout_mode, LayoutMode::Cascade);
        for i in 1..wm.windows.len() {
            let dx = wm.windows[i].pos.x - wm.windows[i - 1].pos.x;
            let dy = wm.windows[i].pos.y - wm.windows[i - 1].pos.y;
            assert!((dx - 28.0).abs() < 0.1, "x offset wrong at slot {i}: {dx}");
            assert!((dy - 28.0).abs() < 0.1, "y offset wrong at slot {i}: {dy}");
        }
    }

    #[test]
    fn apply_layout_noop_when_all_windows_minimized() {
        let mut wm = make_wm();
        wm.add_window("s1".into(), "S1".into());
        wm.minimize_all();
        let pos_before = wm.windows[0].pos;
        wm.tile();
        assert_eq!(wm.windows[0].pos, pos_before);
    }

    // ── sorted_windows ───────────────────────────────────────────────────────

    #[test]
    fn sorted_windows_orders_by_ascending_z_order() {
        let mut wm = make_wm();
        let id1 = wm.add_window("s1".into(), "S1".into());
        wm.add_window("s2".into(), "S2".into());
        wm.add_window("s3".into(), "S3".into());
        wm.focus(&id1); // give id1 the highest z
        let sorted = wm.sorted_windows();
        let top_idx = *sorted.last().unwrap();
        assert_eq!(wm.windows[top_idx].id, id1, "last in sorted order must be the focused window");
    }

    // ── focused_session_id ───────────────────────────────────────────────────

    #[test]
    fn focused_session_id_returns_correct_session() {
        let mut wm = make_wm();
        wm.add_window("session-abc".into(), "Shell".into());
        assert_eq!(wm.focused_session_id(), Some("session-abc"));
    }

    #[test]
    fn focused_session_id_none_when_no_windows() {
        assert!(make_wm().focused_session_id().is_none());
    }
}
