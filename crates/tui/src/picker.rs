use kindric_core::accounts::SessionInfo;
use kindric_core::search::matches;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    Session(usize),
    Redraw,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PickerOutcome {
    Continue,
    Cancel,
    Chosen(Entry),
}

#[derive(Debug, Default)]
pub struct Picker {
    pub query: String,
    pub selected: usize,
}

impl Picker {
    pub fn entries(&self, sessions: &[SessionInfo]) -> Vec<Entry> {
        sessions
            .iter()
            .enumerate()
            .filter(|(_, session)| matches(session, &self.query))
            .map(|(index, _)| Entry::Session(index))
            .chain(std::iter::once(Entry::Redraw))
            .collect()
    }

    pub fn handle_key(&mut self, key: KeyEvent, sessions: &[SessionInfo]) -> PickerOutcome {
        let entries = self.entries(sessions);
        match key.code {
            KeyCode::Esc => PickerOutcome::Cancel,
            KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => PickerOutcome::Cancel,
            KeyCode::Enter => entries
                .get(self.selected)
                .copied()
                .map_or(PickerOutcome::Continue, PickerOutcome::Chosen),
            KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                PickerOutcome::Continue
            },
            KeyCode::Down => {
                self.selected = (self.selected + 1).min(entries.len() - 1);
                PickerOutcome::Continue
            },
            KeyCode::Backspace => {
                self.query.pop();
                self.selected = 0;
                PickerOutcome::Continue
            },
            KeyCode::Char(c) if !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => {
                self.query.push(c);
                self.selected = 0;
                PickerOutcome::Continue
            },
            _ => PickerOutcome::Continue,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kindric_core::accounts::SessionState;

    fn session(account: &str, name: &str, cwd: &str) -> SessionInfo {
        SessionInfo {
            account: account.into(),
            short_id: Some("abc".into()),
            session_id: format!("{account}-{name}"),
            name: name.into(),
            cwd: cwd.into(),
            state: SessionState::Waiting,
            isolated: false,
            progress: None,
        }
    }

    fn sessions() -> Vec<SessionInfo> {
        vec![
            session("louis", "boss-fight", "/home/u/Roblox/frieren"),
            session("lystriq", "api", "/home/u/Pro/api"),
            session("louis", "", "/home/u/Merkio"),
        ]
    }

    fn type_text(picker: &mut Picker, text: &str, sessions: &[SessionInfo]) {
        for c in text.chars() {
            picker.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE), sessions);
        }
    }

    #[test]
    fn redraw_entry_always_comes_last() {
        let sessions = sessions();
        let mut picker = Picker::default();
        assert_eq!(picker.entries(&sessions).len(), 4);
        type_text(&mut picker, "zzz", &sessions);
        assert_eq!(picker.entries(&sessions), [Entry::Redraw]);
    }

    #[test]
    fn typing_filters_and_enter_picks_the_selected_session() {
        let sessions = sessions();
        let mut picker = Picker::default();
        type_text(&mut picker, "louis", &sessions);
        picker.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), &sessions);
        let outcome = picker.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &sessions);
        assert_eq!(outcome, PickerOutcome::Chosen(Entry::Session(2)));
    }

    #[test]
    fn selection_stays_in_bounds() {
        let sessions = sessions();
        let mut picker = Picker::default();
        for _ in 0..10 {
            picker.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), &sessions);
        }
        assert_eq!(picker.selected, 3);
        let escape = picker.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &sessions);
        assert_eq!(escape, PickerOutcome::Cancel);
    }
}
