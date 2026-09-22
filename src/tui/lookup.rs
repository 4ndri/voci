//! Lookup draft, live results, suggestions, and recent saved previews.

use super::{
    App, Focus, FocusTarget,
    input::{Input, InputMode},
    render::{bordered, draw_input},
};
use super::{keybindings::Action, navigation::next_index};
use crate::{
    app::Completion,
    domain::{Language, LookupResult},
    history::{HistoryEntry, HistoryPage},
    lookup::LookupRequest,
    presentation::{display_time, lookup_error_text},
    text::safe_text,
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    widgets::{Clear, List, ListItem, ListState, Paragraph, Wrap},
};

#[derive(Default)]
pub(super) struct LookupPane {
    pub(super) input: Input,
    pub(super) source: Option<Language>,
    pub(super) target: Option<Language>,
    pub(super) live: Option<LookupResult>,
    pub(super) problem: Option<String>,
    pub(super) loading: bool,
    pub(super) generation: u64,
    pub(super) recent: Vec<HistoryEntry>,
    pub(super) recent_state: ListState,
    pub(super) suggestions: Vec<HistoryEntry>,
    pub(super) suggestion_state: ListState,
    pub(super) suggestion_query: Option<(String, Option<Language>, Option<Language>)>,
    pub(super) suggestion_generation: u64,
    pub(super) preview: Option<HistoryEntry>,
    pub(super) recent_generation: u64,
    pub(super) recent_error: Option<String>,
}

impl LookupPane {
    pub(super) fn new(source: Option<Language>, target: Option<Language>) -> Self {
        Self {
            source,
            target,
            ..Self::default()
        }
    }

    pub(super) fn result(&self) -> Option<&LookupResult> {
        match &self.preview {
            Some(entry) => entry.result(),
            None => self.live.as_ref(),
        }
    }

    pub(super) fn navigate_language(&mut self, focus: Focus, action: Action) {
        let selected = match focus {
            Focus::Source => &mut self.source,
            Focus::Target => &mut self.target,
            _ => return,
        };
        let options = [None, Some(Language::German), Some(Language::English)];
        let index = options
            .iter()
            .position(|value| value == selected)
            .expect("language selector supports every domain language");
        let delta = match action {
            Action::Right | Action::Down => 1,
            Action::Left | Action::Up => 2,
            _ => 0,
        };
        *selected = options[(index + delta) % options.len()];
    }

    pub(super) fn navigate_recent(&mut self, action: Action) -> bool {
        if self.recent.is_empty() {
            return false;
        }
        let index = next_index(
            action,
            self.recent_state.selected().unwrap_or(0),
            self.recent.len(),
        );
        self.recent_state.select(Some(index));
        self.preview = self.recent.get(index).cloned();
        true
    }

    pub(super) fn begin(&mut self) -> (u64, LookupRequest) {
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

    pub(super) fn update_suggestion_query(&mut self, active: bool) -> bool {
        let query = (active
            && self.input.mode() == InputMode::Insert
            && !self.loading
            && !self.input.text().trim().is_empty())
        .then(|| (self.input.text(), self.source, self.target));
        if query == self.suggestion_query {
            return false;
        }
        self.suggestion_query = query;
        self.suggestion_generation += 1;
        self.suggestions.clear();
        self.suggestion_state = ListState::default();
        true
    }

    pub(super) fn apply_suggestions(
        &mut self,
        generation: u64,
        result: Result<HistoryPage, String>,
        notice: &mut String,
    ) {
        if generation != self.suggestion_generation || self.suggestion_query.is_none() {
            return;
        }
        match result {
            Ok(page) => self.suggestions = page.entries,
            Err(error) => *notice = error,
        }
    }

    pub(super) fn complete(&mut self, id: u64, completion: Completion) -> Option<String> {
        if id != self.generation {
            return None;
        }
        self.loading = false;
        match completion.result {
            Ok(result) => {
                self.live = Some(result);
                self.problem = None;
            }
            Err(error) => {
                self.live = None;
                self.problem = Some(lookup_error_text(&error));
            }
        }
        Some(completion.warnings.join(" · "))
    }

    pub(super) fn apply_recent(
        &mut self,
        generation: u64,
        result: Result<HistoryPage, String>,
        notice: &mut String,
    ) {
        if generation != self.recent_generation {
            return;
        }
        match result {
            Ok(page) => {
                if self.recent_error.as_deref() == Some(notice.as_str()) {
                    notice.clear();
                }
                self.recent_error = None;
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
                    && let Some(updated) = self.recent.iter().find(|entry| entry.id == preview.id)
                {
                    self.preview = Some(updated.clone());
                }
            }
            Err(error) => {
                self.recent_error = Some(error.clone());
                *notice = error;
            }
        }
    }
}

// Compose the Lookup tab with the shared details pane and shell focus policy.
impl App {
    pub(super) fn draw_lookup(&mut self, frame: &mut Frame, area: Rect) {
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
            &self.lookup.input,
            rows[0],
            block,
            self.focus == Focus::Input && self.history.dialog.is_none() && !self.pane_mode,
        );
        let cols = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(rows[1]);
        self.focus_regions.extend([
            (rows[0], FocusTarget::Pane(Focus::Input)),
            (cols[0], FocusTarget::Pane(Focus::Source)),
            (cols[1], FocusTarget::Pane(Focus::Target)),
        ]);
        frame.render_widget(
            Paragraph::new(self.lookup.source.map_or("Auto", Language::code))
                .block(self.block(" Source ", Focus::Source)),
            cols[0],
        );
        frame.render_widget(
            Paragraph::new(self.lookup.target.map_or("Default", Language::code))
                .block(self.block(" Target ", Focus::Target)),
            cols[1],
        );
        self.draw_details(frame, rows[2]);
        if recent_height > 0 {
            self.focus_regions
                .push((rows[3], FocusTarget::Pane(Focus::Recent)));
            let items = self
                .lookup
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
                        self.lookup
                            .recent_error
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
                    &mut self.lookup.recent_state,
                );
            }
        }
        if self.has_suggestions() {
            let popup = Rect::new(
                rows[0].x,
                rows[0].bottom(),
                rows[0].width,
                (self.lookup.suggestions.len() as u16 + 2).min(area.height.saturating_sub(3)),
            );
            self.focus_regions
                .push((popup, FocusTarget::Pane(Focus::Input)));
            let items = self
                .lookup
                .suggestions
                .iter()
                .map(|entry| {
                    let result = entry.result();
                    ListItem::new(format!(
                        "{} · {} · {}",
                        safe_text(&entry.query),
                        result.map(|r| r.pair.to_string()).unwrap_or_default(),
                        result
                            .and_then(|r| r.candidates.first())
                            .map(|c| safe_text(&c.text))
                            .unwrap_or_default(),
                    ))
                })
                .collect::<Vec<_>>();
            frame.render_widget(Clear, popup);
            frame.render_stateful_widget(
                List::new(items)
                    .block(bordered(" From history ", true))
                    .highlight_symbol("> ")
                    .highlight_style(Style::default().fg(Color::Cyan)),
                popup,
                &mut self.lookup.suggestion_state,
            );
        }
    }
}
