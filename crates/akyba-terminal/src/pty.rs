//! A shell running in a pseudo-terminal, with one thread reading its output, one writing input,
//! and one waiting for it to exit.

use std::io::{Read as _, Write as _};
use std::path::Path;
use std::sync::mpsc;
use std::thread;

use akyba_core::shell_env::shell_env;
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};

/// How many output chunks may wait for the UI. When it falls behind, the reader blocks, the pty's
/// buffer fills and the program pauses: flow control without bookkeeping.
const OUTPUT_BACKLOG: usize = 64;
const READ_CHUNK: usize = 64 * 1024;

pub enum PtyEvent {
    Output(Vec<u8>),
    Exited(Option<u32>),
}

pub struct Pty {
    master: Box<dyn MasterPty + Send>,
    input: mpsc::Sender<Vec<u8>>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    /// The shell's process id: everything the terminal runs is under it.
    shell: Option<u32>,
}

impl Pty {
    /// Starts the user's login shell in `cwd`. `extra_env` is added to the app's own environment.
    pub fn spawn(
        cwd: &Path,
        cols: u16,
        rows: u16,
        extra_env: &[(&str, &str)],
    ) -> anyhow::Result<(Self, async_channel::Receiver<PtyEvent>)> {
        let pair = native_pty_system().openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })?;
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_owned());
        let mut command = CommandBuilder::new(&shell);
        command.arg("-l");
        command.cwd(cwd);
        command.env_clear();
        for (key, value) in shell_env(std::env::vars(), extra_env) {
            command.env(key, value);
        }
        // Apps started from the Dock get no locale; without one, shells treat text as ASCII and
        // show typed characters like `ě` or emoji as `<0001fa90>`. Terminal.app sets it the same way.
        if std::env::var_os("LANG").is_none() && !extra_env.iter().any(|(key, _)| *key == "LANG") {
            command.env("LANG", "en_US.UTF-8");
        }
        let mut child = pair.slave.spawn_command(command)?;
        drop(pair.slave);
        let killer = child.clone_killer();
        let shell = child.process_id();

        let (events, receiver) = async_channel::bounded(OUTPUT_BACKLOG);
        let mut reader = pair.master.try_clone_reader()?;
        let output = events.clone();
        thread::Builder::new().name("pty-read".into()).spawn(move || {
            let mut buffer = vec![0; READ_CHUNK];
            while let Ok(read) = reader.read(&mut buffer) {
                if read == 0 || output.send_blocking(PtyEvent::Output(buffer[..read].to_vec())).is_err() {
                    break;
                }
            }
        })?;
        thread::Builder::new().name("pty-wait".into()).spawn(move || {
            let code = child.wait().ok().map(|status| status.exit_code());
            let _ = events.send_blocking(PtyEvent::Exited(code));
        })?;

        let mut writer = pair.master.take_writer()?;
        let (input, pending) = mpsc::channel::<Vec<u8>>();
        thread::Builder::new().name("pty-write".into()).spawn(move || {
            for bytes in pending {
                if writer.write_all(&bytes).and_then(|()| writer.flush()).is_err() {
                    break;
                }
            }
        })?;

        Ok((Self { master: pair.master, input, killer, shell }, receiver))
    }

    pub fn write(&self, bytes: impl Into<Vec<u8>>) {
        let _ = self.input.send(bytes.into());
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        let _ = self.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
    }

    /// The process group in the foreground of this terminal: the shell, or what it runs.
    pub fn foreground_pid(&self) -> Option<i32> {
        self.master.process_group_leader()
    }

    pub fn shell_pid(&self) -> Option<u32> {
        self.shell
    }

    pub fn kill(&mut self) {
        let _ = self.killer.kill();
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        self.kill();
    }
}
