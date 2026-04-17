# multi-cli

A Rust desktop terminal multiplexer — GUI version of tmux + VSCode Terminal.
Renders multiple independent shell sessions as floating/tiled/cascaded windows inside a single egui application.

## Build & Run

```bash
cargo build              # debug
cargo build --release    # optimized
cargo run                # launch the app
cargo clippy             # lint
cargo test               # unit tests
```

## Architecture

```
MultiCliApp (src/app.rs)
  ├── WindowManager (src/window_manager.rs)   — layout, focus, z-order
  └── HashMap<id, ShellSession> (src/shell_session.rs)
        └── Arc<Mutex<TerminalBuffer>> (src/terminal_buffer.rs)
```

### Module responsibilities

| File | Role |
|------|------|
| `src/main.rs` | eframe entry point, window size (1280×800) |
| `src/app.rs` | `MultiCliApp` — egui render loop, input dispatch, toolbar/sidebar/workspace UI |
| `src/window_manager.rs` | `WindowManager` + `ShellWindow` — Free/Tile/Cascade layouts, z-order, focus |
| `src/shell_session.rs` | `ShellSession` — spawns PTY pair, reader/writer threads via `portable-pty` |
| `src/terminal_buffer.rs` | `TerminalBuffer` — wraps `vt100::Parser` for ANSI parsing, CJK wide-char detection |

### Data flow

**Output:** Shell process → PTY reader thread → `TerminalBuffer::feed()` → `visible_lines()` → egui painter

**Input:** egui keyboard events → `ShellSession::write_input()` → PTY writer thread → shell process

### Shell kinds (`ShellKind`)

- `PowerShell` — injects UTF-8 encoding + OSC 2 `$PWD` prompt at startup
- `Cmd` — sends `chcp 65001\r` on first input to enable UTF-8
- `Bash` — sets `LANG`/`LC_ALL` to `en_US.UTF-8`, injects `PROMPT_COMMAND` for OSC 2 `$PWD`
- `Custom(String)` — arbitrary executable

### Current working directory tracking

Shells emit OSC 2 escape sequences (`\e]2;<path>\a`). `TerminalBuffer::screen_title()` reads this via `vt100::Screen::title()`. The status bar and session restore both use this value.

### Session persistence

State is saved to `%APPDATA%/multi-cli/state.json` (via `state_path()`):
- Auto-save every 60 seconds
- Manual save/load via toolbar buttons
- Saved on app exit (`on_exit`)
- Restored on startup (`load_state_or_default`)
- Persisted fields: window name, shell kind, position, size, minimized flag, last working dir

### Layout modes

- **Free** — user drags windows freely; no automatic repositioning
- **Tile** — grid layout, `cols = ceil(sqrt(n))`
- **Cascade** — overlapping windows offset by 28px per step

Switching to Tile or Cascade immediately calls `apply_layout()`. Free mode does not.

### Rendering details

- Cell size: `char_w = 7.4px`, `line_h = 14.0px`, font size 12px Monospace
- CJK wide chars occupy `char_w * 2.0` columns (detected via Unicode range table in `is_wide_char`)
- Cursor: semi-transparent ACCENT block + solid 2px left bar (focused window only)
- Alternate screen (`vt100::Screen::alternate_screen()`) disables scroll-follow; used by TUI apps like vim/htop
- Terminal content is clipped to `term_rect` via `painter.with_clip_rect()`

### UI layout constants

```rust
TITLEBAR_H = 28.0
SIDEBAR_W  = 160.0
TOOLBAR_H  = 40.0
STATUS_H   = 18.0   // CWD status bar at window bottom
```

### Key bindings

- `Ctrl+N` — new shell (uses currently selected shell kind)
- Titlebar drag — move window
- Bottom-right grip (`⤡`) — resize window (min 280×160)
- Red circle — close window + kill session
- Yellow circle — minimize window
- Sidebar double-click — rename session

### CJK font

At startup, `setup_cjk_font()` probes Windows system fonts in order:
`msyh.ttc` → `simsun.ttc` → `meiryo.ttc`. The first found is appended to both Monospace and Proportional font families.

## Dependencies

```toml
eframe      = "0.27"   # egui application framework
egui        = "0.27"   # immediate-mode GUI
portable-pty = "0.8"   # cross-platform PTY
vt100       = "0.15"   # VT100/ANSI terminal parser
crossbeam-channel = "0.5"
uuid        = "1"      # session/window IDs
serde + serde_json      # state serialization
```

## Key design notes

- `ShellSession` and `ShellWindow` use different IDs. `ShellWindow.session_id` links them.
- `WindowManager.windows` is the authoritative list; `MultiCliApp.sessions` is the PTY map.
- Both reader and writer run in dedicated `std::thread::spawn` threads — egui polls at ~30 fps (`request_repaint_after(33ms)`).
- PTY is always 120 cols × 40 rows regardless of window size (resize not yet wired up).
- `input_tx` channel is `bounded(256)` — backpressure if the shell is slow.
