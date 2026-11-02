use kindric_core::accounts::SessionInfo;
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};

use crate::app::{App, Overlay};
use crate::layout;
use crate::new_session::{Field, NewSessionForm};
use crate::picker::{Entry, Picker};
use crate::render;

const ACTIVE: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
const MUTED: Style = Style::new().add_modifier(Modifier::DIM);
const ERROR: Style = Style::new().fg(Color::Red);

pub fn draw(frame: &mut Frame, app: &App) {
    let areas = layout::split(frame.area());
    draw_list(frame, areas.list, app);
    let mut cursor = None;
    for (index, area) in areas.panes.into_iter().enumerate() {
        let pane_cursor = draw_pane(frame, area, app, index);
        if index == app.active {
            cursor = pane_cursor;
        }
    }
    let screen = frame.area();
    cursor = match &app.overlay {
        Overlay::Hidden => cursor,
        Overlay::Picker(picker) => Some(draw_picker(frame, screen, picker, app)),
        Overlay::NewSession(form) => Some(draw_form(frame, screen, form)),
    };
    if let Some(position) = cursor {
        frame.set_cursor_position(position);
    }
}

fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let kept: String = text.chars().take(width.saturating_sub(1)).collect();
    format!("{kept}…")
}

fn session_lines(app: &App, index: usize, session: &SessionInfo, width: usize) -> [Line<'static>; 2] {
    let number = if index < 9 { format!("{} ", index + 1) } else { "  ".to_string() };
    let style = match app.slots.iter().position(|slot| slot.as_ref().is_some_and(|slot| slot.shows(session))) {
        Some(slot) if slot == app.active => ACTIVE.add_modifier(Modifier::REVERSED),
        Some(_) => ACTIVE,
        None => Style::new(),
    };
    let unread = app.unread.get(&session.session_id).copied().unwrap_or(0);
    let mut detail = format!("  {} · {}", session.account, session.state.label());
    if session.isolated {
        detail.push_str(" · isolated");
    }
    [title_line(&number, &session.title(), unread, width, style), Line::styled(truncate(&detail, width), MUTED)]
}

/// The unread count sits at the end of the title line, so a long title never hides it.
fn title_line(number: &str, title: &str, unread: usize, width: usize, style: Style) -> Line<'static> {
    if unread == 0 {
        return Line::styled(truncate(&format!("{number}{title}"), width), style);
    }
    let badge = format!(" {unread} msg");
    let room = width.saturating_sub(number.chars().count() + badge.chars().count());
    Line::from(vec![
        Span::styled(format!("{number}{}", truncate(title, room)), style),
        Span::styled(badge, Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
    ])
}

fn draw_list(frame: &mut Frame, area: Rect, app: &App) {
    let width = usize::from(layout::inner(area).width);
    let mut lines = Vec::new();
    match &app.snapshot {
        None => lines.push(Line::styled("loading…", MUTED)),
        Some(snapshot) if snapshot.sessions.is_empty() => {
            lines.push(Line::styled("no sessions", MUTED));
            lines.push(Line::styled("Ctrl+N to start one", MUTED));
        },
        Some(snapshot) => {
            for (index, session) in snapshot.sessions.iter().enumerate() {
                lines.extend(session_lines(app, index, session, width));
            }
        },
    }
    let errors = app.snapshot.iter().flat_map(|snapshot| snapshot.errors.iter());
    for error in errors.chain(app.status.iter()) {
        lines.push(Line::default());
        lines.push(Line::styled(error.clone(), ERROR));
    }
    let block = Block::bordered().title(" sessions ").title_bottom(Line::styled(" Ctrl+P · Ctrl+N ", MUTED));
    frame.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }), area);
}

fn draw_pane(frame: &mut Frame, area: Rect, app: &App, index: usize) -> Option<Position> {
    let is_active = index == app.active;
    let border = if is_active { ACTIVE } else { Style::new() };
    let side = if index == 0 { "Ctrl+H" } else { "Ctrl+L" };
    let Some(slot) = &app.slots[index] else {
        let help = vec![
            Line::default(),
            Line::from("Alt+1..9  show session n"),
            Line::from("Ctrl+P    find a session"),
            Line::from("Ctrl+N    new session"),
            Line::from("Ctrl+H/L  left / right pane"),
            Line::from("Ctrl+Q    quit (sessions keep running)"),
        ];
        let block = Block::bordered().border_style(border).title(format!(" empty · {side} "));
        frame.render_widget(Paragraph::new(help).style(MUTED).block(block), area);
        return None;
    };
    let session_title = app
        .sessions()
        .iter()
        .find(|session| slot.shows(session))
        .map_or_else(|| slot.short_id.clone(), SessionInfo::title);
    let title = format!(" {} · {} ", session_title, slot.account);
    frame.render_widget(Block::bordered().border_style(border).title(Span::styled(title, border)), area);
    let inner = layout::inner(area);
    slot.pane.with_term(|term| render::draw_term(term, inner, frame.buffer_mut()))
}

fn centered(screen: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(screen.width);
    let height = height.min(screen.height);
    Rect::new(screen.x + (screen.width - width) / 2, screen.y + (screen.height - height) / 3, width, height)
}

fn draw_picker(frame: &mut Frame, screen: Rect, picker: &Picker, app: &App) -> Position {
    let sessions = app.sessions();
    let entries = picker.entries(sessions);
    let area = centered(screen, 70, entries.len().min(15) as u16 + 4);
    let width = usize::from(layout::inner(area).width);
    let mut lines = vec![Line::from(format!("> {}", picker.query)), Line::default()];
    let first = picker.selected.saturating_sub(14);
    for (offset, entry) in entries.iter().enumerate().skip(first).take(15) {
        let text = match entry {
            Entry::Session(index) => {
                let session = &sessions[*index];
                format!("{} · {} · {}", session.title(), session.account, session.state.label())
            },
            Entry::Redraw => "Redraw the screen".to_string(),
        };
        let style = if offset == picker.selected { Style::new().add_modifier(Modifier::REVERSED) } else { Style::new() };
        lines.push(Line::styled(truncate(&text, width), style));
    }
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(lines).block(Block::bordered().border_style(ACTIVE).title(" go to ")), area);
    let column = (picker.query.chars().count() + 2).min(width.saturating_sub(1)) as u16;
    Position::new(area.x + 1 + column, area.y + 1)
}

fn draw_form(frame: &mut Frame, screen: Rect, form: &NewSessionForm) -> Position {
    let area = centered(screen, 70, 10);
    let width = usize::from(layout::inner(area).width);
    let account = form.accounts.get(form.account).map_or("(no account)", String::as_str);
    let label_style = |field: Field| if form.field == field { ACTIVE } else { Style::new() };
    let mut lines = vec![
        Line::from(vec![Span::styled("Account   ", label_style(Field::Account)), Span::raw(format!("‹ {account} ›"))]),
        Line::from(vec![
            Span::styled("Directory ", label_style(Field::Directory)),
            Span::raw(truncate(&form.directory, width.saturating_sub(10))),
        ]),
        Line::from(vec![Span::styled("Name      ", label_style(Field::Name)), Span::raw(form.name.clone())]),
        Line::default(),
    ];
    if let Some(error) = &form.error {
        lines.push(Line::styled(error.clone(), ERROR));
    }
    lines.push(Line::styled("Tab next field · ←/→ account · Enter start · Esc cancel", MUTED));
    frame.render_widget(Clear, area);
    let block = Block::bordered().border_style(ACTIVE).title(" new session ");
    frame.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }), area);
    let (row, text) = match form.field {
        Field::Account => (0, format!("‹ {account}")),
        Field::Directory => (1, truncate(&form.directory, width.saturating_sub(10))),
        Field::Name => (2, form.name.clone()),
    };
    let column = (10 + text.chars().count()).min(width.saturating_sub(1)) as u16;
    Position::new(area.x + 1 + column, area.y + 1 + row)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_keeps_short_text_and_marks_cuts() {
        assert_eq!(truncate("roblox", 10), "roblox");
        assert_eq!(truncate("frieren-world-boss", 8), "frieren…");
        assert_eq!(truncate("é", 1), "é");
    }

    #[test]
    fn unread_badge_survives_a_long_title() {
        let line = title_line("3 ", "Router CEO et flux de données Overview", 2, 26, Style::new());
        let text: String = line.spans.iter().map(|span| span.content.as_ref()).collect();
        assert!(text.ends_with(" 2 msg"), "{text}");
        assert_eq!(text.chars().count(), 26);
        let plain = title_line("3 ", "cs-test", 0, 26, Style::new());
        assert_eq!(plain.spans.iter().map(|span| span.content.as_ref()).collect::<String>(), "3 cs-test");
    }

    #[test]
    fn centered_box_fits_small_screens() {
        assert_eq!(centered(Rect::new(0, 0, 50, 8), 70, 10), Rect::new(0, 0, 50, 8));
        assert_eq!(centered(Rect::new(0, 0, 100, 30), 70, 10), Rect::new(15, 6, 70, 10));
    }
}
