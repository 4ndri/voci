//! Saved-history browsing state, filtering, and the history tab layout.

use super::{App, Effect, Focus, FocusTarget, HistoryRead, HistorySelection};
use super::{input::Input, keybindings::Action, navigation::next_index};
use crate::{
    domain::Language,
    history::{Cursor, HistoryEntry, HistoryFilter, HistoryPage},
    presentation::display_time,
    text::safe_text,
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    widgets::{List, ListItem, ListState, Paragraph, Wrap},
};

mod dialog;

pub(super) struct Dialog {
    pub(super) text: Input,
    pub(super) today: bool,
    pub(super) field: usize,
}

#[derive(Default)]
pub(super) struct HistoryPane {
    pub(super) entries: Vec<HistoryEntry>,
    pub(super) state: ListState,
    pub(super) filter: HistoryFilter,
    pub(super) dialog: Option<Dialog>,
    pub(super) generation: u64,
    pub(super) error: Option<String>,
}

impl HistoryPane {
    pub(super) fn navigate(&mut self, action: Action) -> Effect {
        if matches!(action, Action::Home | Action::End) {
            return Effect::Read(HistoryRead {
                cursor: None,
                oldest: action == Action::End,
                selection: if action == Action::End {
                    HistorySelection::Last
                } else {
                    HistorySelection::First
                },
            });
        }
        let count = self.entries.len();
        if count == 0 {
            return Effect::None;
        }
        let index = self.state.selected().unwrap_or(0);
        if matches!(action, Action::Down | Action::PageDown) && index + 1 >= count {
            return Effect::Read(HistoryRead {
                cursor: self.entries.last().map(HistoryEntry::cursor),
                oldest: false,
                selection: HistorySelection::First,
            });
        }
        if matches!(action, Action::Up | Action::PageUp) && index == 0 {
            return Effect::Read(HistoryRead {
                cursor: self.entries.first().map(HistoryEntry::cursor),
                oldest: true,
                selection: HistorySelection::Last,
            });
        }
        self.state.select(Some(next_index(action, index, count)));
        Effect::None
    }

    pub(super) fn selected(&self) -> Option<&HistoryEntry> {
        self.state.selected().and_then(|i| self.entries.get(i))
    }

    /// Return whether a new page requires resetting the shared details selection.
    pub(super) fn apply(
        &mut self,
        generation: u64,
        selection: HistorySelection,
        result: Result<HistoryPage, String>,
        notice: &mut String,
    ) -> bool {
        if generation != self.generation {
            return false;
        }
        let preserve = selection == HistorySelection::Preserve;
        match result {
            Ok(page) => {
                if self.error.as_deref() == Some(notice.as_str()) {
                    notice.clear();
                }
                self.error = None;
                if page.entries.is_empty() && !self.entries.is_empty() && !preserve {
                    return false;
                }
                let selected = preserve
                    .then(|| self.selected().map(|entry| entry.id.clone()))
                    .flatten();
                self.entries = page.entries;
                let index = selected
                    .and_then(|id| self.entries.iter().position(|entry| entry.id == id))
                    .unwrap_or(if selection == HistorySelection::Last {
                        self.entries.len().saturating_sub(1)
                    } else {
                        0
                    });
                self.state
                    .select((!self.entries.is_empty()).then_some(index));
                true
            }
            Err(error) => {
                self.error = Some(error.clone());
                *notice = error;
                self.entries.clear();
                self.state.select(None);
                false
            }
        }
    }

    pub(super) fn refresh(&self) -> HistoryRead {
        HistoryRead {
            cursor: self.selected().map(|entry| Cursor {
                timestamp: entry.started_at,
                sequence: entry.sequence.saturating_add(1),
            }),
            oldest: false,
            selection: HistorySelection::Preserve,
        }
    }
}

impl App {
    pub(super) fn draw_history(&mut self, frame: &mut Frame, area: Rect) {
        let rows = Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).split(area);
        frame.render_widget(
            Paragraph::new(format!(
                "Filter: {} · {}",
                if self.history.filter.text.is_empty() {
                    "all text"
                } else {
                    &self.history.filter.text
                },
                if self.history.filter.today {
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
            self.focus_regions
                .push((panes[0], FocusTarget::Pane(Focus::History)));
            let block = self.block(" Encounters · newest first ", Focus::History);
            if self.history.entries.is_empty() {
                let text = self.history.error.clone().unwrap_or_else(|| {
                    if self.history.filter.text.is_empty() && !self.history.filter.today {
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
                    .history
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
                    &mut self.history.state,
                );
            }
        }
        if !single || self.focus == Focus::Details {
            self.draw_details(frame, panes[1]);
        }
    }
}
