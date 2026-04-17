use crossbeam_channel::{bounded, Receiver, Sender};
use portable_pty::{native_pty_system, Child, PtySize};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::terminal_buffer::TerminalBuffer;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ShellKind {
    PowerShell,
    Cmd,
    Bash,
    /// Opens a PowerShell session and immediately launches the `claude` CLI.
    Claude,
    Custom(String),
}

impl ShellKind {
    /// Short display name shown in the toolbar and saved to state JSON.
    pub fn label(&self) -> &str {
        match self {
            ShellKind::PowerShell => "PowerShell",
            ShellKind::Cmd => "CMD",
            ShellKind::Bash => "Bash",
            ShellKind::Claude => "Claude",
            ShellKind::Custom(s) => s.as_str(),
        }
    }

    /// Build the shell command.
    ///
    /// `startup_cmd` is embedded silently into the PowerShell `-Command` string.
    /// For other shell kinds it is ignored here and handled via PTY input in
    /// [`ShellSession::new`].
    pub fn build_command(&self, initial_dir: Option<&str>, startup_cmd: Option<&str>) -> portable_pty::CommandBuilder {
        match self {
            ShellKind::PowerShell => {
                let mut cmd = portable_pty::CommandBuilder::new("powershell.exe");
                cmd.arg("-NoExit");
                cmd.arg("-Command");
                let cd_part = initial_dir
                    .map(|d| format!("Set-Location '{}';", d.replace('\'', "''")))
                    .unwrap_or_default();
                let startup_part = startup_cmd
                    .filter(|s| !s.is_empty())
                    .map(|s| format!("; {}", s))
                    .unwrap_or_default();
                cmd.arg(format!(
                    "[Console]::OutputEncoding=[Text.Encoding]::UTF8;\
                     $OutputEncoding=[Text.Encoding]::UTF8;\
                     {cd_part}\
                     function global:prompt{{\
                         $p=$PWD.Path;\
                         [Console]::Write([char]27+']2;'+$p+[char]7);\
                         'PS '+$p+'> '\
                     }}{startup_part}",
                ));
                cmd
            }
            ShellKind::Claude => {
                let mut cmd = portable_pty::CommandBuilder::new("powershell.exe");
                cmd.arg("-NoExit");
                cmd.arg("-Command");
                let cd_part = initial_dir
                    .map(|d| format!("Set-Location '{}';", d.replace('\'', "''")))
                    .unwrap_or_default();
                let claude_cmd = startup_cmd.unwrap_or("claude");
                cmd.arg(format!(
                    "[Console]::OutputEncoding=[Text.Encoding]::UTF8;\
                     $OutputEncoding=[Text.Encoding]::UTF8;\
                     {cd_part}\
                     function global:prompt{{\
                         $p=$PWD.Path;\
                         [Console]::Write([char]27+']2;'+$p+[char]7);\
                         'PS '+$p+'> '\
                     }};\
                     [Console]::Write([char]27+']2;'+$PWD.Path+[char]7);\
                     {claude_cmd}",
                ));
                cmd
            }
            ShellKind::Cmd => portable_pty::CommandBuilder::new("cmd.exe"),
            ShellKind::Bash => {
                let mut cmd = portable_pty::CommandBuilder::new("bash");
                cmd.env("LANG", "en_US.UTF-8");
                cmd.env("LC_ALL", "en_US.UTF-8");
                cmd
            }
            ShellKind::Custom(s) => {
                let mut cmd = portable_pty::CommandBuilder::new(s);
                if let Some(dir) = initial_dir {
                    cmd.cwd(dir);
                }
                cmd
            }
        }
    }
}

/// A live shell process connected to a PTY, with a shared terminal buffer.
///
/// Spawning creates three threads: the PTY host thread (which owns the
/// `portable_pty` pair), a writer thread (channel → PTY master), and a reader
/// thread (PTY master → [`TerminalBuffer`]).
pub struct ShellSession {
    pub kind: ShellKind,
    /// Shared terminal state; lock briefly only to read/write, never across frames.
    pub buffer: Arc<Mutex<TerminalBuffer>>,
    /// Send raw bytes to the shell's stdin via the PTY writer thread.
    pub input_tx: Sender<Vec<u8>>,
    /// Set to `false` by the reader thread when the shell process exits.
    #[allow(dead_code)]
    pub alive: Arc<Mutex<bool>>,
}

impl ShellSession {
    pub fn new(
        kind: ShellKind,
        cols: u16,
        rows: u16,
        initial_dir: Option<String>,
        startup_cmd: Option<String>,
    ) -> Self {
        let buffer = Arc::new(Mutex::new(TerminalBuffer::new(cols as usize, rows as usize)));
        let (input_tx, input_rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = bounded(256);
        let alive = Arc::new(Mutex::new(true));

        match &kind {
            ShellKind::Cmd => {
                // @echo off  — suppress command echo for the init sequence
                // @chcp 65001 >nul  — UTF-8, silent
                // @cd /d "path"  — restore dir, silent
                // @cls  — wipe any startup banner
                // @echo on  — restore interactive echo
                let mut init = String::from("@echo off\r");
                init.push_str("@chcp 65001 >nul 2>&1\r");
                if let Some(dir) = &initial_dir {
                    // Escape embedded double-quotes
                    init.push_str(&format!("@cd /d \"{}\"\r", dir.replace('"', "\"\"")));
                }
                init.push_str("@cls\r");
                init.push_str("@echo on\r");
                let _ = input_tx.send(init.into_bytes());
            }
            ShellKind::Bash => {
                // stty -echo silences the PTY echo so none of these lines are shown.
                // stty echo restores it before the user gets the prompt.
                let cd_part = initial_dir
                    .as_deref()
                    .map(|d| format!("cd '{}' 2>/dev/null; ", d.replace('\'', "'\\''")))
                    .unwrap_or_default();
                let init = format!(
                    "stty -echo; \
                     {cd_part}\
                     PROMPT_COMMAND='printf \"\\033]2;%s\\007\" \"$PWD\"'; \
                     stty echo\r"
                );
                let _ = input_tx.send(init.into_bytes());
            }
            // PowerShell/Claude: everything is embedded in build_command(); no stdin needed.
            _ => {}
        }

        // For non-PS shells, send startup_cmd via PTY input (will echo in terminal).
        // PS handles startup_cmd silently via -Command embedding in build_command().
        if let Some(ref scmd) = startup_cmd {
            if !scmd.is_empty() && !matches!(&kind, ShellKind::PowerShell | ShellKind::Claude) {
                let _ = input_tx.send(format!("{}\r", scmd).into_bytes());
            }
        }

        let buf_clone = buffer.clone();
        let alive_clone = alive.clone();
        let cmd = kind.build_command(initial_dir.as_deref(), startup_cmd.as_deref());

        thread::spawn(move || {
            let pty_system = native_pty_system();
            let pair = match pty_system.openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            }) {
                Ok(p) => p,
                Err(e) => {
                    let mut buf = buf_clone.lock().unwrap();
                    let msg = format!("[Error opening PTY: {}]\r\n", e);
                    buf.feed(msg.as_bytes());
                    *alive_clone.lock().unwrap() = false;
                    return;
                }
            };

            let _child: Box<dyn Child + Send> = match pair.slave.spawn_command(cmd) {
                Ok(c) => c,
                Err(e) => {
                    let mut buf = buf_clone.lock().unwrap();
                    let msg = format!("[Error spawning shell: {}]\r\n", e);
                    buf.feed(msg.as_bytes());
                    *alive_clone.lock().unwrap() = false;
                    return;
                }
            };

            // Writer thread: forwards queued input bytes to the PTY master
            let mut writer = pair.master.take_writer().unwrap();
            let alive_w = alive_clone.clone();
            thread::spawn(move || {
                while *alive_w.lock().unwrap() {
                    match input_rx.recv() {
                        Ok(data) => {
                            let _ = writer.write_all(&data);
                            let _ = writer.flush();
                        }
                        Err(_) => break,
                    }
                }
            });

            // Reader thread: feeds PTY output into the terminal buffer
            let mut reader = pair.master.try_clone_reader().unwrap();
            let mut read_buf = [0u8; 4096];
            loop {
                match reader.read(&mut read_buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let mut buf = buf_clone.lock().unwrap();
                        buf.feed(&read_buf[..n]);
                    }
                }
            }

            *alive_clone.lock().unwrap() = false;
        });

        Self {
            kind,
            buffer,
            input_tx,
            alive,
        }
    }

    /// Queue raw bytes for delivery to the shell's stdin via the PTY writer thread.
    pub fn write_input(&self, data: &[u8]) {
        let _ = self.input_tx.send(data.to_vec());
    }

    /// Returns `true` while the shell process is still running.
    #[allow(dead_code)]
    pub fn is_alive(&self) -> bool {
        *self.alive.lock().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── ShellKind::label ─────────────────────────────────────────────────────

    #[test]
    fn label_powershell() { assert_eq!(ShellKind::PowerShell.label(), "PowerShell"); }

    #[test]
    fn label_cmd() { assert_eq!(ShellKind::Cmd.label(), "CMD"); }

    #[test]
    fn label_bash() { assert_eq!(ShellKind::Bash.label(), "Bash"); }

    #[test]
    fn label_custom_returns_inner_string() {
        assert_eq!(ShellKind::Custom("fish".into()).label(), "fish");
        assert_eq!(ShellKind::Custom("zsh".into()).label(), "zsh");
    }

    // ── ShellKind equality / clone ────────────────────────────────────────────

    #[test]
    fn shell_kind_equality() {
        assert_eq!(ShellKind::PowerShell, ShellKind::PowerShell);
        assert_ne!(ShellKind::PowerShell, ShellKind::Cmd);
        assert_eq!(ShellKind::Custom("a".into()), ShellKind::Custom("a".into()));
        assert_ne!(ShellKind::Custom("a".into()), ShellKind::Custom("b".into()));
    }

    #[test]
    fn shell_kind_clone_equals_original() {
        let k = ShellKind::Custom("nu".into());
        assert_eq!(k.clone(), k);
    }

    // ── ShellKind::build_command (smoke — does not spawn a process) ───────────

    #[test]
    fn build_command_does_not_panic_for_any_variant() {
        ShellKind::PowerShell.build_command(None, None);
        ShellKind::PowerShell.build_command(Some("C:\\Users"), Some("claude"));
        ShellKind::Cmd.build_command(None, None);
        ShellKind::Cmd.build_command(Some("C:\\Temp"), None);
        ShellKind::Bash.build_command(None, None);
        ShellKind::Bash.build_command(Some("/home/user"), None);
        ShellKind::Claude.build_command(None, None);
        ShellKind::Claude.build_command(Some("C:\\Users"), None);
        ShellKind::Custom("echo".into()).build_command(None, None);
    }

    #[test]
    fn powershell_command_with_dir_containing_single_quote() {
        // Should not panic when the path contains a single-quote
        ShellKind::PowerShell.build_command(Some("C:\\Users\\O'Brien"), None);
    }

    #[test]
    fn bash_command_with_dir_containing_single_quote() {
        ShellKind::Bash.build_command(Some("/home/o'brien"), None);
    }
}
