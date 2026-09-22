//! Compose feature-owned panes with shell focus, input routing, and effects.

use crate::lookup::LookupRequest;

mod clipboard;
mod details;
mod events;
mod history;
mod input;
mod keybindings;
mod lookup;
mod navigation;
mod render;
mod runtime;
#[cfg(test)]
mod tests;

pub(crate) use keybindings::Keybindings;
pub(crate) use runtime::{check_terminal, run};

use self::input::{InputEffect, InputMode, PastePosition, SelectionAction};
use self::{
    details::DetailsPane,
    history::{Dialog, HistoryPane},
    lookup::LookupPane,
};
use crate::{
    app::Completion,
    domain::{Language, LookupResult},
    history::{Cursor, HistoryEntry, HistoryPage},
    tui::keybindings::Resolver,
};
use ratatui::layout::Rect;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tab {
    Lookup,
    History,
}

impl Tab {
    fn panes(self) -> &'static [Focus] {
        match self {
            Self::Lookup => &[
                Focus::Input,
                Focus::Source,
                Focus::Target,
                Focus::Details,
                Focus::Recent,
            ],
            Self::History => &[Focus::History, Focus::Details],
        }
    }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FocusTarget {
    Pane(Focus),
    Dialog(usize),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HistoryTarget {
    Recent,
    History,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum HistorySelection {
    #[default]
    First,
    Last,
    Preserve,
}

#[derive(Clone, Copy, Default)]
struct HistoryRead {
    cursor: Option<Cursor>,
    oldest: bool,
    selection: HistorySelection,
}

enum Effect {
    None,
    Submit,
    Cancel,
    Quit,
    Read(HistoryRead),
    Copy(String),
    CopySelection(String, SelectionAction),
    Paste(PastePosition),
}

impl From<InputEffect> for Effect {
    fn from(effect: InputEffect) -> Self {
        match effect {
            InputEffect::None => Self::None,
            InputEffect::Paste(position) => Self::Paste(position),
            InputEffect::CopySelection(text, action) => Self::CopySelection(text, action),
        }
    }
}

struct App {
    tab: Tab,
    focus: Focus,
    pane_mode: bool,
    lookup: LookupPane,
    history: HistoryPane,
    details: DetailsPane,
    notice: String,
    bindings: Keybindings,
    resolver: Resolver,
    // Recorded in paint order so overlays own hit testing.
    focus_regions: Vec<(Rect, FocusTarget)>,
}

impl App {
    fn new(from: Option<Language>, to: Option<Language>, bindings: Keybindings) -> Self {
        Self {
            tab: Tab::Lookup,
            focus: Focus::Input,
            pane_mode: false,
            lookup: LookupPane::new(from, to),
            history: HistoryPane::default(),
            details: DetailsPane::default(),
            notice: String::new(),
            bindings,
            resolver: Resolver::default(),
            focus_regions: Vec::new(),
        }
    }

    fn selected(&self) -> Option<&HistoryEntry> {
        match self.tab {
            Tab::Lookup => self.lookup.preview.as_ref(),
            Tab::History => self.history.selected(),
        }
    }

    fn result(&self) -> Option<&LookupResult> {
        match self.tab {
            Tab::Lookup => self.lookup.result(),
            Tab::History => self.history.selected().and_then(HistoryEntry::result),
        }
    }

    fn update_suggestion_query(&mut self) -> bool {
        let active = self.tab == Tab::Lookup
            && self.focus == Focus::Input
            && self.history.dialog.is_none()
            && !self.pane_mode;
        self.lookup.update_suggestion_query(active)
    }

    fn apply_suggestions(&mut self, generation: u64, result: Result<HistoryPage, String>) {
        self.lookup
            .apply_suggestions(generation, result, &mut self.notice);
    }

    fn has_suggestions(&self) -> bool {
        self.tab == Tab::Lookup
            && self.focus == Focus::Input
            && self.lookup.input.mode() == InputMode::Insert
            && self.history.dialog.is_none()
            && !self.pane_mode
            && !self.lookup.loading
            && !self.lookup.suggestions.is_empty()
    }

    fn begin(&mut self) -> (u64, LookupRequest) {
        self.lookup.begin()
    }

    fn complete(&mut self, id: u64, completion: Completion) {
        if let Some(notice) = self.lookup.complete(id, completion) {
            self.notice = notice;
            if self.lookup.live.is_some() {
                self.details.select_candidate(Some(0));
            }
        }
    }

    fn apply_history(
        &mut self,
        generation: u64,
        target: HistoryTarget,
        selection: HistorySelection,
        result: Result<HistoryPage, String>,
    ) {
        match target {
            HistoryTarget::Recent => self
                .lookup
                .apply_recent(generation, result, &mut self.notice),
            HistoryTarget::History => {
                if self
                    .history
                    .apply(generation, selection, result, &mut self.notice)
                {
                    self.details.select_candidate(Some(0));
                }
            }
        }
    }

    fn refresh(&self) -> Effect {
        Effect::Read(self.history.refresh())
    }

    fn select_candidate(&mut self, selected: Option<usize>) {
        self.details.select_candidate(selected);
    }
}
