//! Move focus and selection through panes, languages, and saved encounters.

use super::input::InputMode;
use super::{App, Effect, Focus, Tab};
use crate::tui::keybindings::Action;

impl App {
    pub(super) fn move_pane(&mut self, action: Action) {
        if let Some(dialog) = &mut self.history.dialog {
            dialog.text.set_mode(InputMode::Normal);
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

    pub(super) fn move_focus(&mut self, delta: isize) {
        let order = self.tab.panes();
        let i = order.iter().position(|f| *f == self.focus).unwrap_or(0);
        self.set_focus(order[(i as isize + delta).rem_euclid(order.len() as isize) as usize]);
    }

    pub(super) fn focus_pane(&mut self, number: u8) {
        if !(1..=5).contains(&number) {
            return;
        }
        if let Some(dialog) = &mut self.history.dialog {
            dialog.text.set_mode(InputMode::Normal);
            dialog.field = usize::from(number - 1);
            self.resolver.reset();
        } else if let Some(&focus) = self.tab.panes().get(usize::from(number - 1)) {
            self.set_focus(focus);
        }
    }

    pub(super) fn set_focus(&mut self, focus: Focus) {
        self.focus = focus;
        self.lookup.input.set_mode(InputMode::Normal);
        self.resolver.reset();
        if self.focus == Focus::Recent && !self.lookup.recent.is_empty() {
            self.lookup
                .recent_state
                .select(Some(self.lookup.recent_state.selected().unwrap_or(0)));
            self.lookup.preview = self
                .lookup
                .recent
                .get(self.lookup.recent_state.selected().unwrap_or(0))
                .cloned();
            self.select_candidate(Some(0));
        }
    }

    pub(super) fn navigate(&mut self, action: Action) -> Effect {
        match self.focus {
            Focus::Details => {
                self.navigate_details(action);
                Effect::None
            }
            Focus::Source | Focus::Target => {
                self.lookup.navigate_language(self.focus, action);
                Effect::None
            }
            Focus::Recent => {
                if self.lookup.navigate_recent(action) {
                    self.select_candidate(Some(0));
                }
                Effect::None
            }
            Focus::History => {
                let has_entries = !self.history.entries.is_empty();
                let effect = self.history.navigate(action);
                if has_entries && matches!(effect, Effect::None) {
                    self.select_candidate(Some(0));
                }
                effect
            }
            _ => Effect::None,
        }
    }
}

pub(super) fn next_index(action: Action, current: usize, count: usize) -> usize {
    match action {
        Action::Up => current.saturating_sub(1),
        Action::Down => (current + 1).min(count - 1),
        Action::PageUp => current.saturating_sub(5),
        Action::PageDown => (current + 5).min(count - 1),
        Action::Home => 0,
        Action::End => count - 1,
        _ => current,
    }
}
