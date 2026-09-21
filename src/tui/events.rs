//! Resolve terminal events into state changes and effects for the runtime.

use super::input::{Input, InputMode, SelectionAction};
use super::{App, Dialog, Effect, Focus, FocusTarget, Tab};
use crate::{
    text::safe_text,
    tui::clipboard::Clipboard,
    tui::keybindings::{Action, Context},
};
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use std::time::Instant;

impl App {
    pub(super) fn focused_input(&self) -> Option<&Input> {
        if let Some(dialog) = &self.history.dialog {
            (dialog.field == 0).then_some(&dialog.text)
        } else {
            (self.tab == Tab::Lookup && self.focus == Focus::Input).then_some(&self.lookup.input)
        }
    }

    pub(super) fn focused_input_mut(&mut self) -> Option<&mut Input> {
        if let Some(dialog) = &mut self.history.dialog {
            (dialog.field == 0).then_some(&mut dialog.text)
        } else {
            (self.tab == Tab::Lookup && self.focus == Focus::Input)
                .then_some(&mut self.lookup.input)
        }
    }

    pub(super) fn toggle_panes(&mut self) {
        self.pane_mode = !self.pane_mode;
        if let Some(input) = self.focused_input_mut() {
            input.set_mode(InputMode::Normal);
        }
        self.resolver.reset();
    }

    pub(super) fn escape(&mut self) -> Effect {
        if self.pane_mode {
            self.pane_mode = false;
            self.resolver.reset();
            return Effect::None;
        }
        if self.resolver.pending() {
            self.resolver.reset();
            return Effect::None;
        }
        if let Some(dialog) = &mut self.history.dialog {
            if dialog.field == 0 && dialog.text.mode() != InputMode::Normal {
                dialog.text.set_mode(InputMode::Normal);
            } else {
                self.history.dialog = None;
            }
            return Effect::None;
        }
        if self.lookup.loading {
            self.lookup.generation += 1;
            self.lookup.loading = false;
            self.lookup.problem = Some("Lookup cancelled.".into());
            return Effect::Cancel;
        }
        if self.lookup.input.mode() != InputMode::Normal {
            self.lookup.input.set_mode(InputMode::Normal);
        } else {
            self.lookup.preview = None;
            self.select_candidate(Some(0));
        }
        Effect::None
    }

    pub(super) fn event(&mut self, event: Event) -> Effect {
        match event {
            Event::Mouse(mouse) => self.mouse_event(mouse),
            Event::Key(key) => self.key_event(key),
            Event::Paste(text) => {
                if !self.pane_mode
                    && let Some(input) = self.focused_input_mut()
                    && let Err(error) = input.paste(&text)
                {
                    self.notice = error;
                }
                Effect::None
            }
            _ => Effect::None,
        }
    }

    fn mouse_event(&mut self, mouse: MouseEvent) -> Effect {
        if mouse.kind == MouseEventKind::Down(MouseButton::Left)
            && let Some((_, target)) = self
                .focus_regions
                .iter()
                .rev()
                .find(|(area, _)| area.contains((mouse.column, mouse.row).into()))
        {
            let target = *target;
            match target {
                FocusTarget::Pane(focus) if self.history.dialog.is_none() => {
                    if self.focus != focus {
                        self.set_focus(focus);
                    }
                }
                FocusTarget::Dialog(field) => {
                    if let Some(dialog) = &mut self.history.dialog
                        && dialog.field != field
                    {
                        dialog.text.set_mode(InputMode::Normal);
                        dialog.field = field;
                    }
                }
                _ => return Effect::None,
            }
            self.pane_mode = false;
            self.resolver.reset();
        }
        Effect::None
    }

    fn key_event(&mut self, key: KeyEvent) -> Effect {
        if key.kind == KeyEventKind::Release {
            return Effect::None;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Effect::Quit;
        }
        let text_editing = self
            .focused_input()
            .is_some_and(|input| input.mode() == InputMode::Insert);
        if key.code == KeyCode::Esc {
            return self.escape();
        }
        if self.pane_mode {
            return self.pane_key(key);
        }
        if text_editing {
            if let Some(action) =
                self.resolver
                    .feed(&self.bindings, key, Context::Insert, Instant::now())
            {
                return self.input_command(action);
            }
            if !self.resolver.pending() {
                self.focused_input_mut()
                    .expect("insert mode requires a focused input")
                    .edit(key);
            }
            return Effect::None;
        }
        if self.history.dialog.is_some() {
            return self.dialog_event(key);
        }
        let context = match self.focused_input().map(|i| i.mode()) {
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
        self.apply_action(action)
    }

    fn pane_key(&mut self, key: KeyEvent) -> Effect {
        if key.code == KeyCode::Enter {
            self.toggle_panes();
            return Effect::None;
        }
        if let Some(action) = self
            .resolver
            .feed(&self.bindings, key, Context::Pane, Instant::now())
        {
            if action == Action::Pane {
                self.toggle_panes();
            } else if let Action::FocusPane(number) = action {
                self.focus_pane(number);
            } else if action == Action::Cancel {
                return self.escape();
            } else if matches!(
                action,
                Action::Left | Action::Right | Action::Up | Action::Down
            ) {
                self.move_pane(action);
            }
        }
        Effect::None
    }

    fn apply_action(&mut self, action: Action) -> Effect {
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
                    | Action::PasteBefore
                    | Action::WordBegin
                    | Action::WordEnd
                    | Action::Visual
                    | Action::YankSelection
                    | Action::DeleteSelection
                    | Action::DeleteLine
                    | Action::ChangeSelection
            )
        {
            let effect = input.action(action).into();
            if matches!(
                action,
                Action::Edit | Action::Append | Action::Submit | Action::Undo | Action::Redo
            ) {
                self.lookup.preview = None;
            }
            return effect;
        }
        match action {
            Action::FocusPane(number) => {
                self.focus_pane(number);
                Effect::None
            }
            Action::Pane => {
                self.toggle_panes();
                Effect::None
            }
            Action::Cancel => self.escape(),
            Action::Quit => Effect::Quit,
            Action::NextTab | Action::PreviousTab => {
                self.resolver.reset();
                self.lookup.input.set_mode(InputMode::Normal);
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
                self.lookup.input.set_mode(InputMode::Insert);
                self.lookup.preview = None;
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
                let text = Input::with_text(&self.history.filter.text);
                self.history.dialog = Some(Dialog {
                    text,
                    today: self.history.filter.today,
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
                        crate::presentation::values(
                            r,
                            if action == Action::CopyValue {
                                Some(self.details.selection.selected().unwrap_or(0))
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

    pub(super) fn clipboard_effect(&mut self, clipboard: &mut impl Clipboard, effect: Effect) {
        match effect {
            Effect::Paste(position) => match clipboard.paste() {
                Ok(text) => {
                    if let Some(input) = self.focused_input_mut() {
                        self.notice = match input.paste_at(&text, position) {
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
                            SelectionAction::Yank => input.set_mode(InputMode::Normal),
                            SelectionAction::Cut => input.cut(),
                            SelectionAction::CutLine => input.cut_line(),
                            SelectionAction::Change => input.change_selection(),
                        }
                    }
                    self.notice = match action {
                        SelectionAction::Yank => "Selection copied to clipboard.",
                        SelectionAction::Cut | SelectionAction::CutLine => "Text cut to clipboard.",
                        SelectionAction::Change => "Selection cut to clipboard; insert mode.",
                    }
                    .into();
                }
                Err(error) => self.notice = error,
            },
            _ => {}
        }
    }

    // Command dispatch is shared by insert-mode lookup and filter fields.
    pub(super) fn input_command(&mut self, action: Action) -> Effect {
        if self.has_suggestions() {
            match action {
                Action::Up | Action::Down => {
                    let selected = self.lookup.suggestion_state.selected();
                    self.lookup
                        .suggestion_state
                        .select(match (action, selected) {
                            (Action::Down, None) => Some(0),
                            (Action::Down, Some(i)) if i + 1 < self.lookup.suggestions.len() => {
                                Some(i + 1)
                            }
                            (Action::Up, Some(i)) if i > 0 => Some(i - 1),
                            (Action::Up, None) => Some(self.lookup.suggestions.len() - 1),
                            _ => None,
                        });
                    return Effect::None;
                }
                Action::Submit => {
                    if let Some(entry) = self
                        .lookup
                        .suggestion_state
                        .selected()
                        .and_then(|i| self.lookup.suggestions.get(i))
                        .cloned()
                    {
                        self.lookup.input = Input::with_text(&entry.query);
                        self.lookup.preview = Some(entry);
                        self.lookup.input.set_mode(InputMode::Normal);
                        self.focus = Focus::Details;
                        self.select_candidate(Some(0));
                        self.resolver.reset();
                        return Effect::None;
                    }
                }
                _ => {}
            }
        }
        match action {
            Action::Pane => self.toggle_panes(),
            Action::FocusPane(number) => self.focus_pane(number),
            Action::Cancel => return self.escape(),
            Action::WordBegin | Action::WordEnd => {
                return self
                    .focused_input_mut()
                    .expect("input commands require a focused input")
                    .action(action)
                    .into();
            }
            _ if self.history.dialog.is_some() => return self.dialog_control(action),
            Action::Submit => return Effect::Submit,
            Action::NextFocus => self.move_focus(1),
            Action::PreviousFocus => self.move_focus(-1),
            _ => {}
        }
        Effect::None
    }
}
