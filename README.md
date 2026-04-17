# multi-cli

> GUI 版 tmux + VSCode Terminal —— 用 Rust 和 egui 构建的桌面多终端管理器

在单个原生窗口内运行多个独立 Shell 会话，支持自由拖拽、平铺、层叠三种布局模式，深度集成 Claude CLI 多用户管理。

---

## 功能特性

### 多窗口终端
- 每个 Shell 会话独立运行在真实 PTY 中
- 窗口可自由拖拽、缩放、最小化、关闭
- 三种布局模式：**Free**（自由）/ **Tile**（平铺）/ **Cascade**（层叠）
- 一键最小化全部 / 一键还原全部

### Shell 类型
| Shell | 说明 |
|-------|------|
| **Claude** | 启动 Claude CLI，支持多用户、权限跳过、Telegram 插件 |
| **PowerShell** | 注入 UTF-8 编码 + OSC 2 路径追踪 |
| **CMD** | 启动时发送 `chcp 65001` 启用 UTF-8 |
| **Bash** | 设置 `LANG`/`LC_ALL` + `PROMPT_COMMAND` 路径追踪 |
| **Custom** | 任意可执行文件，支持配置初始目录和启动命令 |

### Claude CLI 深度集成
- **多用户切换**：工具栏下拉选择用户，每个用户配置独立 HOME 目录，启动时自动注入 `HOME` / `USERPROFILE` / `CLAUDE_CONFIG_DIR` 环境变量
- **状态栏实时显示**：当前目录 · 当前用户 · 5小时 Token 用量 · 周用量
- **快速切换工作目录**：双击状态栏弹出目录变更对话框，自动执行 `/exit` + 重启 Claude
- **启动参数**：可勾选 `--dangerously-skip-permissions` 和 Telegram 插件

### 终端渲染
- 完整 VT100 / ANSI 转义序列解析（`vt100` crate）
- CJK 宽字符支持（自动探测 `msyh.ttc` / `simsun.ttc` / `meiryo.ttc`）
- 交替屏支持（vim、htop 等 TUI 应用）
- 文本选择与右键复制

### 会话持久化
- 自动每 60 秒保存状态至 `%APPDATA%\multi-cli\state.json`
- 退出时保存，启动时还原（窗口位置、大小、Shell 类型）
- Claude / Custom Shell 还原时回到配置的初始目录

---

## 截图


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

## 安装与构建

**环境要求：** Rust 1.75+，Windows 10/11（使用 ConPTY）

```bash
git clone https://github.com/yourname/multi-cli
cd multi-cli

cargo build --release          # 编译
cargo run                      # 直接运行
```

release 产物位于 `target/release/multi-cli.exe`。

---

## 快捷键

| 操作 | 快捷键 / 鼠标 |
|------|--------------|
| 新建 Shell | `Ctrl+N` 或工具栏 `+ NEW` |
| 移动窗口 | 拖拽标题栏 |
| 调整窗口大小 | 拖拽右下角 `⤡` 手柄 |
| 最小化窗口 | 标题栏黄色圆点 |
| 关闭窗口 | 标题栏红色圆点 |
| 重命名会话 | 双击侧边栏条目 |
| 切换工作目录 | 双击 Claude/Custom 状态栏 |
| 复制文本 | 选中文本后右键菜单 |

---

## 设置说明

点击工具栏右侧 `⚙ SETTINGS` 打开设置面板。

### APPEARANCE
- **Font Size** — 终端字号（8–24px）
- **Line Spacing** — 行高比例

### TERMINAL
- **Default Shell** — Ctrl+N 和 NEW 按钮默认打开的 Shell 类型
- **PTY Columns / Rows** — PTY 尺寸（新会话生效）

### CLAUDE
- **Directory** — Claude 会话默认工作目录
- **Skip Permissions** — 启动时追加 `--dangerously-skip-permissions`
- **Telegram** — 启动时追加 `--channels plugin:telegram@claude-plugins-official`

#### USERS — Claude 多用户管理
| 字段 | 说明 |
|------|------|
| **Default** | 系统默认用户，路径 `~/.claude`，不注入环境变量 |
| **Name** | 工具栏下拉显示名称 |
| **Home Dir** | 该用户的 HOME 路径（如 `D:\home\alice`） |

当配置 2 个及以上用户时，工具栏显示用户选择器。切换用户后，新建 Claude 窗口将注入：

```powershell
$env:HOME          = "D:\home\alice"
$env:USERPROFILE   = "D:\home\alice"
$env:CLAUDE_CONFIG_DIR = "D:\home\alice\.claude"
```

### CUSTOM SHELLS
自定义可执行文件，支持设置：显示名称、命令路径、初始目录、启动命令。

### LAYOUT
- **Sidebar Width** — 侧边栏宽度

---

## 状态栏说明

Claude 会话窗口底部状态栏显示四段信息：

```
G:\Projects\multi-cli    Default │ 5h: 73% (1h20m) │ wk: 45% (3d6h)
└── 当前工作目录 ─────┘   └─用户┘   └── 5h配额 ──┘   └── 周配额 ──┘
```

- Token 数据每 **10 秒**更新一次
- 如使用自定义用户，优先从其 `{home}/.claude/` 目录读取用量文件，无文件则解析终端输出
- 时间显示为剩余重置倒计时

---

## 项目结构

```
src/
├── main.rs             # eframe 入口，窗口尺寸 1280×800
├── app.rs              # MultiCliApp — 渲染循环、输入分发、设置、工具栏、状态栏
├── window_manager.rs   # WindowManager + ShellWindow — 布局、焦点、Z 轴
├── shell_session.rs    # ShellSession — PTY 生命周期、读写线程
└── terminal_buffer.rs  # TerminalBuffer — vt100 解析、CJK 宽字符检测
```

### 数据流

```
输出：Shell → PTY 读线程 → TerminalBuffer::feed() → visible_lines() → egui painter
输入：egui 键盘事件 → ShellSession::write_input() → PTY 写线程 → Shell
```

---

## 依赖

| Crate | 版本 | 用途 |
|-------|------|------|
| `eframe` | 0.27 | egui 应用框架 |
| `egui` | 0.27 | 立即模式 GUI |
| `portable-pty` | 0.8 | 跨平台 PTY（Windows ConPTY） |
| `vt100` | 0.15 | VT100/ANSI 终端解析 |
| `crossbeam-channel` | 0.5 | 有界通道（PTY 输入背压） |
| `uuid` | 1 | 会话/窗口唯一 ID |
| `serde` + `serde_json` | 1 | 状态序列化 |
| `arboard` | 3 | 剪贴板访问 |

---

## License

MIT
