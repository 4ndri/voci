use crate::{
    app::LookupService,
    domain::*,
    presentation::{result_lines, safe_text},
    provider::DictionaryProvider,
};
use crossterm::{
    event::{
        DisableBracketedPaste, EnableBracketedPaste, Event, EventStream, KeyCode, KeyEventKind,
        KeyModifiers,
    },
    execute,
};
use futures_util::StreamExt;
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use std::{
    io::{self, IsTerminal},
    sync::Arc,
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

pub fn check_terminal() -> io::Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(io::Error::other(
            "voci shell requires an interactive terminal on stdin and stdout. Use voci <word> for redirected output.",
        ));
    }
    Ok(())
}

struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), DisableBracketedPaste);
        ratatui::restore();
    }
}

#[derive(Default)]
struct Input {
    text: String,
    cursor: usize,
}

impl Input {
    fn insert(&mut self, text: &str) {
        self.text.insert_str(self.cursor, text);
        self.cursor += text.len();
        // Inserting combining marks can change grapheme boundaries around the cursor.
        self.cursor = self
            .text
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .chain(std::iter::once(self.text.len()))
            .find(|index| *index >= self.cursor)
            .unwrap_or(self.text.len());
    }

    fn left(&mut self) {
        self.cursor = self.text[..self.cursor]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(index, _)| index);
    }

    fn right(&mut self) {
        if let Some(grapheme) = self.text[self.cursor..].graphemes(true).next() {
            self.cursor += grapheme.len();
        }
    }

    fn backspace(&mut self) {
        let end = self.cursor;
        self.left();
        self.text.drain(self.cursor..end);
    }

    fn delete(&mut self) {
        if let Some(grapheme) = self.text[self.cursor..].graphemes(true).next() {
            self.text.drain(self.cursor..self.cursor + grapheme.len());
        }
    }

    fn visible(&self, width: usize) -> (String, u16) {
        if width == 0 {
            return (String::new(), 0);
        }
        let mut start = 0;
        while self.text[start..self.cursor].width() >= width && start < self.cursor {
            start += self.text[start..].graphemes(true).next().unwrap().len();
        }
        let column = self.text[start..self.cursor].width() as u16;
        let mut visible = String::new();
        let mut used = 0;
        for grapheme in self.text[start..].graphemes(true) {
            let next = grapheme.width();
            if used + next > width {
                break;
            }
            visible.push_str(grapheme);
            used += next;
        }
        (visible, column)
    }
}

enum Status {
    Idle,
    Loading(u64),
    Result(LookupResult),
    Error(String),
    Cancelled,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Input,
    Source,
    Target,
    Results,
}

enum Action {
    None,
    Submit,
    Cancel,
    Quit,
}

struct App {
    input: Input,
    source: Option<Language>,
    target: Option<Language>,
    focus: Focus,
    status: Status,
    scroll: u16,
    generation: u64,
}

impl App {
    fn new(source: Option<Language>, target: Option<Language>) -> Self {
        Self {
            input: Input::default(),
            source,
            target,
            focus: Focus::Input,
            status: Status::Idle,
            scroll: 0,
            generation: 0,
        }
    }

    fn begin(&mut self) -> (u64, LookupRequest) {
        self.generation += 1;
        self.status = Status::Loading(self.generation);
        self.scroll = 0;
        (
            self.generation,
            LookupRequest {
                query: self.input.text.clone(),
                from: self.source,
                to: self.target,
            },
        )
    }

    fn complete(&mut self, id: u64, result: Result<LookupResult, LookupError>) {
        if matches!(self.status, Status::Loading(active) if active == id) {
            self.status = match result {
                Ok(result) => Status::Result(result),
                Err(error) => Status::Error(safe_text(&error.to_string())),
            };
        }
    }

    fn event(&mut self, event: Event) -> Action {
        if let Event::Paste(text) = &event {
            if self.focus == Focus::Input {
                if text.chars().any(char::is_control) {
                    self.status = Status::Error(
                        "Paste must contain a single query without control characters.".into(),
                    );
                    return Action::Cancel;
                }
                self.input.insert(&safe_text(text));
            }
            return Action::None;
        }
        let Event::Key(key) = event else {
            return Action::None;
        };
        if key.kind == KeyEventKind::Release {
            return Action::None;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Action::Quit;
        }
        match key.code {
            KeyCode::Esc => {
                if matches!(self.status, Status::Loading(_)) {
                    self.status = Status::Cancelled;
                    return Action::Cancel;
                }
                return Action::Quit;
            }
            KeyCode::Enter => return Action::Submit,
            KeyCode::Tab | KeyCode::BackTab => {
                let order = [Focus::Input, Focus::Source, Focus::Target, Focus::Results];
                let index = order.iter().position(|focus| *focus == self.focus).unwrap();
                let delta = if key.code == KeyCode::BackTab
                    || key.modifiers.contains(KeyModifiers::SHIFT)
                {
                    3
                } else {
                    1
                };
                self.focus = order[(index + delta) % order.len()];
            }
            _ => match self.focus {
                Focus::Input => match key.code {
                    KeyCode::Char(c)
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                            && !c.is_control() =>
                    {
                        self.input.insert(&safe_text(&c.to_string()))
                    }
                    KeyCode::Left => self.input.left(),
                    KeyCode::Right => self.input.right(),
                    KeyCode::Home => self.input.cursor = 0,
                    KeyCode::End => self.input.cursor = self.input.text.len(),
                    KeyCode::Backspace => self.input.backspace(),
                    KeyCode::Delete => self.input.delete(),
                    _ => {}
                },
                Focus::Source | Focus::Target => {
                    let delta = match key.code {
                        KeyCode::Right | KeyCode::Down => 1,
                        KeyCode::Left | KeyCode::Up => 2,
                        _ => 0,
                    };
                    let choices = [None, Some(Language::German), Some(Language::English)];
                    let selected = if self.focus == Focus::Source {
                        &mut self.source
                    } else {
                        &mut self.target
                    };
                    let index = choices.iter().position(|value| value == selected).unwrap();
                    *selected = choices[(index + delta) % choices.len()];
                }
                Focus::Results => match key.code {
                    KeyCode::Up => self.scroll = self.scroll.saturating_sub(1),
                    KeyCode::Down => self.scroll = self.scroll.saturating_add(1),
                    KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(5),
                    KeyCode::PageDown => self.scroll = self.scroll.saturating_add(5),
                    KeyCode::Home => self.scroll = 0,
                    _ => {}
                },
            },
        }
        Action::None
    }

    fn lines(&self) -> Vec<String> {
        match &self.status {
            Status::Idle => vec!["Enter a word to look up.".into()],
            Status::Loading(_) => vec!["Looking up… Esc cancels.".into()],
            Status::Result(result) => result_lines(result),
            Status::Error(error) => vec![error.clone()],
            Status::Cancelled => {
                vec!["Lookup cancelled. Edit the word or press Enter to retry.".into()]
            }
        }
    }

    fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        if area.width < 24 || area.height < 12 {
            frame.render_widget(
                Paragraph::new("Resize terminal (24×12 minimum). Esc / Ctrl-C exits.")
                    .wrap(Wrap { trim: false }),
                area,
            );
            return;
        }
        let rows = Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(3),
        ])
        .split(area);
        let input_block = self.block(" Word ", Focus::Input);
        let inner = input_block.inner(rows[0]);
        let (visible, cursor) = self.input.visible(inner.width as usize);
        frame.render_widget(Paragraph::new(visible).block(input_block), rows[0]);
        if self.focus == Focus::Input {
            frame.set_cursor_position((inner.x + cursor, inner.y));
        }
        let languages =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(rows[1]);
        frame.render_widget(
            Paragraph::new(self.source.map_or("Auto", Language::code))
                .block(self.block(" Source ", Focus::Source)),
            languages[0],
        );
        frame.render_widget(
            Paragraph::new(self.target.map_or("Default", Language::code))
                .block(self.block(" Target ", Focus::Target)),
            languages[1],
        );
        let result_block = self.block(" Lookup ", Focus::Results);
        let inner = result_block.inner(rows[2]);
        let content = self.lines().join("\n");
        let wrapped = textwrap::fill(&content, inner.width as usize);
        let max_scroll = wrapped
            .lines()
            .count()
            .saturating_sub(inner.height as usize)
            .min(u16::MAX as usize) as u16;
        let paragraph = Paragraph::new(wrapped);
        self.scroll = self.scroll.min(max_scroll);
        frame.render_widget(
            paragraph.scroll((self.scroll, 0)).block(result_block),
            rows[2],
        );
        frame.render_widget(Paragraph::new("Enter lookup · Tab focus · Arrows edit/select/scroll\nEsc cancel/exit · Ctrl-C exit").wrap(Wrap { trim: false }), rows[3]);
    }

    fn block(&self, title: &'static str, focus: Focus) -> Block<'static> {
        Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(if self.focus == focus {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default()
            })
    }
}

pub async fn run<P: DictionaryProvider + 'static>(
    service: Arc<LookupService<P>>,
    from: Option<Language>,
    to: Option<Language>,
) -> io::Result<()> {
    check_terminal()?;
    let _guard = TerminalGuard;
    let mut terminal = ratatui::try_init()?;
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(io::stdout(), DisableBracketedPaste);
        previous_hook(info);
    }));
    execute!(io::stdout(), EnableBracketedPaste)?;
    let mut events = EventStream::new();
    let mut app = App::new(from, to);
    let mut pending: Option<tokio::task::JoinHandle<(u64, Result<LookupResult, LookupError>)>> =
        None;
    let outcome = loop {
        if let Err(error) = terminal.draw(|frame| app.draw(frame)) {
            break Err(error);
        }
        tokio::select! {
            event = events.next() => {
                let action = match event {
                    Some(Ok(event)) => app.event(event),
                    Some(Err(error)) => break Err(error),
                    None => break Ok(()),
                };
                match action {
                    Action::Submit => {
                        if let Some(task) = pending.take() { task.abort(); }
                        let (id, request) = app.begin();
                        let service = Arc::clone(&service);
                        pending = Some(tokio::spawn(async move { (id, service.lookup(request).await) }));
                    },
                    Action::Cancel => { if let Some(task) = pending.take() { task.abort(); } },
                    Action::Quit => break Ok(()),
                    Action::None => {},
                }
            },
            result = async { pending.as_mut().unwrap().await }, if pending.is_some() => {
                pending = None;
                match result {
                    Ok((id, result)) => app.complete(id, result),
                    Err(_) => break Err(io::Error::other("Lookup task failed.")),
                }
            },
            _ = tokio::signal::ctrl_c() => break Ok(()),
        }
    };
    if let Some(task) = pending {
        task.abort();
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEvent;
    use ratatui::{Terminal, backend::TestBackend};

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn unicode_editing_keeps_graphemes_and_cursor_valid() {
        let mut input = Input::default();
        input.insert("äe\u{301}猫");
        input.left();
        input.backspace();
        assert_eq!(input.text, "ä猫");
        input.delete();
        assert_eq!(input.text, "ä");
        input.insert("ß");
        assert_eq!(input.text, "äß");
        assert_eq!(input.visible(2), ("ß".into(), 1));
    }

    #[test]
    fn focus_paste_language_and_quit() {
        let mut app = App::new(None, None);
        assert!(matches!(app.event(key(KeyCode::Char('q'))), Action::None));
        app.event(Event::Paste("ä".into()));
        assert_eq!(app.input.text, "qä");
        app.event(key(KeyCode::Tab));
        app.event(key(KeyCode::Right));
        assert_eq!(app.source, Some(Language::German));
        app.event(key(KeyCode::Tab));
        app.event(key(KeyCode::Left));
        assert_eq!(app.target, Some(Language::English));
        assert!(matches!(app.event(key(KeyCode::Esc)), Action::Quit));
    }

    #[test]
    fn cancellation_and_stale_completions_preserve_input() {
        let mut app = App::new(None, None);
        app.input.insert("Gift");
        let (first, _) = app.begin();
        let (second, _) = app.begin();
        app.complete(first, Err(LookupError::Network));
        assert!(matches!(app.status, Status::Loading(id) if id == second));
        assert!(matches!(app.event(key(KeyCode::Esc)), Action::Cancel));
        app.complete(second, Err(LookupError::Network));
        assert!(matches!(app.status, Status::Cancelled));
        assert_eq!(app.input.text, "Gift");
        assert!(matches!(app.event(key(KeyCode::Enter)), Action::Submit));
        let (third, _) = app.begin();
        app.complete(third, Err(LookupError::Network));
        assert!(matches!(app.status, Status::Error(_)));
    }

    #[test]
    fn views_render_at_small_sizes_and_show_loading_errors() {
        let mut app = App::new(None, None);
        for (width, height) in [(80, 24), (24, 12), (8, 3), (1, 1)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            for status in [
                Status::Idle,
                Status::Loading(1),
                Status::Error("Network failure".into()),
                Status::Cancelled,
            ] {
                app.status = status;
                terminal.draw(|frame| app.draw(frame)).unwrap();
                let screen: String = terminal
                    .backend()
                    .buffer()
                    .content()
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect();
                if width >= 24 {
                    assert!(screen.contains("Source"));
                    if matches!(app.status, Status::Loading(_)) {
                        assert!(screen.contains("Looking up"));
                    }
                    if matches!(app.status, Status::Error(_)) {
                        assert!(screen.contains("Network failure"));
                    }
                }
            }
        }
    }

    #[test]
    fn result_view_uses_shared_rendering_and_preserves_language_selections() {
        let mut app = App::new(Some(Language::German), Some(Language::English));
        app.input.insert("Verbindlichkeit");
        let (id, request) = app.begin();
        app.complete(
            id,
            Ok(LookupResult {
                query: request.query.clone(),
                headword: request.query.clone(),
                normalized_headword: "verbindlichkeit".into(),
                pair: INITIAL_PAIRS[0],
                candidates: vec![TranslationCandidate {
                    text: "liability".into(),
                    normalized: "liability".into(),
                    part_of_speech: Some("NOUN".into()),
                    sense: None,
                    prefix: String::new(),
                    back_translations: vec![],
                }],
                provider: "fixture".into(),
                attribution: None,
                kind: ResultKind::Dictionary,
            }),
        );
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let screen: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(screen.contains("1. liability"));
        assert!(screen.contains("de → en"));
        let (_, next) = app.begin();
        assert_eq!(next.from, request.from);
        assert_eq!(next.to, request.to);
        assert_eq!(next.query, request.query);
    }
}
