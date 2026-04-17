# 🧭 Rust 多 Shell 桌面终端系统设计方案（egui + portable-pty）

---

# 1️⃣ 项目目标

构建一个桌面级多终端管理工具，类似以下产品组合：

* VSCode Terminal（GUI 终端）
* tmux（多 session + 分屏）
* Windows Terminal（多窗口）
* Warp（现代终端 UI）

---

## 🎯 核心能力

* 多 shell session（PowerShell / bash / cmd）
* 每个 shell 独立 PTY（portable-pty）
* 多子窗口管理（独立窗口，而不是单 pane）
* 支持窗口布局：

  * Free（自由拖动）
  * Tile（自动平铺）
  * Cascade（层叠）
* 一键最小化所有窗口
* 一键自动排列
* 手动拖拽布局
* GUI 操作（egui）

---

# 2️⃣ 总体架构

```
            ┌──────────────────────────┐
            │     egui UI Layer        │
            │  windows / layout / UX   │
            └────────────┬─────────────┘
                         │
            ┌────────────▼────────────┐
            │   Window Manager        │
            │ layout + focus + state  │
            └────────────┬────────────┘
                         │
    ┌────────────────────┼────────────────────┐
    ▼                    ▼                    ▼
```

┌──────────────┐   ┌──────────────┐   ┌──────────────┐
│ Shell Window │   │ Shell Window │   │ Shell Window │
│ PTY ps1     │   │ PTY bash     │   │ PTY ssh      │
└──────┬───────┘   └──────┬───────┘   └──────┬───────┘
│                  │                  │
└──────── portable-pty sessions ──────┘

---

# 3️⃣ 技术栈

| 模块       | 技术                       |
| -------- | ------------------------ |
| UI       | egui                     |
| Shell 执行 | portable-pty             |
| ANSI 解析  | vte                      |
| 并发       | std thread / tokio       |
| 状态管理     | Rust struct + Arc<Mutex> |

---

# 4️⃣ 核心模块设计

---

## 4.1 Shell Session（PTY层）

struct ShellSession {
id: String,
name: String,

```
pty_writer: Box<dyn std::io::Write>,
pty_reader: Box<dyn std::io::Read>,

buffer: String,
```

}

### 职责

* 管理 shell 生命周期
* 处理输入输出 IO
* 缓存终端输出 buffer

---

## 4.2 Shell Window（UI层）

struct ShellWindow {
id: String,
session_id: String,

```
title: String,

pos: egui::Pos2,
size: egui::Vec2,

minimized: bool,
focused: bool,

scroll: f32,
```

}

### 职责

* UI 窗口容器
* 绑定 session
* 支持拖拽 / resize / focus

---

## 4.3 Window Manager（核心控制器）

struct WindowManager {
windows: Vec<ShellWindow>,
sessions: Vec<ShellSession>,

```
layout_mode: LayoutMode,
focused_window: Option<String>,
```

}

---

## Layout Mode

enum LayoutMode {
Free,      // 自由拖动窗口
Tile,      // 自动平铺
Cascade,   // 层叠窗口
}

---

# 5️⃣ PTY 输入输出模型

---

## 输入流

Keyboard Input
↓
Focused Window
↓
ShellSession
↓
PTY write
↓
Shell Process

---

## 输出流

Shell Process
↓
PTY Reader Thread
↓
Session Buffer
↓
UI Render (egui)

---

# 6️⃣ UI 设计（egui）

---

## 6.1 主界面布局

┌────────────────────────────────────────────┐
│ Toolbar: [New] [Tile] [Cascade] [Minimize]│
├───────────────┬────────────────────────────┤
│ Sidebar       │ Workspace                  │
│ Sessions      │ (Multiple Windows)        │
│               │                           │
│ ps1           │ ┌────────┬──────────────┐ │
│ bash          │ │ bash   │ powershell   │ │
│ ssh-prod      │ ├────────┼──────────────┤ │
│               │ │ logs   │ docker       │ │
└───────────────┴────────────────────────────┘

---

## 6.2 Sidebar（Session Manager）

## [ + ] New Shell

▶ ps1
bash
ssh-prod

---

## 6.3 Toolbar（全局控制）

* New Shell
* Tile All
* Cascade
* Minimize All
* Reset Layout

---

## 6.4 Shell Window UI

┌──────────────────────────────┐
│ bash session        [x][_]   │
├──────────────────────────────┤
│ terminal output              │
│                              │
│                              │
├──────────────────────────────┤
│ > input                      │
└──────────────────────────────┘

---

## 6.5 交互行为

* 点击窗口 → focus
* 拖拽标题栏 → 移动窗口
* resize → 调整大小
* minimize → 隐藏窗口内容

---

# 7️⃣ 布局系统设计

---

## 7.1 Free Layout（自由布局）

用户自由拖动窗口，类似桌面应用

---

## 7.2 Tile Layout（自动平铺）

┌──────────┬──────────┐
│ shell 1  │ shell 2  │
├──────────┼──────────┤
│ shell 3  │ shell 3  │
└──────────┴──────────┘

算法：

cols = sqrt(n)
rows = ceil(n / cols)

---

## 7.3 Cascade Layout（层叠）

┌──────────────┐
│ shell 1      │
│ ┌──────────┐ │
│ │ shell 2  │ │
│ │ ┌──────┐ │ │
│ │ │shell3│ │ │
│ │ └──────┘ │ │
└──────────────┘

---

# 8️⃣ 输入系统设计

Keyboard Event
↓
Focused Window
↓
ShellSession
↓
PTY write

---

## egui 输入处理

* 全局快捷键（Ctrl+N / Ctrl+T）
* window input capture
* session switch

---

# 9️⃣ 输出系统设计

struct TerminalBuffer {
lines: Vec<String>,
scroll: usize,
}

---

## ANSI 解析

使用 vte crate

---

# 🔟 窗口管理功能

---

## 一键最小化

for window in windows.iter_mut() {
window.minimized = true;
}

---

## 一键排列（Tile）

layout_mode = LayoutMode::Tile;
recalculate_positions();

---

## 手动拖拽

egui drag delta → window.pos += delta

---

# 1️⃣1️⃣ 扩展能力

* SSH session
* Docker exec session
* Remote PTY server
* Web terminal
* Plugin system

---

# 1️⃣2️⃣ 项目本质总结

该系统本质是：

Rust 实现的 GUI Window Manager + Terminal Multiplexer

融合：

* tmux（多 session）
* VSCode Terminal（GUI）
* Desktop Window Manager（布局）

---

# 🚀 一句话总结

使用 portable-pty 管理 shell session，用 egui 构建多窗口 UI，通过 WindowManager 实现 Free/Tile/Cascade 布局，打造 GUI 版 tmux + VSCode Terminal 的混合桌面终端系统
