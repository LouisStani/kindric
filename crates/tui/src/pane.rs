use std::io;
use std::sync::Arc;
use std::sync::mpsc::Sender;

use alacritty_terminal::event::{Event as TermEvent, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSendError, EventLoopSender, Msg};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::tty::{self, Options, Shell};
use claude_sessions_core::attach::{AttachCommand, attach_command};

use crate::AppEvent;

/// Identifies one attached pane. `generation` changes every time a slot gets a new
/// session, so late events from the previous one can be told apart and dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaneKey {
    pub slot: usize,
    pub generation: u64,
}

#[derive(Clone)]
pub struct Listener {
    key: PaneKey,
    events: Sender<AppEvent>,
}

impl EventListener for Listener {
    fn send_event(&self, event: TermEvent) {
        // The receiver only disappears when the app is shutting down.
        let _ = self.events.send(AppEvent::Term(self.key, event));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaneSize {
    pub columns: u16,
    pub lines: u16,
}

impl PaneSize {
    fn window_size(self) -> WindowSize {
        WindowSize { num_lines: self.lines, num_cols: self.columns, cell_width: 0, cell_height: 0 }
    }
}

impl Dimensions for PaneSize {
    fn total_lines(&self) -> usize {
        self.screen_lines()
    }

    fn screen_lines(&self) -> usize {
        usize::from(self.lines)
    }

    fn columns(&self) -> usize {
        usize::from(self.columns)
    }
}

pub struct Pane {
    pub key: PaneKey,
    term: Arc<FairMutex<Term<Listener>>>,
    sender: EventLoopSender,
}

impl Pane {
    /// Opens background session `short_id` of `account` in a new pty.
    pub fn attach(
        account: &str,
        short_id: &str,
        key: PaneKey,
        size: PaneSize,
        events: Sender<AppEvent>,
    ) -> io::Result<Self> {
        let listener = Listener { key, events };
        // The terminal interface never shows the implicit `default`: its pty cannot remove
        // `CLAUDE_CONFIG_DIR`.
        let options = launcher_options(&attach_command(account, short_id, false));
        let pty = tty::new(&options, size.window_size(), 0)?;
        let term = Term::new(Config::default(), &size, listener.clone());
        let term = Arc::new(FairMutex::new(term));
        let event_loop = EventLoop::new(Arc::clone(&term), listener, pty, false, false)?;
        let sender = event_loop.channel();
        event_loop.spawn();
        Ok(Self { key, term, sender })
    }

    pub fn write(&self, bytes: Vec<u8>) -> Result<(), EventLoopSendError> {
        self.sender.send(Msg::Input(bytes.into()))
    }

    pub fn resize(&self, size: PaneSize) -> Result<(), EventLoopSendError> {
        self.term.lock().resize(size);
        self.sender.send(Msg::Resize(size.window_size()))
    }

    pub fn shutdown(&self) -> Result<(), EventLoopSendError> {
        self.sender.send(Msg::Shutdown)
    }

    pub fn with_term<R>(&self, read: impl FnOnce(&Term<Listener>) -> R) -> R {
        read(&self.term.lock())
    }
}

fn launcher_options(command: &AttachCommand) -> Options {
    let shell = Shell::new(command.program.clone(), command.args.clone());
    let env = command.env.iter().cloned().collect();
    Options { shell: Some(shell), env, ..Options::default() }
}
