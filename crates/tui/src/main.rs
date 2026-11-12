mod app;
mod keys;
mod layout;
mod new_session;
mod pane;
mod picker;
mod render;
mod ui;

use std::io::{self, stdout};
use std::iter;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::mpsc::{self, Sender};
use std::thread;

use alacritty_terminal::event::Event as TermEvent;
use kindric_core::launcher::Launched;
use kindric_core::{accounts, mcp, refresher};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste, Event};
use ratatui::crossterm::execute;
use ratatui::layout::Rect;

use crate::app::{App, Flow};
use crate::pane::PaneKey;

const USAGE: &str = "usage: kindric [mcp | unread | --version]";

pub enum AppEvent {
    Input(Event),
    InputFailed(io::Error),
    Term(PaneKey, TermEvent),
    Snapshot(refresher::Update),
    Created(Result<(String, Launched), String>),
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] => run_ui(),
        ["mcp"] => mcp::run(),
        // The hook of the sessions kindric-accounts starts: tells a session of its unread messages.
        ["unread"] => {
            kindric_core::mailbox::run_unread_hook();
            Ok(())
        },
        ["--version" | "-V"] => {
            println!("kindric {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        },
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        },
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("kindric: {error}");
            ExitCode::FAILURE
        },
    }
}

fn run_ui() -> io::Result<()> {
    accounts::check_launcher_version()?;
    let start_dir = std::env::current_dir()?;
    let home = std::env::home_dir().ok_or_else(|| io::Error::other("HOME is not set"))?;
    let result = ratatui::try_init().and_then(|mut terminal| run_loop(&mut terminal, home, start_dir));
    let restored = execute!(stdout(), DisableBracketedPaste).and_then(|()| ratatui::try_restore());
    result.and(restored)
}

fn run_loop(terminal: &mut DefaultTerminal, home: PathBuf, start_dir: PathBuf) -> io::Result<()> {
    execute!(stdout(), EnableBracketedPaste)?;
    let (events, inbox) = mpsc::channel();
    spawn_input_reader(events.clone());
    let publisher = events.clone();
    refresher::spawn(false, move |update| publisher.send(AppEvent::Snapshot(update)).is_ok());
    let size = terminal.size()?;
    let mut app = App::new(events, Rect::new(0, 0, size.width, size.height), home, start_dir);
    loop {
        terminal.draw(|frame| ui::draw(frame, &app))?;
        let first = inbox.recv().map_err(|_| io::Error::other("event channel closed"))?;
        for event in iter::once(first).chain(inbox.try_iter()) {
            match app.handle(event) {
                Flow::Continue => {},
                Flow::Quit => return Ok(()),
                Flow::FullRedraw => terminal.clear()?,
            }
        }
    }
}

fn spawn_input_reader(events: Sender<AppEvent>) {
    thread::spawn(move || {
        loop {
            let message = match event::read() {
                Ok(event) => AppEvent::Input(event),
                Err(error) => AppEvent::InputFailed(error),
            };
            let failed = matches!(message, AppEvent::InputFailed(_));
            if events.send(message).is_err() || failed {
                return;
            }
        }
    });
}
