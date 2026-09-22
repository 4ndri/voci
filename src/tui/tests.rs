//! Shared fixtures for behavior-focused TUI regression modules.

mod clipboard;
mod details;
mod editing;
mod history;
mod navigation;
mod suggestions;

use super::input::Input;
use super::*;
use crate::history::HistoryFilter;
use crate::{
    app::Coordinator,
    domain::{INITIAL_PAIRS, ResultKind, TranslationCandidate},
    tui::clipboard::Clipboard,
    tui::keybindings::Action,
};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::{Terminal, backend::TestBackend};

fn key(c: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

fn enter() -> Event {
    Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
}

#[derive(Default)]
struct TestClipboard {
    text: String,
    unavailable: bool,
}

impl Clipboard for TestClipboard {
    fn copy(&mut self, text: String) -> Result<(), String> {
        if self.unavailable {
            return Err("Clipboard unavailable".into());
        }
        self.text = text;
        Ok(())
    }

    fn paste(&mut self) -> Result<String, String> {
        if self.unavailable {
            return Err("Clipboard unavailable".into());
        }
        Ok(self.text.clone())
    }
}

fn saved_entry(sequence: i64) -> HistoryEntry {
    HistoryEntry {
        id: sequence.to_string(),
        sequence,
        query: format!("word {sequence}"),
        from: Some(Language::German),
        to: None,
        provider: None,
        started_at: sequence,
        finished_at: None,
        finished: None,
    }
}

fn page(sequences: &[i64]) -> Result<HistoryPage, String> {
    Ok(HistoryPage {
        entries: sequences.iter().map(|&i| saved_entry(i)).collect(),
        has_more: false,
    })
}

fn ctrl(c: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
}

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> Event {
    Event::Mouse(crossterm::event::MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

fn click(column: u16, row: u16) -> Event {
    mouse(MouseEventKind::Down(MouseButton::Left), column, row)
}

fn screen(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

fn scrolling_app(view: usize, count: usize) -> App {
    let mut app = App::new(None, None, Keybindings::default());
    app.focus = Focus::Details;
    let result = LookupResult {
        query: "word".into(),
        headword: "word".into(),
        normalized_headword: "word".into(),
        pair: INITIAL_PAIRS[0],
        candidates: (0..count)
            .map(|index| TranslationCandidate {
                text: format!("value {index}"),
                normalized: format!("value {index}"),
                part_of_speech: None,
                sense: None,
                prefix: String::new(),
                back_translations: vec![],
            })
            .collect(),
        provider: "fixture".into(),
        attribution: None,
        kind: ResultKind::Dictionary,
    };
    if view == 0 {
        app.lookup.live = Some(result);
    } else {
        let mut entry = saved_entry(1);
        entry.finished = Some(crate::app::history_outcome(&Ok(result)));
        if view == 1 {
            app.lookup.preview = Some(entry);
        } else {
            app.tab = Tab::History;
            app.history.entries.push(entry);
            app.history.state.select(Some(0));
        }
    }
    app
}
