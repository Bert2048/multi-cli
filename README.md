# multi-cli

> A GUI version of tmux + VSCode Terminal — a desktop multi-terminal manager built with Rust and egui

Run multiple independent shell sessions inside a single native window with three layout modes (free/tile/cascade) and deep Claude CLI multi-user integration.

[中文文档](README-cn.md)

---

## Features

### Multi-Window Terminal
- Each shell session runs independently in a real PTY
- Windows can be freely dragged, resized, minimized, and closed
- Three layout modes: **Free** / **Tile** / **Cascade**
- One-click minimize all / restore all

### Shell Types
| Shell | Description |
|-------|-------------|
| **Claude** | Launches the Claude CLI with multi-user support, permission skip, and Telegram plugin |
| **PowerShell** | Injects UTF-8 encoding + OSC 2 path tracking |
| **CMD** | Sends `chcp 65001` on startup to enable UTF-8 |
| **Bash** | Sets `LANG`/`LC_ALL` + `PROMPT_COMMAND` for path tracking |
| **Custom** | Any executable, with configurable initial directory and startup command |

### Deep Claude CLI Integration
- **Multi-user switching**: Select a user from the toolbar dropdown. Each user has its own HOME directory, and `HOME` / `USERPROFILE` / `CLAUDE_CONFIG_DIR` environment variables are injected automatically on launch
- **Live status bar**: Current directory · Current user · 5-hour token usage · Weekly usage
- **Quick working directory switch**: Double-click the status bar to open a directory change dialog. Automatically runs `/exit` and restarts Claude
- **Launch options**: Toggle `--dangerously-skip-permissions` and the Telegram plugin

### Terminal Rendering
- Full VT100 / ANSI escape sequence parsing (via the `vt100` crate)
- CJK wide character support (auto-detects `msyh.ttc` / `simsun.ttc` / `meiryo.ttc`)
- Alternate screen support (for TUI apps like vim, htop)
- Text selection and right-click copy

### Session Persistence
- Auto-saves state to `%APPDATA%\multi-cli\state.json` every 60 seconds
- Saves on exit, restores on startup (window position, size, shell type)
- Claude / Custom shells restore to their configured initial directory

---

## Screenshot

```
┌─────────────────────────────────────────────────────────────────┐
│ ⬡ MULTI-CLI  [Claude ▾] [👤 Default ▾] [+ NEW]  [TILE][CASCADE] │
├──────────────┬──────────────────────────────────────────────────┤
│ SESSIONS     │                                                  │
│              │  ┌─────────────────────┐ ┌────────────────────┐ │
│ ▶ Claude 1   │  │ Claude 1        ● ─ │ │ PowerShell 1   ● ─ │ │
│   PowerShell │  │                     │ │                    │ │
│   Claude 2   │  │  > claude chat      │ │  PS C:\> ls        │ │
│              │  │                     │ │                    │ │
│ Layout: TILE │  │ G:\Projects │Default│ │ C:\Users\me        │ │
└──────────────┴──────────────────────────────────────────────────┘
```

---

## Installation & Build

**Requirements:** Rust 1.75+, Windows 10/11 (uses ConPTY)

```bash
git clone https://github.com/yourname/multi-cli
cd multi-cli

cargo build --release          # build
cargo run                      # run directly
```

The release binary is at `target/release/multi-cli.exe`.

---

## Keyboard Shortcuts

| Action | Shortcut / Mouse |
|--------|------------------|
| New shell | `Ctrl+N` or toolbar `+ NEW` |
| Move window | Drag title bar |
| Resize window | Drag bottom-right `⤡` handle |
| Minimize window | Yellow dot in title bar |
| Close window | Red dot in title bar |
| Rename session | Double-click sidebar entry |
| Switch working directory | Double-click Claude/Custom status bar |
| Copy text | Right-click menu after selection |

---

## Settings

Click `⚙ SETTINGS` on the right side of the toolbar to open the settings panel.

### APPEARANCE
- **Font Size** — Terminal font size (8–24px)
- **Line Spacing** — Line height ratio

### TERMINAL
- **Default Shell** — Shell type opened by `Ctrl+N` and the NEW button
- **PTY Columns / Rows** — PTY dimensions (applies to new sessions)

### CLAUDE
- **Directory** — Default working directory for Claude sessions
- **Skip Permissions** — Appends `--dangerously-skip-permissions` on launch
- **Telegram** — Appends `--channels plugin:telegram@claude-plugins-official` on launch

#### USERS — Claude Multi-User Management
| Field | Description |
|-------|-------------|
| **Default** | System default user, path `~/.claude`, no injected environment variables |
| **Name** | Display name shown in the toolbar dropdown |
| **Home Dir** | HOME path for this user (e.g. `D:\home\alice`) |

When two or more users are configured, the toolbar shows a user selector. After switching users, new Claude windows will inject:

```powershell
$env:HOME              = "D:\home\alice"
$env:USERPROFILE       = "D:\home\alice"
$env:CLAUDE_CONFIG_DIR = "D:\home\alice\.claude"
```

### CUSTOM SHELLS
Custom executables with configurable display name, command path, initial directory, and startup command.

### LAYOUT
- **Sidebar Width** — Sidebar width

---

## Status Bar

The Claude session window's bottom status bar shows four segments:

```
G:\Projects\multi-cli    Default │ 5h: 73% (1h20m) │ wk: 45% (3d6h)
└── working directory ┘   └user┘   └── 5h quota ─┘   └── weekly quota ┘
```

- Token data updates every **10 seconds**
- For custom users, usage is read from `{home}/.claude/` first; if unavailable, the terminal output is parsed
- Time values show the countdown until the next reset

---

## Project Structure

```
src/
├── main.rs             # eframe entry, 1280×800 window
├── app.rs              # MultiCliApp — render loop, input dispatch, settings, toolbar, status bar
├── window_manager.rs   # WindowManager + ShellWindow — layout, focus, z-order
├── shell_session.rs    # ShellSession — PTY lifecycle, reader/writer threads
└── terminal_buffer.rs  # TerminalBuffer — vt100 parsing, CJK wide character detection
```

### Data Flow

```
Output: Shell → PTY reader thread → TerminalBuffer::feed() → visible_lines() → egui painter
Input:  egui keyboard events → ShellSession::write_input() → PTY writer thread → Shell
```

---

## Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `eframe` | 0.27 | egui application framework |
| `egui` | 0.27 | Immediate-mode GUI |
| `portable-pty` | 0.8 | Cross-platform PTY (Windows ConPTY) |
| `vt100` | 0.15 | VT100/ANSI terminal parsing |
| `crossbeam-channel` | 0.5 | Bounded channel (PTY input backpressure) |
| `uuid` | 1 | Session/window unique IDs |
| `serde` + `serde_json` | 1 | State serialization |
| `arboard` | 3 | Clipboard access |

---

## License

MIT
