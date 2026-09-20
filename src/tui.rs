mod input;
use input::{Input, InputEffect, InputMode, SelectionAction};

use crate::{
    clipboard::{Clipboard, DesktopClipboard},
    coordinator::{Completion, Coordinator},
    domain::*,
    history::{Cursor, HistoryEntry, HistoryFilter, HistoryPage, display_time},
    keybindings::{Action, Context, Keybindings, Resolver},
    presentation::safe_text,
};
use crossterm::{
    cursor::SetCursorStyle,
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
    text::Line,
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use std::{
    io::{self, IsTerminal},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, watch};
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
        let _ = execute!(
            io::stdout(),
            DisableBracketedPaste,
            SetCursorStyle::DefaultUserShape
        );
        ratatui::restore();
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
    CopySelection(String, SelectionAction),
    Paste,
}
impl From<InputEffect> for Effect {
    fn from(effect: InputEffect) -> Self {
        match effect {
            InputEffect::None => Self::None,
            InputEffect::Paste => Self::Paste,
            InputEffect::CopySelection(text, cut) => Self::CopySelection(text, cut),
        }
    }
}
struct App {
    tab: Tab,
    focus: Focus,
    pane_mode: bool,
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
    // Reading position within an oversized candidate, separate from the viewport.
    candidate_line: usize,
    // First visible wrapped line across all candidates.
    details_scroll: usize,
    details_viewport: (u16, u16),
    read_generation: u64,
    recent_generation: u64,
    history_error: Option<String>,
    recent_error: Option<String>,
    notice: String,
    bindings: Keybindings,
    resolver: Resolver,
}
impl App {
    fn new(from: Option<Language>, to: Option<Language>, bindings: Keybindings) -> Self {
        Self {
            tab: Tab::Lookup,
            focus: Focus::Input,
            pane_mode: false,
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
            candidate_line: 0,
            details_scroll: 0,
            details_viewport: (1, 1),
            read_generation: 0,
            recent_generation: 0,
            history_error: None,
            recent_error: None,
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
                query: self.input.text(),
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
                self.select_candidate(Some(0));
            }
            Err(error) => {
                self.live = None;
                self.problem = Some(error.to_string());
            }
        }
    }
    fn apply_history(
        &mut self,
        generation: u64,
        recent: bool,
        last: bool,
        preserve: bool,
        result: Result<HistoryPage, String>,
    ) {
        let expected = if recent {
            self.recent_generation
        } else {
            self.read_generation
        };
        if generation != expected {
            return;
        }
        let stored_error = if recent {
            &mut self.recent_error
        } else {
            &mut self.history_error
        };
        match result {
            Ok(page) => {
                if stored_error.as_deref() == Some(self.notice.as_str()) {
                    self.notice.clear();
                }
                *stored_error = None;
                if recent {
                    let selected = self
                        .recent_state
                        .selected()
                        .and_then(|i| self.recent.get(i))
                        .map(|entry| entry.id.clone());
                    self.recent = page.entries;
                    self.recent_state.select(
                        selected.and_then(|id| self.recent.iter().position(|entry| entry.id == id)),
                    );
                    if let Some(preview) = &self.preview
                        && let Some(updated) =
                            self.recent.iter().find(|entry| entry.id == preview.id)
                    {
                        self.preview = Some(updated.clone());
                    }
                } else if !page.entries.is_empty() || self.entries.is_empty() || preserve {
                    let selected = preserve
                        .then(|| {
                            self.history_state
                                .selected()
                                .and_then(|i| self.entries.get(i))
                                .map(|entry| entry.id.clone())
                        })
                        .flatten();
                    self.entries = page.entries;
                    let index = selected
                        .and_then(|id| self.entries.iter().position(|entry| entry.id == id))
                        .unwrap_or(if last {
                            self.entries.len().saturating_sub(1)
                        } else {
                            0
                        });
                    self.history_state
                        .select((!self.entries.is_empty()).then_some(index));
                    self.select_candidate(Some(0));
                }
            }
            Err(error) => {
                *stored_error = Some(error.clone());
                self.notice = error;
                if !recent {
                    self.entries.clear();
                    self.history_state.select(None);
                }
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
    fn focused_input(&self) -> Option<&Input> {
        if let Some(dialog) = &self.dialog {
            (dialog.field == 0).then_some(&dialog.text)
        } else {
            (self.tab == Tab::Lookup && self.focus == Focus::Input).then_some(&self.input)
        }
    }
    fn focused_input_mut(&mut self) -> Option<&mut Input> {
        if let Some(dialog) = &mut self.dialog {
            (dialog.field == 0).then_some(&mut dialog.text)
        } else {
            (self.tab == Tab::Lookup && self.focus == Focus::Input).then_some(&mut self.input)
        }
    }
    fn toggle_panes(&mut self) {
        self.pane_mode = !self.pane_mode;
        if let Some(input) = self.focused_input_mut() {
            input.mode(InputMode::Normal);
        }
        self.resolver.reset();
    }
    fn escape(&mut self) -> Effect {
        if self.pane_mode {
            self.pane_mode = false;
            self.resolver.reset();
            return Effect::None;
        }
        if self.resolver.pending() {
            self.resolver.reset();
            return Effect::None;
        }
        if let Some(dialog) = &mut self.dialog {
            if dialog.field == 0 && dialog.text.get_mode() != InputMode::Normal {
                dialog.text.mode(InputMode::Normal);
            } else {
                self.dialog = None;
            }
            return Effect::None;
        }
        if self.loading {
            self.generation += 1;
            self.loading = false;
            self.problem = Some("Lookup cancelled.".into());
            return Effect::Cancel;
        }
        if self.input.get_mode() != InputMode::Normal {
            self.input.mode(InputMode::Normal);
        } else {
            self.preview = None;
            self.select_candidate(Some(0));
        }
        Effect::None
    }
    fn event(&mut self, event: Event) -> Effect {
        if let Event::Paste(text) = event {
            if !self.pane_mode
                && let Some(input) = self.focused_input_mut()
                && let Err(error) = input.paste(&text)
            {
                self.notice = error;
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
        let text_editing = self
            .focused_input()
            .is_some_and(|input| input.get_mode() == InputMode::Insert);
        if key.code == KeyCode::Esc {
            return self.escape();
        }
        if self.pane_mode {
            if key.code == KeyCode::Enter {
                self.toggle_panes();
                return Effect::None;
            }
            if let Some(action) =
                self.resolver
                    .feed(&self.bindings, key, Context::Pane, Instant::now())
            {
                if action == Action::Pane {
                    self.toggle_panes();
                } else if action == Action::Cancel {
                    return self.escape();
                } else if matches!(
                    action,
                    Action::Left | Action::Right | Action::Up | Action::Down
                ) {
                    self.move_pane(action);
                }
            }
            return Effect::None;
        }
        if text_editing {
            if let Some(action) =
                self.resolver
                    .feed(&self.bindings, key, Context::Insert, Instant::now())
            {
                return self.input_command(action);
            }
            if !self.resolver.pending() {
                self.focused_input_mut().unwrap().edit(key);
            }
            return Effect::None;
        }
        if self.dialog.is_some() {
            return self.dialog_event(key);
        }
        let context = match self.focused_input().map(|i| i.get_mode()) {
            Some(InputMode::Visual) => Context::Visual,
            Some(_) => Context::Input,
            None if self.tab == Tab::History => Context::History,
            None => Context::Lookup,
        };
        let Some(action) = self
            .resolver
            .feed(&self.bindings, key, context, Instant::now())
        else {
            return Effect::None;
        };
        if let Some(input) = self.focused_input_mut()
            && matches!(
                action,
                Action::Edit
                    | Action::Append
                    | Action::Undo
                    | Action::Redo
                    | Action::Submit
                    | Action::Left
                    | Action::Right
                    | Action::Home
                    | Action::End
                    | Action::Paste
                    | Action::WordBegin
                    | Action::WordEnd
                    | Action::Visual
                    | Action::YankSelection
                    | Action::DeleteSelection
                    | Action::ChangeSelection
            )
        {
            let effect = input.action(action).into();
            if matches!(
                action,
                Action::Edit | Action::Append | Action::Submit | Action::Undo | Action::Redo
            ) {
                self.preview = None;
            }
            return effect;
        }
        match action {
            Action::Pane => {
                self.toggle_panes();
                Effect::None
            }
            Action::Cancel => self.escape(),
            Action::Quit => Effect::Quit,
            Action::NextTab | Action::PreviousTab => {
                self.resolver.reset();
                self.input.mode(InputMode::Normal);
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
                self.select_candidate(Some(0));
                if self.tab == Tab::History {
                    self.refresh()
                } else {
                    Effect::None
                }
            }
            Action::Edit => {
                self.focus = Focus::Input;
                self.input.mode(InputMode::Insert);
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
                let text = Input::with_text(&self.filter.text);
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
    fn move_pane(&mut self, action: Action) {
        if let Some(dialog) = &mut self.dialog {
            dialog.text.mode(InputMode::Normal);
            dialog.field = if matches!(action, Action::Left | Action::Up) {
                dialog.field.saturating_sub(1)
            } else {
                (dialog.field + 1).min(4)
            };
            self.resolver.reset();
            return;
        }
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
    }
    fn clipboard_effect(&mut self, clipboard: &mut impl Clipboard, effect: Effect) {
        match effect {
            Effect::Paste => match clipboard.paste() {
                Ok(text) => {
                    if let Some(input) = self.focused_input_mut() {
                        self.notice = match input.paste(&text) {
                            Ok(()) => "Pasted from clipboard.".into(),
                            Err(error) => error,
                        };
                    }
                }
                Err(error) => self.notice = error,
            },
            Effect::Copy(text) => {
                self.notice = match clipboard.copy(text) {
                    Ok(()) => "Copied to clipboard.".into(),
                    Err(error) => error,
                }
            }
            Effect::CopySelection(text, action) => match clipboard.copy(text) {
                Ok(()) => {
                    if let Some(input) = self.focused_input_mut() {
                        match action {
                            SelectionAction::Yank => input.mode(InputMode::Normal),
                            SelectionAction::Cut => input.cut(),
                            SelectionAction::Change => input.change_selection(),
                        }
                    }
                    self.notice = match action {
                        SelectionAction::Yank => "Selection copied to clipboard.",
                        SelectionAction::Cut => "Text cut to clipboard.",
                        SelectionAction::Change => "Selection cut to clipboard; insert mode.",
                    }
                    .into();
                }
                Err(error) => self.notice = error,
            },
            _ => {}
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
        self.input.mode(InputMode::Normal);
        self.resolver.reset();
        if self.focus == Focus::Recent && !self.recent.is_empty() {
            self.recent_state
                .select(Some(self.recent_state.selected().unwrap_or(0)));
            self.preview = self
                .recent
                .get(self.recent_state.selected().unwrap_or(0))
                .cloned();
            self.select_candidate(Some(0));
        }
    }
    fn select_candidate(&mut self, selected: Option<usize>) {
        self.move_candidate(selected);
        self.details_scroll = 0;
    }
    fn move_candidate(&mut self, selected: Option<usize>) {
        self.candidate.select(selected);
        self.candidate_line = 0;
    }
    fn max_candidate_line(&self, index: usize) -> usize {
        self.result().map_or(0, |result| {
            let lines = candidate_lines(result, index, self.details_viewport.0).len();
            if lines > self.details_viewport.1.max(1) as usize {
                lines.saturating_sub(1)
            } else {
                0
            }
        })
    }
    fn align_details(&mut self, heights: &[usize], action: Option<Action>) {
        let selected = self.candidate.selected().unwrap_or(0);
        let Some(&lines) = heights.get(selected) else {
            self.details_scroll = 0;
            self.candidate_line = 0;
            return;
        };
        let height = self.details_viewport.1.max(1) as usize;
        self.candidate_line = if lines > height {
            self.candidate_line.min(lines.saturating_sub(1))
        } else {
            0
        };
        let start: usize = heights[..selected].iter().sum();
        let cursor = start + self.candidate_line;
        let max_scroll = heights.iter().sum::<usize>().saturating_sub(height);
        // Keep the viewport still until the selection crosses the directional
        // margin. Reversing direction moves the selection through the pane first.
        let lower = (height - 1) * 7 / 10;
        let upper = (height - 1) * 3 / 10;
        match action {
            Some(Action::Down | Action::PageDown) => {
                self.details_scroll = self.details_scroll.max(cursor.saturating_sub(lower));
            }
            Some(Action::Up | Action::PageUp) => {
                self.details_scroll = self.details_scroll.min(cursor.saturating_sub(upper));
            }
            Some(Action::Home) => self.details_scroll = 0,
            Some(Action::End) => self.details_scroll = max_scroll,
            _ => {}
        }
        // Also keep selection visible after resizing or replacing results.
        self.details_scroll = self.details_scroll.min(cursor);
        let visible_end = if lines <= height {
            start + lines
        } else {
            cursor + 1
        };
        self.details_scroll = self.details_scroll.max(visible_end.saturating_sub(height));
        self.details_scroll = self.details_scroll.min(max_scroll);
    }
    fn navigate_details(&mut self, action: Action) {
        let count = self.result().map_or(0, |result| result.candidates.len());
        if count == 0 {
            return;
        }
        let selected = self.candidate.selected().unwrap_or(0).min(count - 1);
        let max = self.max_candidate_line(selected);
        let step = if matches!(action, Action::PageUp | Action::PageDown) {
            5
        } else {
            1
        };
        match action {
            Action::Down | Action::PageDown if self.candidate_line < max => {
                self.candidate_line = (self.candidate_line + step).min(max);
            }
            Action::Up | Action::PageUp if self.candidate_line > 0 => {
                self.candidate_line = self.candidate_line.saturating_sub(step);
            }
            Action::Down | Action::PageDown => {
                let next = (selected + step).min(count - 1);
                if next != selected {
                    self.move_candidate(Some(next));
                }
            }
            Action::Up | Action::PageUp => {
                let previous = selected.saturating_sub(step);
                if previous != selected {
                    self.move_candidate(Some(previous));
                    self.candidate_line = self.max_candidate_line(previous);
                }
            }
            Action::Home => self.move_candidate(Some(0)),
            Action::End => {
                self.move_candidate(Some(count - 1));
                self.candidate_line = self.max_candidate_line(count - 1);
            }
            _ => return,
        }
        let result = self.result().unwrap();
        let heights = result
            .candidates
            .iter()
            .enumerate()
            .map(|(index, _)| candidate_lines(result, index, self.details_viewport.0).len())
            .collect::<Vec<_>>();
        self.align_details(&heights, Some(action));
    }
    fn navigate(&mut self, action: Action) -> Effect {
        if self.focus == Focus::Details {
            self.navigate_details(action);
            return Effect::None;
        }
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
        let count = if self.focus == Focus::Recent {
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
            self.select_candidate(Some(0));
        }
        if self.focus == Focus::History {
            self.select_candidate(Some(0));
        }
        Effect::None
    }
    // Command dispatch is shared by insert-mode lookup and filter fields.
    fn input_command(&mut self, action: Action) -> Effect {
        match action {
            Action::Pane => self.toggle_panes(),
            Action::Cancel => return self.escape(),
            Action::WordBegin | Action::WordEnd => {
                return self.focused_input_mut().unwrap().action(action).into();
            }
            _ if self.dialog.is_some() => return self.dialog_control(action),
            Action::Submit => return Effect::Submit,
            Action::NextFocus => self.move_focus(1),
            Action::PreviousFocus => self.move_focus(-1),
            _ => {}
        }
        Effect::None
    }
    fn dialog_event(&mut self, key: KeyEvent) -> Effect {
        let d = self.dialog.as_ref().unwrap();
        if d.field == 1 && key.code == KeyCode::Char(' ') && key.modifiers.is_empty() {
            self.dialog.as_mut().unwrap().today = !d.today;
            self.resolver.reset();
            return Effect::None;
        }
        let context = if d.field != 0 {
            Context::Dialog
        } else if d.text.get_mode() == InputMode::Visual {
            Context::Visual
        } else {
            Context::Input
        };
        let Some(action) = self
            .resolver
            .feed(&self.bindings, key, context, Instant::now())
        else {
            return Effect::None;
        };
        if d.field == 0
            && !matches!(
                action,
                Action::Pane | Action::Cancel | Action::NextFocus | Action::PreviousFocus
            )
        {
            return self.dialog.as_mut().unwrap().text.action(action).into();
        }
        self.dialog_control(action)
    }
    fn dialog_control(&mut self, action: Action) -> Effect {
        let d = self.dialog.as_mut().unwrap();
        match action {
            Action::Pane => self.toggle_panes(),
            Action::Cancel => return self.escape(),
            Action::NextFocus | Action::PreviousFocus => {
                d.text.mode(InputMode::Normal);
                d.field = (d.field + if action == Action::NextFocus { 1 } else { 4 }) % 5;
                self.resolver.reset();
            }
            Action::Left | Action::Right if d.field == 1 => d.today = !d.today,
            Action::Submit => {
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
                    text: d.text.text(),
                    today: d.today,
                };
                self.dialog = None;
                self.entries.clear();
                self.history_state.select(None);
                self.select_candidate(None);
                self.resolver.reset();
                return Effect::Read {
                    cursor: None,
                    oldest: false,
                    last: false,
                    preserve: false,
                };
            }
            _ => {}
        }
        Effect::None
    }
    fn block(&self, title: &str, focus: Focus) -> Block<'static> {
        bordered(title, self.focus == focus)
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
        let mode = if self.pane_mode {
            "PANE"
        } else {
            self.focused_input()
                .map_or("NORMAL", |input| input.get_mode().label())
        };
        let help = if self.pane_mode {
            format!(
                "PANE · {} left / {} right · {} up / {} down\nEnter, Esc or {} finishes pane selection\n{}",
                self.bindings.label(Action::Left),
                self.bindings.label(Action::Right),
                self.bindings.label(Action::Up),
                self.bindings.label(Action::Down),
                self.bindings.label(Action::Pane),
                safe_text(&self.notice)
            )
        } else if let Some(input) = self.focused_input() {
            let actions = match input.get_mode() {
                InputMode::Insert => format!(
                    "{} submits · Esc normal",
                    self.bindings.label(Action::Submit)
                ),
                InputMode::Normal => format!(
                    "{}/{} insert · {} append · {} paste · {} select · {} cut",
                    self.bindings.label(Action::Submit),
                    self.bindings.label(Action::Edit),
                    self.bindings.label(Action::Append),
                    self.bindings.label(Action::Paste),
                    self.bindings.label(Action::Visual),
                    self.bindings.label(Action::DeleteSelection)
                ),
                InputMode::Visual => format!(
                    "{} copy · {} cut · {} change · {} replace · Esc normal",
                    self.bindings.label(Action::YankSelection),
                    self.bindings.label(Action::DeleteSelection),
                    self.bindings.label(Action::ChangeSelection),
                    self.bindings.label(Action::Paste)
                ),
            };
            let motions = if input.get_mode() == InputMode::Insert {
                format!(
                    "Ctrl-Left/Right word · {} panes · {} field",
                    self.bindings.label(Action::Pane),
                    self.bindings.label(Action::NextFocus)
                )
            } else if input.get_mode() == InputMode::Normal {
                format!(
                    "{} word ← · {} word → · {} undo · {} redo · {} panes",
                    self.bindings.label(Action::WordBegin),
                    self.bindings.label(Action::WordEnd),
                    self.bindings.label(Action::Undo),
                    self.bindings.label(Action::Redo),
                    self.bindings.label(Action::Pane)
                )
            } else {
                format!(
                    "{} word ← · {} word → · {} panes",
                    self.bindings.label(Action::WordBegin),
                    self.bindings.label(Action::WordEnd),
                    self.bindings.label(Action::Pane)
                )
            };
            format!("{mode} · {actions}\n{motions}\n{}", safe_text(&self.notice))
        } else {
            format!(
                "{mode} · {} tab · {} pane selection · {} next field\n{} filter · {} query · {} value · {} all\n{}",
                self.bindings.label(Action::NextTab),
                self.bindings.label(Action::Pane),
                self.bindings.label(Action::NextFocus),
                self.bindings.label(Action::Filter),
                self.bindings.label(Action::CopyQuery),
                self.bindings.label(Action::CopyValue),
                self.bindings.label(Action::CopyAll),
                safe_text(&self.notice)
            )
        };
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
        draw_input(
            frame,
            &self.input,
            rows[0],
            block,
            self.focus == Focus::Input && self.dialog.is_none() && !self.pane_mode,
        );
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
                        self.recent_error
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
        self.details_viewport = (parts[1].width, parts[1].height);
        let selected = self
            .candidate
            .selected()
            .unwrap_or(0)
            .min(result.candidates.len().saturating_sub(1));
        if self.candidate.selected() != Some(selected) {
            self.select_candidate(Some(selected));
        }
        let candidates = result
            .candidates
            .iter()
            .enumerate()
            .map(|(index, _)| candidate_lines(&result, index, parts[1].width))
            .collect::<Vec<_>>();
        let heights = candidates.iter().map(Vec::len).collect::<Vec<_>>();
        self.align_details(&heights, None);
        let selected_line = self.candidate_line;
        let lines = candidates
            .into_iter()
            .enumerate()
            .flat_map(|(index, lines)| {
                lines.into_iter().enumerate().map(move |(line, text)| {
                    let prefix = if index == selected && line == selected_line {
                        "> "
                    } else {
                        "  "
                    };
                    Line::styled(
                        format!("{prefix}{text}"),
                        if index == selected {
                            Style::default().fg(Color::Cyan)
                        } else {
                            Style::default()
                        },
                    )
                })
            })
            .skip(self.details_scroll)
            .take(parts[1].height as usize)
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(lines), parts[1]);
        frame.render_widget(
            Paragraph::new(safe_text(result.attribution.as_deref().unwrap_or("")))
                .wrap(Wrap { trim: false }),
            parts[2],
        );
    }
    fn draw_dialog(&mut self, frame: &mut Frame, area: Rect) {
        let width = area.width.min(68);
        let height = area.height.min(14);
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
        let rows = Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .split(inner);
        let d = self.dialog.as_ref().unwrap();
        let field_block = |title: &str, field| bordered(title, d.field == field);
        let block = field_block(" Text ", 0);
        draw_input(
            frame,
            &d.text,
            rows[0],
            block,
            d.field == 0 && !self.pane_mode,
        );
        frame.render_widget(
            Paragraph::new(if d.today { "Today" } else { "All history" })
                .block(field_block(" Date ", 1)),
            rows[1],
        );
        // Give the longer Cancel label enough room at the minimum 24-column size.
        let buttons =
            Layout::horizontal([Constraint::Min(7), Constraint::Min(7), Constraint::Min(8)])
                .split(rows[2]);
        for (index, label) in ["Apply", "Clear", "Cancel"].into_iter().enumerate() {
            frame.render_widget(
                Paragraph::new(label)
                    .centered()
                    .block(field_block("", index + 2)),
                buttons[index],
            );
        }
        let help = if self.pane_mode {
            format!("Enter / Esc / {} finish", self.bindings.label(Action::Pane))
        } else {
            format!(
                "{} field · Enter edit/apply\nSpace date · Esc normal/cancel",
                self.bindings.label(Action::NextFocus)
            )
        };
        frame.render_widget(Paragraph::new(help), rows[3]);
    }
}
fn candidate_lines(result: &LookupResult, index: usize, width: u16) -> Vec<String> {
    let Some(candidate) = result.candidates.get(index) else {
        return vec![];
    };
    let value = crate::clipboard::values(result, Some(index)).unwrap_or_default();
    let sense = candidate
        .sense
        .as_ref()
        .or(candidate.back_translations.first())
        .map(|s| format!(" · {}", safe_text(s)))
        .unwrap_or_default();
    let pos = candidate
        .part_of_speech
        .as_ref()
        .map(|s| format!(" ({})", safe_text(s)))
        .unwrap_or_default();
    textwrap::wrap(
        &format!("{}. {value}{pos}{sense}", index + 1),
        width.saturating_sub(2).max(1) as usize,
    )
    .into_iter()
    .map(|line| line.into_owned())
    .collect()
}
fn bordered(title: &str, focused: bool) -> Block<'static> {
    Block::default()
        .title(title.to_owned())
        .borders(Borders::ALL)
        .border_style(if focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        })
}
fn draw_input(frame: &mut Frame, input: &Input, area: Rect, block: Block<'_>, focused: bool) {
    let inner = block.inner(area);
    let (visible, cursor) = input.visible(inner.width as usize);
    frame.render_widget(Paragraph::new(visible).block(block), area);
    if focused && inner.width > 0 && inner.height > 0 {
        frame.set_cursor_position((inner.x + cursor, inner.y));
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
        let _ = execute!(
            io::stdout(),
            DisableBracketedPaste,
            SetCursorStyle::DefaultUserShape
        );
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
    read_history(
        &mut jobs,
        &coordinator,
        HistoryFilter::default(),
        None,
        (false, false, false),
        app.recent_generation,
        true,
    );
    let mut cursor_style = None;
    let outcome = loop {
        let desired_style = app
            .focused_input()
            .map_or(SetCursorStyle::SteadyBlock, |input| {
                input.get_mode().cursor_style()
            });
        if cursor_style != Some(desired_style) {
            if let Err(error) = execute!(terminal.backend_mut(), desired_style) {
                break Err(error);
            }
            cursor_style = Some(desired_style);
        }
        if let Err(error) = terminal.draw(|frame| app.draw(frame)) {
            break Err(error);
        }
        let effect = tokio::select! {
            event = events.next() => match event {
                Some(Ok(event)) => app.event(event),
                Some(Err(error)) => break Err(error),
                None => Effect::Quit,
            },
            _ = tokio::signal::ctrl_c() => Effect::Quit,
            Some(message) = progress_rx.recv() => {
                app.notice = safe_text(&message);
                Effect::None
            },
            Some(result) = jobs.join_next(), if !jobs.is_empty() => {
                match result {
                    Ok(Message::Lookup(id, completion)) => {
                        cancellations.remove(&id);
                        if id == app.generation {
                            app.complete(id, completion);
                        } else if !completion.warnings.is_empty() {
                            app.notice = completion.warnings.join(" · ");
                        }
                        app.recent_generation += 1;
                        read_history(
                            &mut jobs, &coordinator, HistoryFilter::default(), None,
                            (false, false, false), app.recent_generation, true,
                        );
                        if app.tab == Tab::History { app.refresh() } else { Effect::None }
                    },
                    Ok(Message::History(generation, recent, (last, preserve), result)) => {
                        app.apply_history(generation, recent, last, preserve, result);
                        Effect::None
                    },
                    Err(error) => {
                        app.notice = format!("Background task failed: {error}");
                        Effect::None
                    }
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
                app.read_generation += 1;
                read_history(
                    &mut jobs,
                    &coordinator,
                    app.filter.clone(),
                    cursor,
                    (oldest, last, preserve),
                    app.read_generation,
                    false,
                );
            }
            effect @ (Effect::Copy(_) | Effect::CopySelection(..) | Effect::Paste) => {
                app.clipboard_effect(&mut clipboard, effect);
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
    #[test]
    fn normal_input_enters_insert_and_pastes_at_unicode_cursor() {
        let mut app = App::new(None, None, Keybindings::default());
        app.input.insert("ae\u{301}猫z");
        app.escape();
        app.event(key('h'));
        app.event(key('h'));
        let mut clipboard = TestClipboard {
            text: "ö".into(),
            ..Default::default()
        };
        let effect = app.event(key('p'));
        assert!(matches!(effect, Effect::Paste));
        app.clipboard_effect(&mut clipboard, effect);
        assert_eq!(app.input.text(), "ae\u{301}ö猫z");
        assert_eq!(app.input.cursor(), "ae\u{301}ö".len());
        assert!(matches!(app.event(enter()), Effect::None));
        assert!(app.input.get_mode() == InputMode::Insert);
        app.event(key('p'));
        assert_eq!(app.input.text(), "ae\u{301}öp猫z");
        assert!(matches!(app.event(enter()), Effect::Submit));
    }
    #[test]
    fn change_selection_copies_before_editing_and_groups_replacement_for_undo() {
        for (profile, binding) in [
            (
                include_str!("../assets/keybindings/qwerty.keybinding.toml"),
                'c',
            ),
            (
                include_str!("../assets/keybindings/neo-noted.keybinding.toml"),
                'c',
            ),
            ("[actions]\nchange_selection=['z']", 'z'),
        ] {
            for filter in [false, true] {
                for reverse in [false, true] {
                    let mut app = App::new(None, None, Keybindings::parse(profile).unwrap());
                    if filter {
                        app.tab = Tab::History;
                        app.focus = Focus::History;
                        app.event(key('/'));
                    }
                    let original = "ae\u{301}👩‍💻z";
                    let input = app.focused_input_mut().unwrap();
                    *input = Input::with_text(original);
                    input.mode(InputMode::Normal);
                    input.set_cursor(if reverse { "ae\u{301}".len() } else { 1 });
                    app.event(key('v'));
                    app.event(Event::Key(KeyEvent::new(
                        if reverse {
                            KeyCode::Left
                        } else {
                            KeyCode::Right
                        },
                        KeyModifiers::NONE,
                    )));
                    let selected = app.focused_input().unwrap().selection();
                    let mut clipboard = TestClipboard {
                        text: "previous".into(),
                        unavailable: true,
                    };
                    let effect = app.event(key(binding));
                    app.clipboard_effect(&mut clipboard, effect);
                    let input = app.focused_input().unwrap();
                    assert_eq!(input.text(), original);
                    assert_eq!(input.selection(), selected);
                    assert!(input.get_mode() == InputMode::Visual);
                    assert_eq!(clipboard.text, "previous");
                    clipboard.unavailable = false;
                    let effect = app.event(key(binding));
                    // Text must remain intact until clipboard copying succeeds.
                    assert_eq!(app.focused_input().unwrap().text(), original);
                    app.clipboard_effect(&mut clipboard, effect);
                    assert_eq!(clipboard.text, "e\u{301}👩‍💻");
                    let input = app.focused_input().unwrap();
                    assert_eq!(input.text(), "az");
                    assert_eq!(input.cursor(), 1);
                    assert!(input.selection().is_none());
                    assert!(input.get_mode() == InputMode::Insert);
                    assert_eq!(input.get_mode().cursor_style(), SetCursorStyle::SteadyBar);
                    app.event(key('c')); // Ordinary typing in insert mode.
                    app.event(key('猫'));
                    app.escape();
                    app.event(key('u'));
                    assert_eq!(app.focused_input().unwrap().text(), original);
                    app.event(Event::Key(KeyEvent::new(
                        KeyCode::Char('r'),
                        KeyModifiers::CONTROL,
                    )));
                    assert_eq!(app.focused_input().unwrap().text(), "ac猫z");
                    assert_eq!(clipboard.text, "e\u{301}👩‍💻");
                }
            }
        }
    }
    #[test]
    fn change_requires_a_nonempty_visual_selection() {
        let mut app = App::new(None, None, Keybindings::default());
        app.input = Input::with_text("word");
        app.input.mode(InputMode::Normal);
        app.input.set_cursor(0);
        assert!(matches!(app.event(key('c')), Effect::None));
        assert_eq!(app.input.text(), "word");
        assert!(app.input.get_mode() == InputMode::Normal);
        for text in ["", "word"] {
            app.input = Input::with_text(text); // Cursor at the end gap.
            app.input.mode(InputMode::Visual);
            assert!(matches!(app.event(key('c')), Effect::None));
            assert_eq!(app.input.text(), text);
            assert!(app.input.get_mode() == InputMode::Visual);
        }
        assert!(matches!(
            app.event(Event::Key(KeyEvent::new(
                KeyCode::Char('c'),
                KeyModifiers::CONTROL
            ))),
            Effect::Quit
        ));
    }
    #[test]
    fn undo_redo_bindings_are_normal_input_only_and_preserve_history_refresh() {
        let ctrl_r = || Event::Key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        for (profile, undo, redo) in [
            (
                include_str!("../assets/keybindings/qwerty.keybinding.toml"),
                key('u'),
                ctrl_r(),
            ),
            (
                include_str!("../assets/keybindings/neo-noted.keybinding.toml"),
                key('u'),
                ctrl_r(),
            ),
            ("[actions]\nundo=['z']\nredo=['Z']", key('z'), key('Z')),
        ] {
            for filter in [false, true] {
                let mut app = App::new(None, None, Keybindings::parse(profile).unwrap());
                if filter {
                    app.tab = Tab::History;
                    app.focus = Focus::History;
                    app.filter.text = "saved ".into();
                    app.event(key('/'));
                }
                for c in "u猫".chars() {
                    app.event(key(c));
                }
                app.event(ctrl_r()); // Insert-mode Ctrl-r must not refresh or redo.
                let expected = if filter { "saved u猫" } else { "u猫" };
                assert_eq!(app.focused_input().unwrap().text(), expected);
                app.escape();
                assert!(matches!(app.event(undo.clone()), Effect::None));
                assert_eq!(
                    app.focused_input().unwrap().text(),
                    if filter { "saved " } else { "" }
                );
                assert!(matches!(app.event(redo.clone()), Effect::None));
                assert_eq!(app.focused_input().unwrap().text(), expected);
                assert!(app.focused_input().unwrap().get_mode() == InputMode::Normal);
                app.event(key('v'));
                app.event(undo.clone());
                app.event(redo.clone());
                assert_eq!(app.focused_input().unwrap().text(), expected);
                app.escape();
                // Failed cut/paste cannot consume or add an undo step.
                app.focused_input_mut().unwrap().set_cursor(0);
                let mut clipboard = TestClipboard {
                    unavailable: true,
                    ..Default::default()
                };
                for c in ['x', 'p'] {
                    let effect = app.event(key(c));
                    app.clipboard_effect(&mut clipboard, effect);
                }
                app.event(undo.clone());
                assert_eq!(
                    app.focused_input().unwrap().text(),
                    if filter { "saved " } else { "" }
                );
                app.event(redo.clone());
                app.dialog = None;
                app.tab = Tab::History;
                app.focus = Focus::History;
                assert!(matches!(app.event(ctrl_r()), Effect::Read { .. }));
            }
        }
    }
    #[test]
    fn append_enters_insert_after_a_whole_grapheme_in_both_inputs() {
        for (profile, binding) in [
            (
                include_str!("../assets/keybindings/qwerty.keybinding.toml"),
                'a',
            ),
            (
                include_str!("../assets/keybindings/neo-noted.keybinding.toml"),
                'a',
            ),
            ("[actions]\nappend = ['z']", 'z'),
        ] {
            for filter in [false, true] {
                for (text, cursor, expected) in [
                    ("abc", 0, "a!abc"),
                    ("ae\u{301}z", 1, "ae\u{301}!az"),
                    ("a👩‍💻z", 1, "a👩‍💻!az"),
                    ("abc", 2, "abc!a"),
                    ("abc", 3, "abc!a"),
                    ("", 0, "!a"),
                ] {
                    let mut app = App::new(None, None, Keybindings::parse(profile).unwrap());
                    if filter {
                        app.tab = Tab::History;
                        app.focus = Focus::History;
                        app.event(key('/'));
                    }
                    let input = app.focused_input_mut().unwrap();
                    input.insert(text);
                    input.set_cursor(cursor);
                    input.mode(InputMode::Normal);
                    assert!(matches!(app.event(key(binding)), Effect::None));
                    assert!(app.focused_input().unwrap().get_mode() == InputMode::Insert);
                    // Subsequent a presses must type text, not move the cursor.
                    app.event(key('!'));
                    app.event(key('a'));
                    assert_eq!(app.focused_input().unwrap().text(), expected);
                    assert!(app.focused_input().unwrap().selection().is_none());
                    let effect = app.event(enter());
                    if filter {
                        assert!(matches!(effect, Effect::Read { .. }));
                        assert_eq!(app.filter.text, expected);
                    } else {
                        assert!(matches!(effect, Effect::Submit));
                    }
                }
            }
        }
    }
    #[test]
    fn word_bindings_work_in_lookup_and_filter_inputs_without_stealing_insert_text() {
        for filter in [false, true] {
            let mut app = App::new(
                None,
                None,
                Keybindings::parse(include_str!(
                    "../assets/keybindings/neo-noted.keybinding.toml"
                ))
                .unwrap(),
            );
            if filter {
                app.tab = Tab::History;
                app.focus = Focus::History;
                app.event(key('/'));
            }
            app.focused_input_mut().unwrap().insert("one  e\u{301}lan");
            let ctrl = |code| Event::Key(KeyEvent::new(code, KeyModifiers::CONTROL));
            app.event(ctrl(KeyCode::Left));
            assert_eq!(app.focused_input().unwrap().cursor(), 5);
            app.event(ctrl(KeyCode::Right));
            assert_eq!(
                app.focused_input().unwrap().cursor(),
                "one  e\u{301}lan".len()
            );
            for c in "bexd".chars() {
                app.event(key(c));
            }
            assert_eq!(app.focused_input().unwrap().text(), "one  e\u{301}lanbexd");
            app.escape();
            app.event(key('b')); // Word beginning, not the Neo list Home alias.
            assert_eq!(app.focused_input().unwrap().cursor(), 5);
            app.event(key('b'));
            assert_eq!(app.focused_input().unwrap().cursor(), 0);
            app.event(key('e'));
            assert_eq!(app.focused_input().unwrap().cursor(), 2);
            app.event(key('v'));
            app.event(ctrl(KeyCode::Right));
            let input = app.focused_input().unwrap();
            assert_eq!(
                &input.text()[input.selection().unwrap()],
                "e  e\u{301}lanbexd"
            );
        }
    }
    #[test]
    fn d_and_x_cut_selection_or_current_grapheme_only_after_clipboard_success() {
        for key_char in ['d', 'x'] {
            for visual in [false, true] {
                let mut app = App::new(None, None, Keybindings::default());
                let original = "a e\u{301}👩‍💻z";
                app.input.insert(original);
                app.escape();
                app.input.set_cursor(2);
                if visual {
                    app.event(key('v'));
                    app.event(key('l'));
                }
                let mut clipboard = TestClipboard {
                    text: "previous".into(),
                    unavailable: true,
                };
                let effect = app.event(key(key_char));
                app.clipboard_effect(&mut clipboard, effect);
                assert_eq!(app.input.text(), original);
                assert_eq!(clipboard.text, "previous");
                clipboard.unavailable = false;
                let effect = app.event(key(key_char));
                app.clipboard_effect(&mut clipboard, effect);
                assert_eq!(
                    clipboard.text,
                    if visual {
                        "e\u{301}👩‍💻"
                    } else {
                        "e\u{301}"
                    }
                );
                assert_eq!(app.input.text(), if visual { "a z" } else { "a 👩‍💻z" });
                let effect = app.event(key('p'));
                app.clipboard_effect(&mut clipboard, effect);
                assert_eq!(app.input.text(), original);
                app.input.set_cursor(app.input.text().len());
                assert!(matches!(app.event(key(key_char)), Effect::None));
            }
        }
    }
    #[test]
    fn visual_selection_copies_cuts_and_replaces_whole_graphemes() {
        let mut app = App::new(
            None,
            None,
            Keybindings::parse(include_str!(
                "../assets/keybindings/neo-noted.keybinding.toml"
            ))
            .unwrap(),
        );
        app.input.insert("ae\u{301}猫z");
        app.escape();
        app.event(key('b')); // Home in Neo Noted.
        app.event(key('r'));
        app.event(key('v'));
        app.event(key('r')); // Include the combining grapheme and CJK character.
        let mut clipboard = TestClipboard::default();
        let effect = app.event(key('y'));
        app.clipboard_effect(&mut clipboard, effect);
        assert_eq!(clipboard.text, "e\u{301}猫");
        assert_eq!(app.input.text(), "ae\u{301}猫z");
        assert!(app.input.get_mode() == InputMode::Normal);
        app.event(key('v'));
        app.event(key('t')); // Reverse selection is also inclusive.
        let effect = app.event(key('d'));
        app.clipboard_effect(&mut clipboard, effect);
        assert_eq!(app.input.text(), "az");
        assert_eq!(app.input.cursor(), 1);
        app.event(key('v'));
        let effect = app.event(key('p'));
        app.clipboard_effect(&mut clipboard, effect);
        assert_eq!(app.input.text(), "ae\u{301}猫");
        assert!(app.input.get_mode() == InputMode::Normal);
    }
    #[test]
    fn rejected_clipboard_actions_preserve_input_and_selection() {
        let mut app = App::new(None, None, Keybindings::default());
        app.input.insert("word");
        app.escape();
        app.input.set_cursor(0);
        app.event(key('v'));
        let mut clipboard = TestClipboard {
            text: "two\nlines".into(),
            ..Default::default()
        };
        let effect = app.event(key('p'));
        app.clipboard_effect(&mut clipboard, effect);
        assert_eq!(app.input.text(), "word");
        assert!(app.input.get_mode() == InputMode::Visual);
        assert!(app.notice.contains("single line"));
        clipboard.unavailable = true;
        for action in ['p', 'd'] {
            let effect = app.event(key(action));
            app.clipboard_effect(&mut clipboard, effect);
            assert_eq!(app.input.text(), "word");
            assert_eq!(app.input.selection(), Some(0..1));
            assert!(app.notice.contains("unavailable"));
        }
    }
    #[test]
    fn filter_input_uses_the_same_modes_and_clipboard() {
        let mut app = App::new(None, None, Keybindings::default());
        app.tab = Tab::History;
        app.focus = Focus::History;
        app.event(key('/'));
        app.event(key('a'));
        app.event(key('b'));
        app.escape();
        assert!(app.dialog.as_ref().unwrap().text.get_mode() == InputMode::Normal);
        app.event(key('h'));
        let mut clipboard = TestClipboard {
            text: "猫".into(),
            ..Default::default()
        };
        let effect = app.event(key('p'));
        app.clipboard_effect(&mut clipboard, effect);
        assert_eq!(app.dialog.as_ref().unwrap().text.text(), "a猫b");
        assert!(matches!(app.event(enter()), Effect::None));
        assert!(app.dialog.as_ref().unwrap().text.get_mode() == InputMode::Insert);
        assert!(matches!(app.event(enter()), Effect::Read { .. }));
        assert_eq!(app.filter.text, "a猫b");
    }
    #[test]
    fn pane_mode_stays_active_until_explicitly_closed() {
        let mut app = App::new(None, None, Keybindings::default());
        let toggle = || Event::Key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
        app.event(toggle());
        app.event(key('j'));
        app.resolver.reset(); // Clearing a pending key sequence must not exit pane mode.
        app.event(key('l'));
        app.event(key('j'));
        assert!(app.pane_mode);
        assert_eq!(app.focus, Focus::Details);
        assert!(matches!(app.event(enter()), Effect::None));
        assert!(!app.pane_mode);
        assert_eq!(app.focus, Focus::Details);
        app.event(toggle());
        app.event(key('q'));
        assert!(app.pane_mode);
        app.event(toggle());
        assert!(!app.pane_mode);
        app.event(toggle());
        app.escape();
        assert!(!app.pane_mode);
    }
    #[test]
    fn enter_leaves_pane_mode_without_editing_submitting_or_applying() {
        for filter in [false, true] {
            for field in 0..5 {
                let mut app = App::new(None, None, Keybindings::default());
                if filter {
                    app.tab = Tab::History;
                    app.focus = Focus::History;
                    app.event(key('/'));
                    app.dialog.as_mut().unwrap().field = field;
                }
                app.event(Event::Key(KeyEvent::new(
                    KeyCode::Char('w'),
                    KeyModifiers::CONTROL,
                )));
                app.event(key('g')); // Enter also clears a pending key sequence.
                assert!(matches!(app.event(enter()), Effect::None));
                assert!(!app.pane_mode);
                assert!(!app.resolver.pending());
                if filter {
                    let dialog = app.dialog.as_ref().unwrap();
                    assert_eq!(dialog.field, field);
                    assert!(dialog.text.text().is_empty());
                    assert!(app.filter.text.is_empty());
                } else {
                    assert_eq!(app.focus, Focus::Input);
                }
                if let Some(input) = app.focused_input() {
                    assert!(input.get_mode() == InputMode::Normal);
                    assert!(matches!(app.event(enter()), Effect::None));
                    assert!(app.focused_input().unwrap().get_mode() == InputMode::Insert);
                }
            }
        }
    }
    #[test]
    fn typing_shortcuts_and_graphemes_does_not_navigate() {
        let mut app = App::new(None, None, Keybindings::default());
        for c in "qgtjkmnblG/".chars() {
            assert!(matches!(app.event(key(c)), Effect::None));
        }
        assert_eq!(app.input.text(), "qgtjkmnblG/");
        assert_eq!(app.tab, Tab::Lookup);
        let mut input = Input::default();
        input.insert("äe\u{301}猫");
        input.left();
        input.backspace();
        assert_eq!(input.text(), "ä猫");
        input.delete();
        assert_eq!(input.text(), "ä");
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
        assert_eq!(app.input.text(), "word");
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
        app.input.mode(InputMode::Normal);
        app.focus = Focus::Details;
        app.move_focus(1);
        assert_eq!(app.preview.as_ref().unwrap().query, "saved word");
        assert_eq!(app.input.text(), "unfinished word");
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
        assert_eq!(app.input.text(), "unfinished word");
    }
    #[test]
    fn letter_remaps_never_steal_text_input() {
        let bindings = Keybindings::parse(
            "[actions]\nsubmit=['s']\ncancel=['c']\nnext_focus=['f']\nchange_selection=['C']",
        )
        .unwrap();
        let mut app = App::new(None, None, bindings);
        for c in "scf".chars() {
            assert!(matches!(app.event(key(c)), Effect::None));
        }
        assert_eq!(app.input.text(), "scf");
    }
    #[test]
    fn pane_directions_follow_the_lookup_layout_without_wrapping() {
        let bindings = Keybindings::parse(include_str!(
            "../assets/keybindings/neo-noted.keybinding.toml"
        ))
        .unwrap();
        let mut app = App::new(None, None, bindings);
        app.event(Event::Key(KeyEvent::new(
            KeyCode::Char('w'),
            KeyModifiers::CONTROL,
        )));
        assert!(app.pane_mode);
        let mut direction = |key_code, expected| {
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
            app.live = Some(result);
        } else {
            let mut entry = saved_entry(1);
            entry.finished = Some(crate::history::Finished::from_result(&Ok(result)));
            if view == 1 {
                app.preview = Some(entry);
            } else {
                app.tab = Tab::History;
                app.entries.push(entry);
                app.history_state.select(Some(0));
            }
        }
        app
    }
    fn selection_row(app: &mut App, width: u16, height: u16) -> usize {
        let rendered = screen(app, width, height);
        let marker = format!("> {}. value", app.candidate.selected().unwrap() + 1);
        let position = rendered
            .find(&marker)
            .expect("selected candidate must be visible");
        rendered[..position].chars().count() / width as usize
    }
    #[test]
    fn details_selection_moves_between_directional_margins_and_list_boundaries() {
        for view in 0..3 {
            // Live lookup, recent preview, and History details.
            for (width, height) in [(80, 30), (120, 32)] {
                let mut app = scrolling_app(view, 40);
                let first_row = selection_row(&mut app, width, height);
                let pane_height = app.details_viewport.1 as usize;
                let lower = (pane_height - 1) * 7 / 10;
                let upper = (pane_height - 1) * 3 / 10;
                assert!(lower > upper);
                // Selection initially moves down without scrolling.
                for index in 1..=lower {
                    app.navigate(Action::Down);
                    assert_eq!(selection_row(&mut app, width, height), first_row + index);
                    assert_eq!(app.details_scroll, 0);
                }
                for _ in 0..12 {
                    app.navigate(Action::Down);
                    assert_eq!(selection_row(&mut app, width, height), first_row + lower);
                }
                // Reversing direction must not jump straight to the upper margin.
                let offset = app.details_scroll;
                for row in (upper..lower).rev() {
                    app.navigate(Action::Up);
                    assert_eq!(selection_row(&mut app, width, height), first_row + row);
                    assert_eq!(app.details_scroll, offset);
                }
                app.navigate(Action::Up);
                assert_eq!(selection_row(&mut app, width, height), first_row + upper);
                assert_eq!(app.details_scroll, offset - 1);
                // At the end, scrolling stops and selection reaches the bottom row.
                for _ in 0..40 {
                    app.navigate(Action::Down);
                }
                assert_eq!(app.candidate.selected(), Some(39));
                assert_eq!(
                    selection_row(&mut app, width, height),
                    first_row + pane_height - 1
                );
                assert_eq!(app.details_scroll, 40 - pane_height);
                // At the beginning, selection can reach the top row again.
                for _ in 0..40 {
                    app.navigate(Action::Up);
                }
                assert_eq!(selection_row(&mut app, width, height), first_row);
                assert_eq!(app.candidate.selected(), Some(0));
                assert_eq!(app.details_scroll, 0);
            }
        }
    }
    #[test]
    fn details_paging_short_lists_and_resize_keep_selection_visible() {
        for view in 0..3 {
            let mut short = scrolling_app(view, 3);
            let first_row = selection_row(&mut short, 120, 32);
            short.navigate(Action::End);
            assert_eq!(selection_row(&mut short, 120, 32), first_row + 2);
            assert_eq!(short.details_scroll, 0);
            short.navigate(Action::Up);
            assert_eq!(selection_row(&mut short, 120, 32), first_row + 1);
            let mut app = scrolling_app(view, 40);
            selection_row(&mut app, 120, 32);
            app.navigate(Action::PageDown);
            app.navigate(Action::PageDown);
            assert_eq!(app.candidate.selected(), Some(10));
            selection_row(&mut app, 120, 32);
            // Shrink and expand, retaining the selected candidate.
            for (width, height) in [(40, 18), (120, 40)] {
                selection_row(&mut app, width, height);
                assert_eq!(app.candidate.selected(), Some(10));
                assert!(app.details_scroll <= 10);
                assert!(10 < app.details_scroll + app.details_viewport.1 as usize);
            }
            app.navigate(Action::PageUp);
            assert_eq!(app.candidate.selected(), Some(5));
            selection_row(&mut app, 120, 40);
            app.navigate(Action::Home);
            assert_eq!(app.details_scroll, 0);
        }
    }
    #[test]
    fn oversized_candidates_scroll_to_their_last_line_in_live_and_saved_details() {
        let result = LookupResult {
            query: "word".into(),
            headword: "word".into(),
            normalized_headword: "word".into(),
            pair: INITIAL_PAIRS[0],
            candidates: vec![TranslationCandidate {
                text: format!("FIRST {} LAST", "translation ".repeat(100)),
                normalized: "word".into(),
                part_of_speech: None,
                sense: None,
                prefix: String::new(),
                back_translations: vec![],
            }],
            provider: "fixture".into(),
            attribution: None,
            kind: ResultKind::Dictionary,
        };
        for saved in [false, true] {
            for (width, height) in [(120, 32), (80, 24), (40, 18)] {
                let mut app = App::new(None, None, Keybindings::default());
                app.focus = Focus::Details;
                if saved {
                    app.tab = Tab::History;
                    let mut entry = saved_entry(1);
                    entry.finished =
                        Some(crate::history::Finished::from_result(&Ok(result.clone())));
                    app.entries.push(entry);
                    app.history_state.select(Some(0));
                } else {
                    app.live = Some(result.clone());
                }
                assert!(screen(&mut app, width, height).contains("FIRST"));
                app.navigate(Action::Up); // Start boundary must not jump to the bottom.
                assert_eq!(app.candidate_line, 0);
                for _ in 0..200 {
                    app.navigate(Action::Down);
                }
                assert!(screen(&mut app, width, height).contains("LAST"));
                assert_eq!(app.candidate.selected(), Some(0));
                // Copy still addresses the entire candidate, not the visible line.
                app.event(key('y'));
                assert!(
                    matches!(app.event(key('y')), Effect::Copy(text) if text == result.candidates[0].text)
                );
                app.navigate(Action::Home);
                assert!(screen(&mut app, width, height).contains("FIRST"));
                app.navigate(Action::End);
                assert!(screen(&mut app, width, height).contains("LAST"));
                // Resizing preserves selection and every line remains reachable.
                screen(&mut app, 40, 18);
                app.navigate(Action::End);
                assert!(screen(&mut app, 40, 18).contains("LAST"));
                app.navigate(Action::PageUp);
                assert!(app.candidate_line < app.max_candidate_line(0));
            }
        }
    }
    #[test]
    fn details_navigation_moves_between_candidates_after_scrolling() {
        let mut app = App::new(None, None, Keybindings::default());
        app.focus = Focus::Details;
        app.live = Some(LookupResult {
            query: "word".into(),
            headword: "word".into(),
            normalized_headword: "word".into(),
            pair: INITIAL_PAIRS[0],
            candidates: (0..2)
                .map(|index| TranslationCandidate {
                    text: format!("{} END{index}", "translation ".repeat(100)),
                    normalized: "word".into(),
                    part_of_speech: None,
                    sense: None,
                    prefix: String::new(),
                    back_translations: vec![],
                })
                .collect(),
            provider: "fixture".into(),
            attribution: None,
            kind: ResultKind::Dictionary,
        });
        screen(&mut app, 80, 24);
        for _ in 0..app.max_candidate_line(0) {
            app.navigate(Action::Down);
        }
        assert_eq!(app.candidate.selected(), Some(0));
        assert!(screen(&mut app, 80, 24).contains("END0"));
        app.navigate(Action::Down);
        assert_eq!(app.candidate.selected(), Some(1));
        assert_eq!(app.candidate_line, 0);
        app.navigate(Action::Up);
        assert_eq!(app.candidate.selected(), Some(0));
        assert!(screen(&mut app, 80, 24).contains("END0"));
        app.navigate(Action::End);
        assert_eq!(app.candidate.selected(), Some(1));
        assert!(screen(&mut app, 80, 24).contains("END1"));
    }
    #[test]
    fn command_sequences_execute_in_both_inputs_and_dialog_controls() {
        for filter in [false, true] {
            let bindings = Keybindings::parse("[actions]\nsubmit=['Ctrl-x Ctrl-s']\nnext_focus=['Ctrl-x Ctrl-f']\nword_begin=['Ctrl-x Ctrl-b']").unwrap();
            let mut app = App::new(None, None, bindings);
            if filter {
                app.tab = Tab::History;
                app.focus = Focus::History;
                app.event(key('/'));
            }
            app.focused_input_mut().unwrap().insert("first last");
            assert!(matches!(app.event(ctrl('x')), Effect::None));
            app.event(ctrl('b'));
            assert_eq!(app.focused_input().unwrap().cursor(), 6);
            // An ordinary letter following a prefix cancels the prefix and is typed.
            app.event(ctrl('x'));
            app.event(key('s'));
            assert_eq!(app.focused_input().unwrap().text(), "first slast");
            // Escape cancels a partial sequence without leaving insert mode.
            app.event(ctrl('x'));
            app.escape();
            assert!(app.focused_input().unwrap().get_mode() == InputMode::Insert);
            app.event(ctrl('s'));
            assert!(!app.loading);
            app.event(ctrl('x'));
            let effect = app.event(ctrl('s'));
            if filter {
                assert!(matches!(effect, Effect::Read { .. }));
                assert_eq!(app.filter.text, "first slast");
                app.event(key('/'));
                app.event(ctrl('x'));
                app.event(ctrl('f'));
                assert_eq!(app.dialog.as_ref().unwrap().field, 1);
                app.event(ctrl('x'));
                app.event(ctrl('f'));
                assert_eq!(app.dialog.as_ref().unwrap().field, 2);
                app.event(ctrl('x'));
                assert!(matches!(app.event(ctrl('s')), Effect::Read { .. }));
            } else {
                assert!(matches!(effect, Effect::Submit));
                app.event(ctrl('x'));
                app.event(ctrl('f'));
                assert_eq!(app.focus, Focus::Source);
            }
        }
    }
    #[test]
    fn modified_prefix_can_have_a_letter_suffix_without_stealing_regular_typing() {
        for filter in [false, true] {
            let mut app = App::new(
                None,
                None,
                Keybindings::parse(
                    "[actions]\nsubmit=['Ctrl-x s']\npane_prefix=['Ctrl-x w']\ncancel=['Ctrl-x e']",
                )
                .unwrap(),
            );
            if filter {
                app.tab = Tab::History;
                app.focus = Focus::History;
                app.event(key('/'));
            }
            for c in "sew".chars() {
                app.event(key(c));
            }
            assert_eq!(app.focused_input().unwrap().text(), "sew");
            app.event(ctrl('x'));
            let effect = app.event(key('s'));
            assert!(if filter {
                matches!(effect, Effect::Read { .. })
            } else {
                matches!(effect, Effect::Submit)
            });
            if filter {
                app.event(key('/'));
            }
            app.event(ctrl('x'));
            app.event(key('w'));
            assert!(app.pane_mode);
            app.event(ctrl('x'));
            app.event(key('e'));
            assert!(!app.pane_mode);
            assert_eq!(app.focused_input().unwrap().text(), "sew");
        }
    }
    #[test]
    fn command_sequence_suffix_is_not_intercepted_by_a_direct_binding() {
        let mut app = App::new(
            None,
            None,
            Keybindings::parse("[actions]\nsubmit=['Ctrl-x Ctrl-w']").unwrap(),
        );
        app.input.insert("word");
        app.event(ctrl('x'));
        assert!(matches!(app.event(ctrl('w')), Effect::Submit));
        assert!(!app.pane_mode);
        app.event(ctrl('w'));
        assert!(app.pane_mode);
    }
    #[test]
    fn history_updates_ignore_stale_reads_and_preserve_selection() {
        let mut app = App::new(None, None, Keybindings::default());
        app.read_generation = 2;
        app.apply_history(2, false, false, false, page(&[3, 2, 1]));
        app.history_state.select(Some(1));
        app.read_generation = 3;
        app.apply_history(2, false, false, false, page(&[9]));
        app.apply_history(2, false, false, false, Err("stale error".into()));
        assert_eq!(app.entries.len(), 3);
        assert!(app.history_error.is_none());
        app.apply_history(3, false, false, true, page(&[4, 3, 2, 1]));
        assert_eq!(app.history_state.selected(), Some(2));
        app.apply_history(3, false, false, false, page(&[]));
        assert_eq!(app.entries.len(), 4); // Paging past the boundary keeps the page.
        app.apply_history(3, false, true, false, page(&[8, 7]));
        assert_eq!(app.history_state.selected(), Some(1));
        app.apply_history(3, false, false, true, page(&[]));
        assert!(app.entries.is_empty());
        assert_eq!(app.history_state.selected(), None);
    }
    #[test]
    fn history_read_errors_recover_without_interference_from_recent_reads() {
        let mut app = App::new(None, None, Keybindings::default());
        app.apply_history(0, false, false, false, page(&[1]));
        app.apply_history(0, false, false, true, Err("read failed".into()));
        assert!(app.entries.is_empty());
        app.recent_generation = 1;
        app.apply_history(1, true, false, false, page(&[3, 2]));
        app.recent_state.select(Some(1));
        app.preview = Some(app.recent[1].clone());
        assert_eq!(app.history_error.as_deref(), Some("read failed"));
        app.apply_history(0, true, false, false, page(&[9]));
        assert_eq!(app.recent.len(), 2);
        let mut updated = page(&[4, 3, 2]).unwrap();
        updated.entries[2].finished = Some(crate::history::Finished::from_result(&Err(
            LookupError::Cancelled,
        )));
        app.apply_history(1, true, false, false, Ok(updated));
        assert_eq!(app.recent_state.selected(), Some(2));
        assert_eq!(app.preview.as_ref().unwrap().status(), "cancelled");
        app.read_generation += 1;
        app.apply_history(1, false, false, true, page(&[3, 2, 1]));
        assert!(app.history_error.is_none());
        assert!(app.notice.is_empty());
        assert_eq!(app.history_state.selected(), Some(0));
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
