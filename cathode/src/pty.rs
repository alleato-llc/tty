use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::Write;
use tokio::sync::mpsc;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

pub struct PtySession {
    master: Box<dyn portable_pty::MasterPty + Send>,
    pub writer: Box<dyn Write + Send>,
    pub child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl PtySession {
    /// Spawn a shell in a PTY.
    /// Returns the session and an mpsc receiver that delivers raw PTY output bytes.
    pub fn spawn(
        shell: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(Self, mpsc::Receiver<Vec<u8>>), BoxError> {
        Self::spawn_in(shell, cols, rows, None)
    }

    /// Like [`spawn`](Self::spawn), but starts the shell in `cwd` when it's an existing
    /// directory (used for "new tab in the same directory").
    pub fn spawn_in(
        shell: &str,
        cols: u16,
        rows: u16,
        cwd: Option<&std::path::Path>,
    ) -> Result<(Self, mpsc::Receiver<Vec<u8>>), BoxError> {
        Self::spawn_in_env(shell, cols, rows, cwd, &[])
    }

    /// Like [`spawn_in`](Self::spawn_in), with extra environment variables set on the
    /// child (used to point a shell at tty's OSC 133 integration — see the host's
    /// `shell_integration`).
    pub fn spawn_in_env(
        shell: &str,
        cols: u16,
        rows: u16,
        cwd: Option<&std::path::Path>,
        env: &[(String, String)],
    ) -> Result<(Self, mpsc::Receiver<Vec<u8>>), BoxError> {
        let pty_system = native_pty_system();
        let pair = pty_system.openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let mut cmd = CommandBuilder::new(shell);
        cmd.env("TERM", "xterm-256color");
        for (k, v) in env {
            cmd.env(k, v);
        }
        if let Some(dir) = cwd {
            if dir.is_dir() {
                cmd.cwd(dir);
            }
        }

        let child = pair.slave.spawn_command(cmd)?;
        let master = pair.master;
        let writer = master.take_writer()?;
        let mut reader = master.try_clone_reader()?;

        let (tx, rx) = mpsc::channel::<Vec<u8>>(256);

        // Spawn a background thread (not tokio task) to avoid blocking the async runtime
        // on synchronous PTY reads.
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match std::io::Read::read(&mut reader, &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if tx.blocking_send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        Ok((
            Self {
                master,
                writer,
                child,
            },
            rx,
        ))
    }

    pub fn write_bytes(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.writer.write_all(data)
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), BoxError> {
        self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }
}
