use crossbeam_channel::{bounded, Receiver, Sender};
use portable_pty::{native_pty_system, Child, PtySize};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::terminal_buffer::TerminalBuffer;

#[derive(Debug, Clone, PartialEq)]
pub enum ShellKind {
    PowerShell,
    Cmd,
    Bash,
    Custom(String),
}

impl ShellKind {
    pub fn label(&self) -> &str {
        match self {
            ShellKind::PowerShell => "PowerShell",
            ShellKind::Cmd => "CMD",
            ShellKind::Bash => "Bash",
            ShellKind::Custom(s) => s.as_str(),
        }
    }

    /// Build a CommandBuilder with UTF-8 encoding configured for the shell.
    pub fn build_command(&self) -> portable_pty::CommandBuilder {
        match self {
            ShellKind::PowerShell => {
                // PowerShell: set console output encoding to UTF-8 on startup
                let mut cmd = portable_pty::CommandBuilder::new("powershell.exe");
                cmd.arg("-NoExit");
                cmd.arg("-Command");
                cmd.arg("[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; $OutputEncoding = [System.Text.Encoding]::UTF8");
                cmd
            }
            ShellKind::Cmd => {
                // CMD: /U flag outputs Unicode (UTF-16LE), we'll send chcp 65001 via input instead
                portable_pty::CommandBuilder::new("cmd.exe")
            }
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

pub struct ShellSession {
    pub id: String,
    pub name: String,
    pub kind: ShellKind,
    pub buffer: Arc<Mutex<TerminalBuffer>>,
    pub input_tx: Sender<Vec<u8>>,
    pub alive: Arc<Mutex<bool>>,
}

impl ShellSession {
    pub fn new(id: String, name: String, kind: ShellKind, cols: u16, rows: u16) -> Self {
        let buffer = Arc::new(Mutex::new(TerminalBuffer::new(cols as usize, rows as usize)));
        let (input_tx, input_rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = bounded(256);
        let alive = Arc::new(Mutex::new(true));

        // For CMD: pre-queue chcp 65001 so it runs immediately after shell starts
        if kind == ShellKind::Cmd {
            let _ = input_tx.send(b"chcp 65001\r".to_vec());
        }

        let buf_clone = buffer.clone();
        let alive_clone = alive.clone();
        let cmd = kind.build_command();

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

            // Writer thread
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

            // Reader thread
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

    pub fn write_input(&self, data: &[u8]) {
        let _ = self.input_tx.send(data.to_vec());
    }

    pub fn is_alive(&self) -> bool {
        *self.alive.lock().unwrap()
    }
}
