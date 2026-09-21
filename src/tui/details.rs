//! Candidate wrapping, selection, and scrolling share one details viewport.

use super::{App, Focus, FocusTarget, Tab};
use crate::history::HistoryEntry;
use crate::{
    domain::LookupResult, presentation::display_time, text::safe_text, tui::keybindings::Action,
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::Line,
    widgets::{Block, ListState, Paragraph, Wrap},
};

pub(super) struct DetailsPane {
    pub(super) selection: ListState,
    // Reading position within an oversized candidate, distinct from viewport offset.
    pub(super) line: usize,
    pub(super) scroll: usize,
    pub(super) viewport: (u16, u16),
}

impl Default for DetailsPane {
    fn default() -> Self {
        Self {
            selection: ListState::default(),
            line: 0,
            scroll: 0,
            viewport: (1, 1),
        }
    }
}

pub(super) enum DetailsContent<'a> {
    Lookup {
        result: Option<&'a LookupResult>,
        loading: bool,
        problem: Option<&'a str>,
    },
    Saved(Option<&'a HistoryEntry>),
}

impl DetailsPane {
    pub(super) fn select_candidate(&mut self, selected: Option<usize>) {
        self.move_candidate(selected);
        self.scroll = 0;
    }

    fn move_candidate(&mut self, selected: Option<usize>) {
        self.selection.select(selected);
        self.line = 0;
    }

    pub(super) fn max_candidate_line(&self, result: Option<&LookupResult>, index: usize) -> usize {
        result.map_or(0, |result| {
            let lines = candidate_lines(result, index, self.viewport.0).len();
            if lines > self.viewport.1.max(1) as usize {
                lines.saturating_sub(1)
            } else {
                0
            }
        })
    }

    fn align_details(&mut self, heights: &[usize], action: Option<Action>) {
        let selected = self.selection.selected().unwrap_or(0);
        let Some(&lines) = heights.get(selected) else {
            self.scroll = 0;
            self.line = 0;
            return;
        };
        let height = self.viewport.1.max(1) as usize;
        self.line = if lines > height {
            self.line.min(lines.saturating_sub(1))
        } else {
            0
        };
        let start: usize = heights[..selected].iter().sum();
        let cursor = start + self.line;
        let max_scroll = heights.iter().sum::<usize>().saturating_sub(height);
        // Keep the viewport still until the selection crosses the directional
        // margin. Reversing direction moves the selection through the pane first.
        let lower = (height - 1) * 7 / 10;
        let upper = (height - 1) * 3 / 10;
        match action {
            Some(Action::Down | Action::PageDown) => {
                self.scroll = self.scroll.max(cursor.saturating_sub(lower));
            }
            Some(Action::Up | Action::PageUp) => {
                self.scroll = self.scroll.min(cursor.saturating_sub(upper));
            }
            Some(Action::Home) => self.scroll = 0,
            Some(Action::End) => self.scroll = max_scroll,
            _ => {}
        }
        // Also keep selection visible after resizing or replacing results.
        self.scroll = self.scroll.min(cursor);
        let visible_end = if lines <= height {
            start + lines
        } else {
            cursor + 1
        };
        self.scroll = self.scroll.max(visible_end.saturating_sub(height));
        self.scroll = self.scroll.min(max_scroll);
    }

    pub(super) fn navigate(&mut self, result: Option<&LookupResult>, action: Action) {
        let count = result.map_or(0, |result| result.candidates.len());
        if count == 0 {
            return;
        }
        let selected = self.selection.selected().unwrap_or(0).min(count - 1);
        let max = self.max_candidate_line(result, selected);
        let step = if matches!(action, Action::PageUp | Action::PageDown) {
            5
        } else {
            1
        };
        match action {
            Action::Down | Action::PageDown if self.line < max => {
                self.line = (self.line + step).min(max);
            }
            Action::Up | Action::PageUp if self.line > 0 => {
                self.line = self.line.saturating_sub(step);
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
                    self.line = self.max_candidate_line(result, previous);
                }
            }
            Action::Home => self.move_candidate(Some(0)),
            Action::End => {
                self.move_candidate(Some(count - 1));
                self.line = self.max_candidate_line(result, count - 1);
            }
            _ => return,
        }
        let result = result.expect("candidate navigation requires a result");
        let heights = result
            .candidates
            .iter()
            .enumerate()
            .map(|(index, _)| candidate_lines(result, index, self.viewport.0).len())
            .collect::<Vec<_>>();
        self.align_details(&heights, Some(action));
    }

    pub(super) fn draw(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        block: Block<'static>,
        content: DetailsContent<'_>,
    ) {
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let (entry, result) = match content {
            DetailsContent::Lookup {
                result,
                loading,
                problem,
            } => {
                if loading {
                    frame.render_widget(Paragraph::new("Looking up… Esc cancels."), inner);
                    return;
                }
                if let Some(problem) = problem {
                    frame.render_widget(
                        Paragraph::new(safe_text(problem)).wrap(Wrap { trim: false }),
                        inner,
                    );
                    return;
                }
                (None, result)
            }
            DetailsContent::Saved(entry) => (entry, entry.and_then(HistoryEntry::result)),
        };
        let Some(result) = result else {
            let text = entry
                .map(|e| {
                    format!(
                        "{}\n{}\nStarted {}\n{}",
                        e.query,
                        e.status(),
                        display_time(e.started_at),
                        e.finished
                            .as_ref()
                            .and_then(|finished| finished.message.as_deref())
                            .unwrap_or_default()
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
        self.viewport = (parts[1].width, parts[1].height);
        let selected = self
            .selection
            .selected()
            .unwrap_or(0)
            .min(result.candidates.len().saturating_sub(1));
        if self.selection.selected() != Some(selected) {
            self.select_candidate(Some(selected));
        }
        let candidates = result
            .candidates
            .iter()
            .enumerate()
            .map(|(index, _)| candidate_lines(result, index, parts[1].width))
            .collect::<Vec<_>>();
        let heights = candidates.iter().map(Vec::len).collect::<Vec<_>>();
        self.align_details(&heights, None);
        let selected_line = self.line;
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
            .skip(self.scroll)
            .take(parts[1].height as usize)
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(lines), parts[1]);
        frame.render_widget(
            Paragraph::new(safe_text(result.attribution.as_deref().unwrap_or("")))
                .wrap(Wrap { trim: false }),
            parts[2],
        );
    }
}

impl App {
    pub(super) fn navigate_details(&mut self, action: Action) {
        let result = match self.tab {
            Tab::Lookup => self.lookup.result(),
            Tab::History => self.history.selected().and_then(HistoryEntry::result),
        };
        self.details.navigate(result, action);
    }

    pub(super) fn draw_details(&mut self, frame: &mut Frame, area: Rect) {
        self.focus_regions
            .push((area, FocusTarget::Pane(Focus::Details)));
        let title = if self.selected().is_some() {
            " Saved encounter "
        } else {
            " Lookup "
        };
        let block = self.block(title, Focus::Details);
        let content = match self.tab {
            Tab::Lookup if self.lookup.preview.is_none() => DetailsContent::Lookup {
                result: self.lookup.live.as_ref(),
                loading: self.lookup.loading,
                problem: self.lookup.problem.as_deref(),
            },
            Tab::Lookup => DetailsContent::Saved(self.lookup.preview.as_ref()),
            Tab::History => DetailsContent::Saved(self.history.selected()),
        };
        self.details.draw(frame, area, block, content);
    }
}

fn candidate_lines(result: &LookupResult, index: usize, width: u16) -> Vec<String> {
    let Some(candidate) = result.candidates.get(index) else {
        return vec![];
    };
    let value = crate::presentation::values(result, Some(index)).unwrap_or_default();
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
