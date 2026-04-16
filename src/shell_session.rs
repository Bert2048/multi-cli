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
    Custom(String),
}

impl ShellKind {
    /// Short display name shown in the toolbar and saved to state JSON.
    pub fn label(&self) -> &str {
        match self {
            ShellKind::PowerShell => "PowerShell",
            ShellKind::Cmd => "CMD",
            ShellKind::Bash => "Bash",
            ShellKind::Custom(s) => s.as_str(),
        }
    }

    /// Build the shell command.  For PowerShell the initial directory and the
    /// prompt-OSC hook are embedded directly in the `-Command` argument so they
    /// execute before the first prompt appears and produce no visible output.
    pub fn build_command(&self, initial_dir: Option<&str>) -> portable_pty::CommandBuilder {
        match self {
            ShellKind::PowerShell => {
                let mut cmd = portable_pty::CommandBuilder::new("powershell.exe");
                cmd.arg("-NoExit");
                cmd.arg("-Command");
                // Escape single-quotes in the path (PowerShell style: '' = literal ')
                let cd_part = initial_dir
                    .map(|d| format!("Set-Location '{}';", d.replace('\'', "''")))
                    .unwrap_or_default();
                // Everything runs at startup, before the first prompt — never visible.
                cmd.arg(format!(
                    "[Console]::OutputEncoding=[Text.Encoding]::UTF8;\
                     $OutputEncoding=[Text.Encoding]::UTF8;\
                     {cd_part}\
                     function global:prompt{{\
                         $p=$PWD.Path;\
                         [Console]::Write([char]27+']2;'+$p+[char]7);\
                         'PS '+$p+'> '\
                     }}",
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
            ShellKind::Custom(s) => portable_pty::CommandBuilder::new(s),
        }
    }
}

/// A live shell process connected to a PTY, with a shared terminal buffer.
///
/// Spawning creates three threads: the PTY host thread (which owns the
/// `portable_pty` pair), a writer thread (channel → PTY master), and a reader
/// thread (PTY master → [`TerminalBuffer`]).
pub struct ShellSession {
    pub id: String,
    pub name: String,
    pub kind: ShellKind,
    /// Shared terminal state; lock briefly only to read/write, never across frames.
    pub buffer: Arc<Mutex<TerminalBuffer>>,
    /// Send raw bytes to the shell's stdin via the PTY writer thread.
    pub input_tx: Sender<Vec<u8>>,
    /// Set to `false` by the reader thread when the shell process exits.
    pub alive: Arc<Mutex<bool>>,
}

impl ShellSession {
    pub fn new(
        id: String,
        name: String,
        kind: ShellKind,
        cols: u16,
        rows: u16,
        initial_dir: Option<String>,
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
            // PowerShell: everything is embedded in build_command(); no stdin needed.
            _ => {}
        }

        let buf_clone = buffer.clone();
        let alive_clone = alive.clone();
        let cmd = kind.build_command(initial_dir.as_deref());

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
            id,
            name,
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
        ShellKind::PowerShell.build_command(None);
        ShellKind::PowerShell.build_command(Some("C:\\Users"));
        ShellKind::Cmd.build_command(None);
        ShellKind::Cmd.build_command(Some("C:\\Temp"));
        ShellKind::Bash.build_command(None);
        ShellKind::Bash.build_command(Some("/home/user"));
        ShellKind::Custom("echo".into()).build_command(None);
    }

    #[test]
    fn powershell_command_with_dir_containing_single_quote() {
        // Should not panic when the path contains a single-quote
        ShellKind::PowerShell.build_command(Some("C:\\Users\\O'Brien"));
    }

    #[test]
    fn bash_command_with_dir_containing_single_quote() {
        ShellKind::Bash.build_command(Some("/home/o'brien"));
    }
}
