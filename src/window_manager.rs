use egui::Pos2;

#[derive(Debug, Clone, PartialEq)]
pub enum LayoutMode {
    Free,
    Tile,
    Cascade,
}

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

pub struct WindowManager {
    pub windows: Vec<ShellWindow>,
    pub layout_mode: LayoutMode,
    pub focused_id: Option<String>,
    pub workspace_rect: egui::Rect,
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

    pub fn free(&mut self) {
        self.layout_mode = LayoutMode::Free;
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
                let h = (ws.height() - padding * (rows + 1) as f32) / rows as f32;

                for (slot, &wi) in visible.iter().enumerate() {
                    let col = slot % cols;
                    let row = slot / cols;
                    self.windows[wi].pos = Pos2::new(
                        ws.min.x + padding + col as f32 * (w + padding),
                        ws.min.y + padding + row as f32 * (h + padding),
                    );
                    self.windows[wi].size = egui::Vec2::new(w, h);
                }
            }
            LayoutMode::Cascade => {
                let base_w = ws.width() * 0.55;
                let base_h = ws.height() * 0.6;
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

    pub fn focused_session_id(&self) -> Option<&str> {
        self.focused_id.as_ref().and_then(|fid| {
            self.windows
                .iter()
                .find(|w| &w.id == fid)
                .map(|w| w.session_id.as_str())
        })
    }

    pub fn sorted_windows(&self) -> Vec<usize> {
        let mut indices: Vec<usize> = (0..self.windows.len()).collect();
        indices.sort_by_key(|&i| self.windows[i].z_order);
        indices
    }
}
