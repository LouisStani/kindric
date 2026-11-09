//! One session shown in a pseudo-terminal. The page draws it with xterm.js; this side only
//! moves bytes and sizes.

use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};

use kindric_core::attach::AttachCommand;
use portable_pty::{Child, ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};

/// The pty side kept by the pane; None once the program has ended and the pty is closed.
type SharedMaster = Arc<Mutex<Option<Box<dyn MasterPty + Send>>>>;

pub struct Pane {
    master: SharedMaster,
    writer: Box<dyn Write + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    waiter: Option<JoinHandle<()>>,
}

fn pty_error(error: impl ToString) -> io::Error {
    io::Error::other(error.to_string())
}

fn size(columns: u16, rows: u16) -> PtySize {
    PtySize { rows, cols: columns, pixel_width: 0, pixel_height: 0 }
}

/// Waits for the program, then closes the pty. On Windows the output of the pseudo-console
/// only ends once it is closed; the reader then sees the end after the last bytes.
fn close_when_done(mut child: Box<dyn Child + Send + Sync>, master: SharedMaster) {
    let _ = child.wait();
    master.lock().unwrap_or_else(PoisonError::into_inner).take();
}

/// Hands every chunk to `on_output`, then calls `on_exit` once the program has gone.
fn forward(mut reader: Box<dyn Read + Send>, mut on_output: impl FnMut(&[u8]), on_exit: impl FnOnce()) {
    let mut buffer = [0u8; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => on_output(&buffer[..read]),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {},
            // Linux reports the end of the program as EIO on the pty.
            Err(_) => break,
        }
    }
    on_exit();
}

impl Pane {
    pub fn open(
        command: &AttachCommand,
        columns: u16,
        rows: u16,
        on_output: impl FnMut(&[u8]) + Send + 'static,
        on_exit: impl FnOnce() + Send + 'static,
    ) -> io::Result<Self> {
        let pair = native_pty_system().openpty(size(columns, rows)).map_err(pty_error)?;
        let mut builder = CommandBuilder::new(&command.program);
        builder.args(&command.args);
        for (name, value) in &command.env {
            builder.env(name, value);
        }
        for name in &command.env_remove {
            builder.env_remove(name);
        }
        if let Some(directory) = &command.directory {
            builder.cwd(directory);
        }
        let child = pair.slave.spawn_command(builder).map_err(pty_error)?;
        drop(pair.slave);
        let reader = pair.master.try_clone_reader().map_err(pty_error)?;
        let writer = pair.master.take_writer().map_err(pty_error)?;
        let killer = child.clone_killer();
        let master: SharedMaster = Arc::new(Mutex::new(Some(pair.master)));
        let waiter = {
            let master = Arc::clone(&master);
            thread::spawn(move || close_when_done(child, master))
        };
        thread::spawn(move || forward(reader, on_output, on_exit));
        Ok(Self { master, writer, killer, waiter: Some(waiter) })
    }

    pub fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()
    }

    pub fn resize(&self, columns: u16, rows: u16) -> io::Result<()> {
        let master = self.master.lock().unwrap_or_else(PoisonError::into_inner);
        let master = master.as_ref().ok_or_else(|| io::Error::other("the session has ended"))?;
        master.resize(size(columns, rows)).map_err(pty_error)
    }
}

impl Drop for Pane {
    /// Ends the attaching process only: the background session keeps running in Claude Code.
    /// Returns once the program is gone and the pty closed.
    fn drop(&mut self) {
        let _ = self.killer.kill();
        if let Some(waiter) = self.waiter.take() {
            let _ = waiter.join();
        }
    }
}

/// Detaches in the background: the attaching process may take a while to end, and the
/// window must not wait for it.
pub fn detach(pane: Pane) -> JoinHandle<()> {
    thread::spawn(move || drop(pane))
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{self, Receiver};
    use std::time::{Duration, Instant};

    use super::*;

    const TIMEOUT: Duration = Duration::from_secs(5);

    enum Seen {
        Output(Vec<u8>),
        Exit,
    }

    /// `sh` and `stty` exist on every Unix; Windows gets its own test below.
    #[cfg(unix)]
    fn shell(script: &str) -> AttachCommand {
        AttachCommand {
            program: "sh".into(),
            args: vec!["-c".into(), script.into()],
            env: Vec::new(),
            env_remove: Vec::new(),
            directory: None,
        }
    }

    fn open(command: &AttachCommand, columns: u16, rows: u16) -> (Pane, Receiver<Seen>) {
        let (sender, seen) = mpsc::channel();
        let exit = sender.clone();
        let pane = Pane::open(
            command,
            columns,
            rows,
            move |bytes| {
                let _ = sender.send(Seen::Output(bytes.to_vec()));
            },
            move || {
                let _ = exit.send(Seen::Exit);
            },
        )
        .expect("pty opens");
        (pane, seen)
    }

    /// Everything printed until `wanted` shows up, or until the timeout.
    fn wait_for(seen: &Receiver<Seen>, wanted: &str) -> String {
        let deadline = Instant::now() + TIMEOUT;
        let mut text = String::new();
        while !text.contains(wanted) {
            let left = deadline.saturating_duration_since(Instant::now());
            match seen.recv_timeout(left) {
                Ok(Seen::Output(bytes)) => text.push_str(&String::from_utf8_lossy(&bytes)),
                Ok(Seen::Exit) | Err(_) => break,
            }
        }
        text
    }

    fn exited(seen: &Receiver<Seen>) -> bool {
        let deadline = Instant::now() + TIMEOUT;
        while let Ok(event) = seen.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            if let Seen::Exit = event {
                return true;
            }
        }
        false
    }

    #[test]
    #[cfg(unix)]
    fn output_arrives_then_the_end_is_reported() {
        let (_pane, seen) = open(&shell("printf ready"), 80, 24);
        assert!(wait_for(&seen, "ready").contains("ready"));
        assert!(exited(&seen));
    }

    /// How the implicit `default` drops `CLAUDE_CONFIG_DIR`; `HOME` is always inherited on Unix.
    #[test]
    #[cfg(unix)]
    fn removed_variables_are_not_inherited() {
        let mut command = shell("printf '[%s]' \"${HOME-unset}\"");
        command.env_remove = vec!["HOME".into()];
        let (_pane, seen) = open(&command, 80, 24);
        let text = wait_for(&seen, "]");
        assert!(text.contains("[unset]"), "{text:?}");
    }

    /// How Claude Code is started in the folder it asks about.
    #[test]
    #[cfg(unix)]
    fn the_program_starts_in_the_directory_given() {
        let directory = std::env::temp_dir().canonicalize().expect("temp dir");
        let mut command = shell("printf '[%s]' \"$(pwd -P)\"");
        command.directory = Some(directory.clone());
        let (_pane, seen) = open(&command, 80, 24);
        let text = wait_for(&seen, "]");
        assert!(text.contains(&format!("[{}]", directory.display())), "{text:?}");
    }

    #[test]
    #[cfg(unix)]
    fn typed_input_reaches_the_program() {
        let (mut pane, seen) = open(&shell("read line; printf 'got:%s' \"$line\""), 80, 24);
        pane.write(b"abc\r").expect("write");
        let text = wait_for(&seen, "got:abc");
        assert!(text.contains("got:abc"), "{text:?}");
    }

    #[test]
    #[cfg(unix)]
    fn resize_reaches_the_program() {
        let (mut pane, seen) = open(&shell("read go; stty size"), 80, 24);
        pane.resize(100, 30).expect("resize");
        pane.write(b"\r").expect("write");
        let text = wait_for(&seen, "30 100");
        assert!(text.contains("30 100"), "{text:?}");
    }

    #[test]
    #[cfg(unix)]
    fn closing_the_pane_ends_the_program() {
        let (pane, seen) = open(&shell("sleep 30"), 80, 24);
        let started = Instant::now();
        drop(pane);
        assert!(started.elapsed() < TIMEOUT, "closing waited for the program to end by itself");
        assert!(exited(&seen));
    }

    #[test]
    #[cfg(unix)]
    fn detaching_returns_before_a_slow_program_has_ended() {
        let (pane, seen) = open(&shell("trap '' HUP; printf ready; sleep 2"), 80, 24);
        assert!(wait_for(&seen, "ready").contains("ready"));
        let started = Instant::now();
        let detached = detach(pane);
        assert!(started.elapsed() < Duration::from_millis(200), "detaching waited {:?}", started.elapsed());
        detached.join().expect("the pane is dropped");
        assert!(exited(&seen));
    }

    /// The Windows pseudo-console (ConPTY): output arrives and the end of the program is seen.
    #[test]
    #[cfg(windows)]
    fn windows_output_arrives_then_the_end_is_reported() {
        let command = AttachCommand {
            program: "cmd".into(),
            args: vec!["/C".into(), "echo ready".into()],
            env: Vec::new(),
            env_remove: Vec::new(),
            directory: None,
        };
        let (mut pane, seen) = open(&command, 80, 24);
        // ConPTY first asks where the cursor is and holds the output until a terminal
        // answers; in the app, xterm.js answers by itself.
        let query = wait_for(&seen, "\x1b[6n");
        assert!(query.contains("\x1b[6n"), "{query:?}");
        pane.write(b"\x1b[1;1R").expect("write");
        let text = wait_for(&seen, "ready");
        assert!(text.contains("ready"), "{text:?}");
        assert!(exited(&seen));
    }
}
