use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::thread;

use alacritty_terminal::event::Event as TermEvent;
use alacritty_terminal::term::TermMode;
use kindric_core::accounts::{self, SessionInfo, Snapshot};
use kindric_core::launcher;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;

use crate::AppEvent;
use crate::keys;
use crate::layout::{self, SLOTS};
use crate::new_session::{FormOutcome, NewSessionForm};
use crate::pane::{Pane, PaneKey};
use crate::picker::{Entry, Picker, PickerOutcome};

#[derive(Debug, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Quit,
    FullRedraw,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    FocusLeft,
    FocusRight,
    NewSession,
    Show(usize),
    OpenPicker,
    Quit,
    Forward,
}

/// Keys the tool keeps for itself; everything else goes to the active pane.
/// Ctrl+L never reaches Claude: it moves to the right pane.
pub fn command_for(key: KeyEvent) -> Command {
    let ctrl = key.modifiers == KeyModifiers::CONTROL;
    let alt = key.modifiers == KeyModifiers::ALT;
    match key.code {
        KeyCode::Char('h') if ctrl => Command::FocusLeft,
        KeyCode::Char('l') if ctrl => Command::FocusRight,
        KeyCode::Char('n') if ctrl => Command::NewSession,
        KeyCode::Char('p') if ctrl => Command::OpenPicker,
        KeyCode::Char('q') if ctrl => Command::Quit,
        KeyCode::Char(digit @ '1'..='9') if alt => Command::Show(digit as usize - '1' as usize),
        _ => Command::Forward,
    }
}

pub enum Overlay {
    Hidden,
    Picker(Picker),
    NewSession(NewSessionForm),
}

pub struct Slot {
    pub pane: Pane,
    pub account: String,
    pub short_id: String,
    pub title: String,
}

impl Slot {
    pub fn shows(&self, session: &SessionInfo) -> bool {
        session.account == self.account && session.short_id.as_deref() == Some(self.short_id.as_str())
    }
}

pub struct App {
    pub snapshot: Option<Snapshot>,
    pub unread: HashMap<String, usize>,
    pub slots: [Option<Slot>; SLOTS],
    pub active: usize,
    pub overlay: Overlay,
    pub status: Option<String>,
    generation: u64,
    screen: Rect,
    events: Sender<AppEvent>,
    home: PathBuf,
    start_dir: PathBuf,
}

impl App {
    pub fn new(events: Sender<AppEvent>, screen: Rect, home: PathBuf, start_dir: PathBuf) -> Self {
        Self {
            snapshot: None,
            unread: HashMap::new(),
            slots: [None, None],
            active: 0,
            overlay: Overlay::Hidden,
            status: None,
            generation: 0,
            screen,
            events,
            home,
            start_dir,
        }
    }

    pub fn sessions(&self) -> &[SessionInfo] {
        self.snapshot.as_ref().map_or(&[], |snapshot| snapshot.sessions.as_slice())
    }

    pub fn handle(&mut self, event: AppEvent) -> Flow {
        match event {
            AppEvent::Input(Event::Key(key)) if key.kind != KeyEventKind::Release => self.on_key(key),
            AppEvent::Input(Event::Paste(text)) => {
                self.paste(&text);
                Flow::Continue
            },
            AppEvent::Input(Event::Resize(columns, lines)) => {
                self.resize(Rect::new(0, 0, columns, lines));
                Flow::Continue
            },
            AppEvent::Input(_) => Flow::Continue,
            AppEvent::InputFailed(error) => {
                self.status = Some(format!("keyboard lost: {error}"));
                self.quit()
            },
            AppEvent::Term(key, event) => {
                self.on_term(key, event);
                Flow::Continue
            },
            AppEvent::Snapshot(result) => {
                match result {
                    Ok((snapshot, unread)) => {
                        self.snapshot = Some(accounts::without_implicit_default(snapshot));
                        self.unread = unread;
                    },
                    Err(error) => self.status = Some(format!("session list unavailable: {error}")),
                }
                Flow::Continue
            },
            AppEvent::Created(result) => {
                match result {
                    Ok((account, launched)) => {
                        self.attach(&account, &launched.short_id);
                        if !launched.warnings.is_empty() {
                            self.status = Some(launched.warnings.join(" · "));
                        }
                    },
                    Err(error) => self.status = Some(error),
                }
                Flow::Continue
            },
        }
    }

    fn on_key(&mut self, key: KeyEvent) -> Flow {
        match std::mem::replace(&mut self.overlay, Overlay::Hidden) {
            Overlay::Picker(picker) => return self.on_picker_key(picker, key),
            Overlay::NewSession(form) => return self.on_form_key(form, key),
            Overlay::Hidden => {},
        }
        match command_for(key) {
            Command::FocusLeft => self.active = 0,
            Command::FocusRight => self.active = SLOTS - 1,
            Command::NewSession => self.open_form(),
            Command::Show(index) => self.show(index),
            Command::OpenPicker => self.overlay = Overlay::Picker(Picker::default()),
            Command::Quit => return self.quit(),
            Command::Forward => self.forward(key),
        }
        Flow::Continue
    }

    fn on_picker_key(&mut self, mut picker: Picker, key: KeyEvent) -> Flow {
        match picker.handle_key(key, self.sessions()) {
            PickerOutcome::Continue => self.overlay = Overlay::Picker(picker),
            PickerOutcome::Cancel => {},
            PickerOutcome::Chosen(Entry::Session(index)) => self.show(index),
            PickerOutcome::Chosen(Entry::Redraw) => return Flow::FullRedraw,
        }
        Flow::Continue
    }

    fn on_form_key(&mut self, mut form: NewSessionForm, key: KeyEvent) -> Flow {
        match form.handle_key(key, &self.home) {
            FormOutcome::Continue => self.overlay = Overlay::NewSession(form),
            FormOutcome::Cancel => {},
            FormOutcome::Submit(request) => {
                self.status = Some(format!("starting a session on {}…", request.account));
                let events = self.events.clone();
                thread::spawn(move || {
                    let result = launcher::launch(&request, None).map(|launched| (request.account.clone(), launched));
                    // The receiver only disappears when the app is shutting down.
                    let _ = events.send(AppEvent::Created(result));
                });
            },
        }
        Flow::Continue
    }

    fn open_form(&mut self) {
        let accounts = self.snapshot.as_ref().map(|snapshot| snapshot.accounts.clone()).unwrap_or_default();
        self.overlay = Overlay::NewSession(NewSessionForm::new(accounts, &self.start_dir));
    }

    fn show(&mut self, index: usize) {
        let Some(session) = self.sessions().get(index).cloned() else {
            self.status = Some(format!("no session #{}", index + 1));
            return;
        };
        let Some(short_id) = session.short_id.clone() else {
            self.status = Some(format!("\"{}\" runs in another terminal: it cannot be opened here", session.title()));
            return;
        };
        let other = (self.active + 1) % SLOTS;
        if self.slots[other].as_ref().is_some_and(|slot| slot.shows(&session)) {
            self.active = other;
            return;
        }
        if self.slots[self.active].as_ref().is_some_and(|slot| slot.shows(&session)) {
            return;
        }
        self.attach(&session.account, &short_id);
    }

    fn attach(&mut self, account: &str, short_id: &str) {
        self.generation += 1;
        let key = PaneKey { slot: self.active, generation: self.generation };
        let size = layout::pane_size(layout::split(self.screen).panes[self.active]);
        match Pane::attach(account, short_id, key, size, self.events.clone()) {
            Ok(pane) => {
                let slot = Slot { pane, account: account.to_string(), short_id: short_id.to_string(), title: String::new() };
                if let Some(old) = self.slots[self.active].replace(slot) {
                    // Detaching only: the background session keeps running in Claude Code.
                    let _ = old.pane.shutdown();
                }
                self.status = None;
            },
            Err(error) => self.status = Some(format!("cannot open {short_id}: {error}")),
        }
    }

    fn on_term(&mut self, key: PaneKey, event: TermEvent) {
        let Some(slot) = self.slots[key.slot].as_mut().filter(|slot| slot.pane.key == key) else {
            return;
        };
        match event {
            TermEvent::PtyWrite(text) => {
                if slot.pane.write(text.into_bytes()).is_err() {
                    self.slots[key.slot] = None;
                }
            },
            TermEvent::Title(title) => slot.title = title,
            TermEvent::ResetTitle => slot.title.clear(),
            TermEvent::ChildExit(_) | TermEvent::Exit => {
                self.slots[key.slot] = None;
                self.status = Some("session detached (it keeps running in the background)".to_string());
            },
            _ => {},
        }
    }

    fn active_mode(&self) -> Option<TermMode> {
        self.slots[self.active].as_ref().map(|slot| slot.pane.with_term(|term| *term.mode()))
    }

    fn write_active(&mut self, bytes: Vec<u8>) {
        let failed = self.slots[self.active].as_ref().is_some_and(|slot| slot.pane.write(bytes).is_err());
        if failed {
            self.slots[self.active] = None;
        }
    }

    fn forward(&mut self, key: KeyEvent) {
        let Some(mode) = self.active_mode() else {
            return;
        };
        if let Some(bytes) = keys::key_bytes(key, mode.contains(TermMode::APP_CURSOR)) {
            self.write_active(bytes);
        }
    }

    fn paste(&mut self, text: &str) {
        match &mut self.overlay {
            Overlay::Picker(picker) => picker.query.push_str(text.trim()),
            Overlay::NewSession(form) => form.directory = text.trim().to_string(),
            Overlay::Hidden => {
                if let Some(mode) = self.active_mode() {
                    self.write_active(keys::paste_bytes(text, mode.contains(TermMode::BRACKETED_PASTE)));
                }
            },
        }
    }

    fn resize(&mut self, screen: Rect) {
        self.screen = screen;
        let areas = layout::split(screen);
        for (index, slot) in self.slots.iter_mut().enumerate() {
            let failed = slot.as_ref().is_some_and(|slot| slot.pane.resize(layout::pane_size(areas.panes[index])).is_err());
            if failed {
                *slot = None;
            }
        }
    }

    fn quit(&mut self) -> Flow {
        for slot in self.slots.iter().flatten() {
            // Detaching only: the background sessions keep running in Claude Code.
            let _ = slot.pane.shutdown();
        }
        Flow::Quit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn tool_keys() {
        assert_eq!(command_for(key(KeyCode::Char('h'), KeyModifiers::CONTROL)), Command::FocusLeft);
        assert_eq!(command_for(key(KeyCode::Char('l'), KeyModifiers::CONTROL)), Command::FocusRight);
        assert_eq!(command_for(key(KeyCode::Char('n'), KeyModifiers::CONTROL)), Command::NewSession);
        assert_eq!(command_for(key(KeyCode::Char('p'), KeyModifiers::CONTROL)), Command::OpenPicker);
        assert_eq!(command_for(key(KeyCode::Char('q'), KeyModifiers::CONTROL)), Command::Quit);
        assert_eq!(command_for(key(KeyCode::Char('2'), KeyModifiers::ALT)), Command::Show(1));
        assert_eq!(command_for(key(KeyCode::Char('9'), KeyModifiers::ALT)), Command::Show(8));
    }

    #[test]
    fn claude_keeps_everything_else() {
        let forwarded = [
            key(KeyCode::Char('c'), KeyModifiers::CONTROL),
            key(KeyCode::Char('l'), KeyModifiers::NONE),
            key(KeyCode::Char('0'), KeyModifiers::ALT),
            key(KeyCode::Char('2'), KeyModifiers::NONE),
            key(KeyCode::Char('h'), KeyModifiers::CONTROL | KeyModifiers::ALT),
            key(KeyCode::Backspace, KeyModifiers::NONE),
            key(KeyCode::Left, KeyModifiers::NONE),
        ];
        for key in forwarded {
            assert_eq!(command_for(key), Command::Forward, "{key:?}");
        }
    }
}
