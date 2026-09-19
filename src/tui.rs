use crate::{
    clipboard::{Clipboard, DesktopClipboard},
    coordinator::{Completion, Coordinator},
    domain::*,
    history::{Cursor, HistoryEntry, HistoryFilter, HistoryPage, display_time},
    keybindings::{Action, Keybindings, Resolver},
    presentation::safe_text,
};
use crossterm::{
    event::{
        DisableBracketedPaste, EnableBracketedPaste, Event, EventStream, KeyCode, KeyEvent,
        KeyEventKind, KeyModifiers,
    },
    execute,
};
use futures_util::StreamExt;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use std::{
    io::{self, IsTerminal},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, watch};
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tab {
    Lookup,
    History,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Focus {
    Input,
    Source,
    Target,
    Details,
    Recent,
    History,
}
struct Dialog {
    text: Input,
    today: bool,
    field: usize,
}
enum Effect {
    None,
    Submit,
    Cancel,
    Quit,
    Read {
        cursor: Option<Cursor>,
        oldest: bool,
        last: bool,
        preserve: bool,
    },
    Copy(String),
}
struct App {
    tab: Tab,
    focus: Focus,
    editing: bool,
    input: Input,
    source: Option<Language>,
    target: Option<Language>,
    live: Option<LookupResult>,
    problem: Option<String>,
    loading: bool,
    generation: u64,
    recent: Vec<HistoryEntry>,
    recent_state: ListState,
    preview: Option<HistoryEntry>,
    entries: Vec<HistoryEntry>,
    history_state: ListState,
    filter: HistoryFilter,
    dialog: Option<Dialog>,
    candidate: ListState,
    history_error: Option<String>,
    notice: String,
    bindings: Keybindings,
    resolver: Resolver,
}
impl App {
    fn new(from: Option<Language>, to: Option<Language>, bindings: Keybindings) -> Self {
        Self {
            tab: Tab::Lookup,
            focus: Focus::Input,
            editing: true,
            input: Input::default(),
            source: from,
            target: to,
            live: None,
            problem: None,
            loading: false,
            generation: 0,
            recent: vec![],
            recent_state: ListState::default(),
            preview: None,
            entries: vec![],
            history_state: ListState::default(),
            filter: HistoryFilter::default(),
            dialog: None,
            candidate: ListState::default(),
            history_error: None,
            notice: String::new(),
            bindings,
            resolver: Resolver::default(),
        }
    }
    fn selected(&self) -> Option<&HistoryEntry> {
        if self.tab == Tab::Lookup {
            self.preview.as_ref()
        } else {
            self.history_state
                .selected()
                .and_then(|i| self.entries.get(i))
        }
    }
    fn result(&self) -> Option<&LookupResult> {
        if self.tab == Tab::Lookup && self.preview.is_none() {
            self.live.as_ref()
        } else {
            self.selected().and_then(HistoryEntry::result)
        }
    }
    fn begin(&mut self) -> (u64, LookupRequest) {
        self.generation += 1;
        self.loading = true;
        self.preview = None;
        self.problem = None;
        (
            self.generation,
            LookupRequest {
                query: self.input.text.clone(),
                from: self.source,
                to: self.target,
            },
        )
    }
    fn complete(&mut self, id: u64, completion: Completion) {
        if id != self.generation {
            return;
        }
        self.loading = false;
        self.notice = completion.warnings.join(" · ");
        match completion.result {
            Ok(result) => {
                self.live = Some(result);
                self.problem = None;
                self.candidate.select(Some(0));
            }
            Err(error) => {
                self.live = None;
                self.problem = Some(error.to_string());
            }
        }
    }
    fn refresh(&self) -> Effect {
        let cursor = self
            .history_state
            .selected()
            .and_then(|i| self.entries.get(i))
            .map(|e| Cursor {
                timestamp: e.started_at,
                sequence: e.sequence.saturating_add(1),
            });
        Effect::Read {
            cursor,
            oldest: false,
            last: false,
            preserve: true,
        }
    }
    fn escape(&mut self) -> Effect {
        if self.dialog.take().is_some() {
            self.resolver.reset();
            return Effect::None;
        }
        if self.resolver.pending() {
            self.resolver.reset();
            return Effect::None;
        }
        if self.loading {
            self.generation += 1;
            self.loading = false;
            self.problem = Some("Lookup cancelled.".into());
            return Effect::Cancel;
        }
        if self.editing {
            self.editing = false;
        } else {
            self.preview = None;
            self.candidate.select(Some(0));
        }
        Effect::None
    }
    fn event(&mut self, event: Event) -> Effect {
        if let Event::Paste(text) = event {
            if text.chars().any(char::is_control) {
                self.notice = "Paste must contain a single line without control characters.".into();
                return Effect::None;
            }
            if let Some(dialog) = &mut self.dialog {
                if dialog.field == 0 {
                    dialog.text.insert(&safe_text(&text));
                }
            } else if self.editing && self.focus == Focus::Input {
                self.input.insert(&safe_text(&text));
            }
            return Effect::None;
        }
        let Event::Key(key) = event else {
            return Effect::None;
        };
        if key.kind == KeyEventKind::Release {
            return Effect::None;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Effect::Quit;
        }
        let text_editing = (self.editing && self.focus == Focus::Input)
            || self.dialog.as_ref().is_some_and(|d| d.field == 0);
        if key.code == KeyCode::Esc
            || ((!text_editing || command_key(key)) && self.bindings.direct(key, Action::Cancel))
        {
            return self.escape();
        }
        if self.dialog.is_some() {
            return self.dialog_event(key);
        }
        if self.editing && self.focus == Focus::Input {
            if command_key(key) && self.bindings.direct(key, Action::Submit) {
                return Effect::Submit;
            }
            if command_key(key) && self.bindings.direct(key, Action::NextFocus) {
                self.move_focus(1);
                return Effect::None;
            }
            if command_key(key) && self.bindings.direct(key, Action::PreviousFocus) {
                self.move_focus(-1);
                return Effect::None;
            }
            edit(&mut self.input, key);
            return Effect::None;
        }
        let Some((action, pane)) = self.resolver.feed(
            &self.bindings,
            key,
            self.tab == Tab::History,
            Instant::now(),
        ) else {
            return Effect::None;
        };
        if pane {
            let focus = if self.tab == Tab::History {
                if matches!(action, Action::Left | Action::Up) {
                    Focus::History
                } else {
                    Focus::Details
                }
            } else {
                match (self.focus, action) {
                    (Focus::Source, Action::Right) => Focus::Target,
                    (Focus::Target, Action::Left) => Focus::Source,
                    (Focus::Input, Action::Down) => Focus::Source,
                    (Focus::Source | Focus::Target, Action::Up) => Focus::Input,
                    (Focus::Source | Focus::Target, Action::Down) => Focus::Details,
                    (Focus::Details, Action::Up) => Focus::Source,
                    (Focus::Details, Action::Down) => Focus::Recent,
                    (Focus::Recent, Action::Up) => Focus::Details,
                    _ => self.focus,
                }
            };
            self.set_focus(focus);
            return Effect::None;
        }
        match action {
            Action::Cancel => self.escape(),
            Action::Quit => Effect::Quit,
            Action::NextTab | Action::PreviousTab => {
                self.resolver.reset();
                self.editing = false;
                self.tab = if self.tab == Tab::Lookup {
                    Tab::History
                } else {
                    Tab::Lookup
                };
                self.focus = if self.tab == Tab::Lookup {
                    Focus::Input
                } else {
                    Focus::History
                };
                self.candidate.select(Some(0));
                if self.tab == Tab::History {
                    self.refresh()
                } else {
                    Effect::None
                }
            }
            Action::Edit => {
                self.focus = Focus::Input;
                self.editing = true;
                self.preview = None;
                self.resolver.reset();
                Effect::None
            }
            Action::Submit => {
                if self.tab == Tab::Lookup
                    && matches!(self.focus, Focus::Input | Focus::Source | Focus::Target)
                {
                    Effect::Submit
                } else {
                    self.focus = Focus::Details;
                    Effect::None
                }
            }
            Action::NextFocus => {
                self.move_focus(1);
                Effect::None
            }
            Action::PreviousFocus => {
                self.move_focus(-1);
                Effect::None
            }
            Action::Filter => {
                let mut text = Input::default();
                text.insert(&self.filter.text);
                self.dialog = Some(Dialog {
                    text,
                    today: self.filter.today,
                    field: 0,
                });
                self.resolver.reset();
                Effect::None
            }
            Action::Refresh => self.refresh(),
            Action::CopyQuery | Action::CopyAll | Action::CopyValue => {
                let value = if action == Action::CopyQuery {
                    self.selected()
                        .map(|e| safe_text(&e.query))
                        .or_else(|| self.result().map(|r| safe_text(&r.query)))
                } else if action == Action::CopyValue && self.focus != Focus::Details {
                    None
                } else {
                    self.result().and_then(|r| {
                        crate::clipboard::values(
                            r,
                            if action == Action::CopyValue {
                                Some(self.candidate.selected().unwrap_or(0))
                            } else {
                                None
                            },
                        )
                    })
                };
                if let Some(value) = value {
                    Effect::Copy(value)
                } else {
                    self.notice =
                        "No selected value to copy; focus details to copy one translation.".into();
                    Effect::None
                }
            }
            _ => self.navigate(action),
        }
    }
    fn move_focus(&mut self, delta: isize) {
        let order: &[Focus] = if self.tab == Tab::History {
            &[Focus::History, Focus::Details]
        } else {
            &[
                Focus::Input,
                Focus::Source,
                Focus::Target,
                Focus::Details,
                Focus::Recent,
            ]
        };
        let i = order.iter().position(|f| *f == self.focus).unwrap_or(0);
        self.set_focus(order[(i as isize + delta).rem_euclid(order.len() as isize) as usize]);
    }
    fn set_focus(&mut self, focus: Focus) {
        self.focus = focus;
        self.editing = false;
        self.resolver.reset();
        if self.focus == Focus::Recent && !self.recent.is_empty() {
            self.recent_state
                .select(Some(self.recent_state.selected().unwrap_or(0)));
            self.preview = self
                .recent
                .get(self.recent_state.selected().unwrap_or(0))
                .cloned();
            self.candidate.select(Some(0));
        }
    }
    fn navigate(&mut self, action: Action) -> Effect {
        if matches!(self.focus, Focus::Source | Focus::Target) {
            let selected = if self.focus == Focus::Source {
                &mut self.source
            } else {
                &mut self.target
            };
            let options = [None, Some(Language::German), Some(Language::English)];
            let i = options.iter().position(|v| v == selected).unwrap();
            let delta = match action {
                Action::Right | Action::Down => 1,
                Action::Left | Action::Up => 2,
                _ => 0,
            };
            *selected = options[(i + delta) % 3];
            return Effect::None;
        }
        if self.focus == Focus::History && matches!(action, Action::Home | Action::End) {
            return Effect::Read {
                cursor: None,
                oldest: action == Action::End,
                last: action == Action::End,
                preserve: false,
            };
        }
        let count = if self.focus == Focus::Details {
            self.result().map_or(0, |r| r.candidates.len())
        } else if self.focus == Focus::Recent {
            self.recent.len()
        } else if self.focus == Focus::History {
            self.entries.len()
        } else {
            0
        };
        if count == 0 {
            return Effect::None;
        }
        let state = match self.focus {
            Focus::Details => &mut self.candidate,
            Focus::Recent => &mut self.recent_state,
            Focus::History => &mut self.history_state,
            _ => return Effect::None,
        };
        let i = state.selected().unwrap_or(0);
        if self.focus == Focus::History {
            if matches!(action, Action::Down | Action::PageDown) && i + 1 >= count {
                return Effect::Read {
                    cursor: self.entries.last().map(HistoryEntry::cursor),
                    oldest: false,
                    last: false,
                    preserve: false,
                };
            }
            if matches!(action, Action::Up | Action::PageUp) && i == 0 {
                return Effect::Read {
                    cursor: self.entries.first().map(HistoryEntry::cursor),
                    oldest: true,
                    last: true,
                    preserve: false,
                };
            }
        }
        let next = match action {
            Action::Up => i.saturating_sub(1),
            Action::Down => (i + 1).min(count - 1),
            Action::PageUp => i.saturating_sub(5),
            Action::PageDown => (i + 5).min(count - 1),
            Action::Home => 0,
            Action::End => count - 1,
            _ => i,
        };
        state.select(Some(next));
        if self.focus == Focus::Recent {
            self.preview = self.recent.get(next).cloned();
            self.candidate.select(Some(0));
        }
        if self.focus == Focus::History {
            self.candidate.select(Some(0));
        }
        Effect::None
    }
    fn dialog_event(&mut self, key: KeyEvent) -> Effect {
        let d = self.dialog.as_mut().unwrap();
        if command_key(key) && self.bindings.direct(key, Action::NextFocus) {
            d.field = (d.field + 1) % 5;
            return Effect::None;
        }
        if command_key(key) && self.bindings.direct(key, Action::PreviousFocus) {
            d.field = (d.field + 4) % 5;
            return Effect::None;
        }
        if command_key(key) && self.bindings.direct(key, Action::Submit) {
            if d.field == 3 {
                d.text = Input::default();
                d.today = false;
                return Effect::None;
            }
            if d.field == 4 {
                self.dialog = None;
                return Effect::None;
            }
            self.filter = HistoryFilter {
                text: d.text.text.clone(),
                today: d.today,
            };
            self.dialog = None;
            self.entries.clear();
            self.history_state.select(None);
            self.candidate.select(None);
            return Effect::Read {
                cursor: None,
                oldest: false,
                last: false,
                preserve: false,
            };
        }
        if d.field == 0 {
            edit(&mut d.text, key);
        } else if d.field == 1
            && (matches!(
                key.code,
                KeyCode::Left | KeyCode::Right | KeyCode::Char(' ')
            ) || self.bindings.direct(key, Action::Left)
                || self.bindings.direct(key, Action::Right))
        {
            d.today = !d.today;
        }
        Effect::None
    }
    fn block(&self, title: &str, focus: Focus) -> Block<'static> {
        Block::default()
            .title(title.to_owned())
            .borders(Borders::ALL)
            .border_style(if self.focus == focus {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default()
            })
    }
    fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        if area.width < 24 || area.height < 12 {
            frame.render_widget(
                Paragraph::new("Resize terminal (24×12 minimum). Ctrl-C exits."),
                area,
            );
            return;
        }
        let rows = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(3),
        ])
        .split(area);
        frame.render_widget(
            Paragraph::new(if self.tab == Tab::Lookup {
                "[ lookup ]    history"
            } else {
                "lookup    [ history ]"
            })
            .style(Style::default().fg(Color::Cyan)),
            rows[0],
        );
        if self.tab == Tab::Lookup {
            self.draw_lookup(frame, rows[1]);
        } else {
            self.draw_history(frame, rows[1]);
        }
        let mode = if self.editing { "INSERT" } else { "NORMAL" };
        let help = format!(
            "{mode} · {} tab · {} pane · {} up / {} down
{} filter · {} query · {} value · {} all · {} home · {} end
{}",
            self.bindings.label(Action::NextTab),
            self.bindings.label(Action::NextFocus),
            self.bindings.label(Action::Up),
            self.bindings.label(Action::Down),
            self.bindings.label(Action::Filter),
            self.bindings.label(Action::CopyQuery),
            self.bindings.label(Action::CopyValue),
            self.bindings.label(Action::CopyAll),
            self.bindings.label(Action::Home),
            self.bindings.label(Action::End),
            safe_text(&self.notice)
        );
        frame.render_widget(Paragraph::new(help), rows[2]);
        if self.dialog.is_some() {
            self.draw_dialog(frame, area);
        }
    }
    fn draw_lookup(&mut self, frame: &mut Frame, area: Rect) {
        let recent_height = if area.height >= 17 {
            7
        } else if area.height >= 13 {
            4
        } else {
            0
        };
        let rows = Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(2),
            Constraint::Length(recent_height),
        ])
        .split(area);
        let block = self.block(" Word ", Focus::Input);
        let inner = block.inner(rows[0]);
        let (visible, cursor) = self.input.visible(inner.width as usize);
        frame.render_widget(Paragraph::new(visible).block(block), rows[0]);
        if self.editing && self.focus == Focus::Input && self.dialog.is_none() {
            frame.set_cursor_position((inner.x + cursor, inner.y));
        }
        let cols = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(rows[1]);
        frame.render_widget(
            Paragraph::new(self.source.map_or("Auto", Language::code))
                .block(self.block(" Source ", Focus::Source)),
            cols[0],
        );
        frame.render_widget(
            Paragraph::new(self.target.map_or("Default", Language::code))
                .block(self.block(" Target ", Focus::Target)),
            cols[1],
        );
        self.draw_details(frame, rows[2]);
        if recent_height > 0 {
            let items = self
                .recent
                .iter()
                .map(|e| {
                    ListItem::new(format!(
                        "{} · {} · {}",
                        safe_text(&e.query),
                        display_time(e.started_at),
                        e.status()
                    ))
                })
                .collect::<Vec<_>>();
            if items.is_empty() {
                frame.render_widget(
                    Paragraph::new(
                        self.history_error
                            .as_deref()
                            .unwrap_or("No saved lookups yet."),
                    )
                    .wrap(Wrap { trim: false })
                    .block(self.block(" Recent lookups ", Focus::Recent)),
                    rows[3],
                );
            } else {
                frame.render_stateful_widget(
                    List::new(items)
                        .block(self.block(" Recent lookups ", Focus::Recent))
                        .highlight_symbol("> ")
                        .highlight_style(Style::default().fg(Color::Cyan)),
                    rows[3],
                    &mut self.recent_state,
                );
            }
        }
    }
    fn draw_history(&mut self, frame: &mut Frame, area: Rect) {
        let rows = Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).split(area);
        frame.render_widget(
            Paragraph::new(format!(
                "Filter: {} · {}",
                if self.filter.text.is_empty() {
                    "all text"
                } else {
                    &self.filter.text
                },
                if self.filter.today {
                    "today"
                } else {
                    "all history"
                }
            ))
            .wrap(Wrap { trim: false }),
            rows[0],
        );
        let panes = if area.width >= 100 {
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(rows[1])
        } else if area.height >= 20 {
            Layout::vertical([Constraint::Percentage(45), Constraint::Percentage(55)])
                .split(rows[1])
        } else {
            std::rc::Rc::from([rows[1], rows[1]])
        };
        let single = area.width < 100 && area.height < 20;
        if !single || self.focus == Focus::History {
            let block = self.block(" Encounters · newest first ", Focus::History);
            if self.entries.is_empty() {
                let text = self.history_error.clone().unwrap_or_else(|| {
                    if self.filter.text.is_empty() && !self.filter.today {
                        "No saved lookups yet.".into()
                    } else {
                        "No matches. Open filters to adjust or clear them.".into()
                    }
                });
                frame.render_widget(
                    Paragraph::new(text).wrap(Wrap { trim: false }).block(block),
                    panes[0],
                );
            } else {
                let items = self
                    .entries
                    .iter()
                    .map(|e| {
                        let values = e
                            .result()
                            .map(|r| {
                                r.candidates
                                    .iter()
                                    .take(2)
                                    .map(|c| safe_text(&c.text))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            })
                            .unwrap_or_default();
                        ListItem::new(format!(
                            "{} · {}\n{} → {} · {} · {}\n{}",
                            safe_text(&e.query),
                            e.status(),
                            e.from.map_or("?", Language::code),
                            e.to.map_or("?", Language::code),
                            safe_text(e.provider.as_deref().unwrap_or("unknown provider")),
                            display_time(e.started_at),
                            values
                        ))
                    })
                    .collect::<Vec<_>>();
                frame.render_stateful_widget(
                    List::new(items)
                        .block(block)
                        .highlight_symbol("> ")
                        .highlight_style(Style::default().fg(Color::Cyan)),
                    panes[0],
                    &mut self.history_state,
                );
            }
        }
        if !single || self.focus == Focus::Details {
            self.draw_details(frame, panes[1]);
        }
    }
    fn draw_details(&mut self, frame: &mut Frame, area: Rect) {
        let entry = self.selected().cloned();
        let result = self.result().cloned();
        let title = if entry.is_some() {
            " Saved encounter "
        } else {
            " Lookup "
        };
        let block = self.block(title, Focus::Details);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if self.tab == Tab::Lookup && self.preview.is_none() {
            if self.loading {
                frame.render_widget(Paragraph::new("Looking up… Esc cancels."), inner);
                return;
            }
            if let Some(problem) = &self.problem {
                frame.render_widget(
                    Paragraph::new(safe_text(problem)).wrap(Wrap { trim: false }),
                    inner,
                );
                return;
            }
        }
        let Some(result) = result else {
            let text = entry
                .map(|e| {
                    format!(
                        "{}\n{}\nStarted {}\n{}",
                        e.query,
                        e.status(),
                        display_time(e.started_at),
                        e.finished.and_then(|f| f.message).unwrap_or_default()
                    )
                })
                .unwrap_or_else(|| "Enter a word to look up.".into());
            frame.render_widget(
                Paragraph::new(text.lines().map(safe_text).collect::<Vec<_>>().join("\n"))
                    .wrap(Wrap { trim: false }),
                inner,
            );
            return;
        };
        let metadata = entry
            .as_ref()
            .map(|e| {
                format!(
                    "Saved {} · finished {}",
                    display_time(e.started_at),
                    e.finished_at
                        .map(display_time)
                        .unwrap_or_else(|| "unknown".into())
                )
            })
            .unwrap_or_default();
        let parts = Layout::vertical([
            Constraint::Length(if entry.is_some() { 3 } else { 1 }),
            Constraint::Min(1),
            Constraint::Length(if inner.height > 5 { 2 } else { 0 }),
        ])
        .split(inner);
        frame.render_widget(
            Paragraph::new(format!(
                "{} · {} · {}\n{}",
                safe_text(&result.headword),
                result.pair,
                safe_text(&result.provider),
                metadata
            ))
            .wrap(Wrap { trim: false }),
            parts[0],
        );
        let items = result
            .candidates
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let value = crate::clipboard::values(&result, Some(i)).unwrap_or_default();
                let sense = c
                    .sense
                    .as_ref()
                    .or(c.back_translations.first())
                    .map(|s| format!(" · {}", safe_text(s)))
                    .unwrap_or_default();
                let pos = c
                    .part_of_speech
                    .as_ref()
                    .map(|s| format!(" ({})", safe_text(s)))
                    .unwrap_or_default();
                let text = textwrap::fill(
                    &format!("{}. {value}{pos}{sense}", i + 1),
                    parts[1].width.saturating_sub(2).max(1) as usize,
                );
                ListItem::new(text)
            })
            .collect::<Vec<_>>();
        if self.candidate.selected().is_none() {
            self.candidate.select(Some(0));
        }
        frame.render_stateful_widget(
            List::new(items)
                .highlight_symbol("> ")
                .highlight_style(Style::default().fg(Color::Cyan)),
            parts[1],
            &mut self.candidate,
        );
        frame.render_widget(
            Paragraph::new(safe_text(result.attribution.as_deref().unwrap_or("")))
                .wrap(Wrap { trim: false }),
            parts[2],
        );
    }
    fn draw_dialog(&mut self, frame: &mut Frame, area: Rect) {
        let width = area.width.min(68);
        let height = area.height.min(12);
        let rect = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );
        frame.render_widget(Clear, rect);
        let block = Block::default()
            .title(" Filter history ")
            .borders(Borders::ALL);
        let inner = block.inner(rect);
        frame.render_widget(block, rect);
        let d = self.dialog.as_ref().unwrap();
        let marker = |field| if d.field == field { ">" } else { " " };
        let text = format!(
            "{} Text: {}\n\n{} Date: {}\n\n{} Apply    {} Clear    {} Cancel\n\nEnter apply · Tab field · Esc cancel",
            marker(0),
            safe_text(&d.text.text),
            marker(1),
            if d.today { "Today" } else { "All history" },
            marker(2),
            marker(3),
            marker(4)
        );
        frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), inner);
    }
}
fn command_key(key: KeyEvent) -> bool {
    !matches!(key.code, KeyCode::Char(_))
        || key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
}
fn edit(input: &mut Input, key: KeyEvent) {
    match key.code {
        KeyCode::Char(c)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                && !c.is_control() =>
        {
            input.insert(&safe_text(&c.to_string()))
        }
        KeyCode::Left => input.left(),
        KeyCode::Right => input.right(),
        KeyCode::Home => input.cursor = 0,
        KeyCode::End => input.cursor = input.text.len(),
        KeyCode::Backspace => input.backspace(),
        KeyCode::Delete => input.delete(),
        _ => {}
    }
}
enum Message {
    Lookup(u64, Completion),
    History(u64, bool, (bool, bool), Result<HistoryPage, String>),
}
fn read_history(
    jobs: &mut tokio::task::JoinSet<Message>,
    coordinator: &Coordinator,
    filter: HistoryFilter,
    cursor: Option<Cursor>,
    placement: (bool, bool, bool),
    generation: u64,
    recent: bool,
) {
    let (oldest, last, preserve) = placement;
    let store = coordinator.history.clone();
    jobs.spawn(async move {
        let result = match store {
            Ok(store) => {
                let mut page = store
                    .page(filter.clone(), cursor, if recent { 5 } else { 50 }, oldest)
                    .await;
                if preserve && cursor.is_some() && page.as_ref().is_ok_and(|p| p.entries.is_empty())
                {
                    page = store.page(filter, None, 50, false).await;
                }
                page.map_err(|e| e.to_string())
            }
            Err(e) => Err(e),
        };
        Message::History(generation, recent, (last, preserve), result)
    });
}
pub async fn run(
    coordinator: Arc<Coordinator>,
    from: Option<Language>,
    to: Option<Language>,
    bindings: Keybindings,
    warnings: Vec<String>,
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
    let mut app = App::new(from, to, bindings);
    app.notice = warnings.join(" · ");
    let mut clipboard = DesktopClipboard::default();
    let mut events = EventStream::new();
    let mut jobs = tokio::task::JoinSet::new();
    let mut cancellations = std::collections::HashMap::<u64, watch::Sender<bool>>::new();
    let (progress_tx, mut progress_rx) = mpsc::unbounded_channel::<String>();
    let mut read_generation = 0;
    let mut recent_generation = 0;
    read_history(
        &mut jobs,
        &coordinator,
        HistoryFilter::default(),
        None,
        (false, false, false),
        recent_generation,
        true,
    );
    let outcome = loop {
        if let Err(error) = terminal.draw(|frame| app.draw(frame)) {
            break Err(error);
        }
        let effect = tokio::select! {
            event=events.next()=>match event{Some(Ok(event))=>app.event(event),Some(Err(error))=>break Err(error),None=>Effect::Quit},
            _=tokio::signal::ctrl_c()=>Effect::Quit,
            Some(message)=progress_rx.recv()=>{app.notice=safe_text(&message);Effect::None},
            Some(result)=jobs.join_next(),if !jobs.is_empty()=>{
                match result {
                    Ok(Message::Lookup(id,completion))=>{
                        cancellations.remove(&id);
                        if id==app.generation{app.complete(id,completion);}else if !completion.warnings.is_empty(){app.notice=completion.warnings.join(" · ");}
                        recent_generation+=1;read_history(&mut jobs,&coordinator,HistoryFilter::default(),None,(false,false,false),recent_generation,true);
                        if app.tab==Tab::History{app.refresh()}else{Effect::None}
                    },
                    Ok(Message::History(generation,recent,(last,preserve),result))=>{
                        if (recent&&generation==recent_generation)||(!recent&&generation==read_generation){
                            match result {
                                Ok(page)=>{
                                    app.history_error=None;
                                    if recent{
                                        let selected=app.recent_state.selected().and_then(|i|app.recent.get(i)).map(|e|e.id.clone());
                                        app.recent=page.entries;
                                        app.recent_state.select(selected.and_then(|id|app.recent.iter().position(|e|e.id==id)));
                                        if let Some(preview)=&app.preview && let Some(updated)=app.recent.iter().find(|e|e.id==preview.id){app.preview=Some(updated.clone());}
                                    }else if !page.entries.is_empty()||app.entries.is_empty()||preserve{
                                        let selected=if preserve{app.history_state.selected().and_then(|i|app.entries.get(i)).map(|e|e.id.clone())}else{None};
                                        app.entries=page.entries;
                                        let i=selected.and_then(|id|app.entries.iter().position(|e|e.id==id)).unwrap_or(if last{app.entries.len().saturating_sub(1)}else{0});
                                        app.history_state.select(if app.entries.is_empty(){None}else{Some(i)});app.candidate.select(Some(0));
                                    }
                                },
                                Err(error)=>{app.history_error=Some(error.clone());app.notice=error;if !recent{app.entries.clear();app.history_state.select(None);}}
                            }
                        }
                        Effect::None
                    },
                    Err(error)=>{app.notice=format!("Background task failed: {error}");Effect::None}
                }
            }
        };
        match effect {
            Effect::Submit => {
                for cancel in cancellations.values() {
                    let _ = cancel.send(true);
                }
                let (id, request) = app.begin();
                let (cancel, receiver) = watch::channel(false);
                cancellations.insert(id, cancel);
                let coordinator = Arc::clone(&coordinator);
                let progress = progress_tx.clone();
                jobs.spawn(async move {
                    Message::Lookup(id, coordinator.run(request, receiver, Some(progress)).await)
                });
            }
            Effect::Cancel => {
                for cancel in cancellations.values() {
                    let _ = cancel.send(true);
                }
            }
            Effect::Quit => break Ok(()),
            Effect::Read {
                cursor,
                oldest,
                last,
                preserve,
            } => {
                if cursor.is_none() {
                    app.entries.clear();
                    app.history_state.select(None);
                }
                read_generation += 1;
                read_history(
                    &mut jobs,
                    &coordinator,
                    app.filter.clone(),
                    cursor,
                    (oldest, last, preserve),
                    read_generation,
                    false,
                );
            }
            Effect::Copy(text) => {
                app.notice = match clipboard.copy(text) {
                    Ok(()) => "Copied to clipboard.".into(),
                    Err(error) => error,
                };
            }
            Effect::None => {}
        }
    };
    for cancel in cancellations.values() {
        let _ = cancel.send(true);
    }
    let mut shutdown_warnings = Vec::new();
    if tokio::time::timeout(Duration::from_secs(2), async {
        while let Some(result) = jobs.join_next().await {
            if let Ok(Message::Lookup(_, completion)) = result {
                shutdown_warnings.extend(completion.warnings);
            }
        }
    })
    .await
    .is_err()
    {
        shutdown_warnings
            .push("History shutdown timed out; an attempt may remain unfinished.".into());
    }
    jobs.abort_all();
    drop(terminal);
    drop(_guard);
    for warning in shutdown_warnings {
        eprintln!("{}", safe_text(&warning));
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    fn key(c: char) -> Event {
        Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }
    #[test]
    fn typing_shortcuts_and_graphemes_does_not_navigate() {
        let mut app = App::new(None, None, Keybindings::default());
        for c in "qgtjkmnblG/".chars() {
            assert!(matches!(app.event(key(c)), Effect::None));
        }
        assert_eq!(app.input.text, "qgtjkmnblG/");
        assert_eq!(app.tab, Tab::Lookup);
        let mut input = Input::default();
        input.insert("äe\u{301}猫");
        input.left();
        input.backspace();
        assert_eq!(input.text, "ä猫");
        input.delete();
        assert_eq!(input.text, "ä");
    }
    #[test]
    fn tabs_filters_and_cancellation_preserve_draft() {
        let mut app = App::new(None, None, Keybindings::default());
        app.input.insert("word");
        let (old, _) = app.begin();
        app.escape();
        app.complete(
            old,
            Completion {
                result: Err(LookupError::Network),
                warnings: vec![],
            },
        );
        assert_eq!(app.problem.as_deref(), Some("Lookup cancelled."));
        app.escape();
        app.event(key('g'));
        assert!(matches!(app.event(key('t')), Effect::Read { .. }));
        assert_eq!(app.tab, Tab::History);
        app.event(key('/'));
        app.event(key('b'));
        app.escape();
        assert!(app.filter.text.is_empty());
        assert_eq!(app.input.text, "word");
    }
    #[test]
    fn recent_preview_copy_and_filter_apply_do_not_replace_the_draft() {
        let mut app = App::new(
            Some(Language::German),
            Some(Language::English),
            Keybindings::default(),
        );
        app.input.insert("unfinished word");
        app.recent.push(HistoryEntry {
            id: "old".into(),
            sequence: 1,
            query: "saved word".into(),
            from: Some(Language::German),
            to: Some(Language::English),
            provider: None,
            started_at: 0,
            finished_at: None,
            finished: None,
        });
        app.editing = false;
        app.focus = Focus::Details;
        app.move_focus(1);
        assert_eq!(app.preview.as_ref().unwrap().query, "saved word");
        assert_eq!(app.input.text, "unfinished word");
        app.event(key('y'));
        assert!(matches!(app.event(key('q')),Effect::Copy(text) if text=="saved word"));
        app.escape();
        assert!(app.preview.is_none());
        app.tab = Tab::History;
        app.focus = Focus::History;
        app.event(key('/'));
        app.event(key('x'));
        assert!(matches!(
            app.event(Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE
            ))),
            Effect::Read { .. }
        ));
        assert_eq!(app.filter.text, "x");
        assert!(app.dialog.is_none());
        assert_eq!(app.input.text, "unfinished word");
    }
    #[test]
    fn letter_remaps_never_steal_text_input() {
        let bindings =
            Keybindings::parse("[actions]\nsubmit=['s']\ncancel=['e']\nnext_focus=['f']").unwrap();
        let mut app = App::new(None, None, bindings);
        for c in "sef".chars() {
            assert!(matches!(app.event(key(c)), Effect::None));
        }
        assert_eq!(app.input.text, "sef");
    }
    #[test]
    fn pane_directions_follow_the_lookup_layout_without_wrapping() {
        let bindings = Keybindings::parse(include_str!(
            "../assets/keybindings/neo-noted.keybinding.toml"
        ))
        .unwrap();
        let mut app = App::new(None, None, bindings);
        app.escape();
        let mut direction = |key_code, expected| {
            app.event(Event::Key(KeyEvent::new(
                KeyCode::Char('w'),
                KeyModifiers::CONTROL,
            )));
            app.event(Event::Key(KeyEvent::new(key_code, KeyModifiers::NONE)));
            assert_eq!(app.focus, expected);
        };
        direction(KeyCode::Left, Focus::Input);
        direction(KeyCode::Char('n'), Focus::Source);
        direction(KeyCode::Char('r'), Focus::Target);
        direction(KeyCode::Down, Focus::Details);
        direction(KeyCode::Down, Focus::Recent);
        direction(KeyCode::Down, Focus::Recent);
        direction(KeyCode::Char('m'), Focus::Details);
        direction(KeyCode::Up, Focus::Source);
        direction(KeyCode::Up, Focus::Input);
    }
    #[test]
    fn adaptive_layouts_and_dialogs_render() {
        let mut app = App::new(None, None, Keybindings::default());
        for (width, height) in [(120, 32), (80, 24), (40, 18), (24, 12), (8, 3), (1, 1)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            for tab in [Tab::Lookup, Tab::History] {
                app.tab = tab;
                for focus in [Focus::History, Focus::Details] {
                    app.focus = focus;
                    terminal.draw(|f| app.draw(f)).unwrap();
                }
            }
            app.dialog = Some(Dialog {
                text: Input::default(),
                today: true,
                field: 0,
            });
            terminal.draw(|f| app.draw(f)).unwrap();
            app.dialog = None;
        }
    }
}
