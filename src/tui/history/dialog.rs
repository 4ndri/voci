//! History filter controls and their modal overlay.

use crate::tui::{App, Effect, FocusTarget, HistoryRead};
use crate::tui::{
    input::{Input, InputMode},
    render::{bordered, draw_input},
};
use crate::{
    history::HistoryFilter,
    tui::keybindings::{Action, Context},
};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    widgets::{Block, Borders, Clear, Paragraph},
};
use std::time::Instant;

impl App {
    pub(in crate::tui) fn dialog_event(&mut self, key: KeyEvent) -> Effect {
        let dialog = self
            .history
            .dialog
            .as_mut()
            .expect("dialog events require an open dialog");
        if dialog.field == 1 && key.code == KeyCode::Char(' ') && key.modifiers.is_empty() {
            dialog.today = !dialog.today;
            self.resolver.reset();
            return Effect::None;
        }
        let context = if dialog.field != 0 {
            Context::Dialog
        } else if dialog.text.mode() == InputMode::Visual {
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
        if dialog.field == 0
            && !matches!(
                action,
                Action::Pane
                    | Action::FocusPane(_)
                    | Action::Cancel
                    | Action::NextFocus
                    | Action::PreviousFocus
            )
        {
            return dialog.text.action(action).into();
        }
        self.dialog_control(action)
    }

    pub(in crate::tui) fn dialog_control(&mut self, action: Action) -> Effect {
        let dialog = self
            .history
            .dialog
            .as_mut()
            .expect("dialog controls require an open dialog");
        match action {
            Action::Pane => self.toggle_panes(),
            Action::FocusPane(number) => self.focus_pane(number),
            Action::Cancel => return self.escape(),
            Action::NextFocus | Action::PreviousFocus => {
                dialog.text.set_mode(InputMode::Normal);
                dialog.field = (dialog.field + if action == Action::NextFocus { 1 } else { 4 }) % 5;
                self.resolver.reset();
            }
            Action::Left | Action::Right if dialog.field == 1 => dialog.today = !dialog.today,
            Action::Submit => {
                if dialog.field == 3 {
                    dialog.text = Input::default();
                    dialog.today = false;
                    return Effect::None;
                }
                if dialog.field == 4 {
                    self.history.dialog = None;
                    return Effect::None;
                }
                self.history.filter = HistoryFilter {
                    text: dialog.text.text(),
                    today: dialog.today,
                };
                self.history.dialog = None;
                self.history.entries.clear();
                self.history.state.select(None);
                self.select_candidate(None);
                self.resolver.reset();
                return Effect::Read(HistoryRead::default());
            }
            _ => {}
        }
        Effect::None
    }

    pub(in crate::tui) fn draw_dialog(&mut self, frame: &mut Frame, area: Rect) {
        // A modal dialog owns all focus; clicks outside its controls do nothing.
        self.focus_regions.clear();
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
        let dialog = self
            .history
            .dialog
            .as_ref()
            .expect("dialog rendering requires an open dialog");
        let field_block = |title: &str, field| {
            bordered(
                &format!(
                    " [{}] {}",
                    self.bindings.label(Action::FocusPane(field as u8 + 1)),
                    title.trim()
                ),
                dialog.field == field,
            )
        };
        self.focus_regions.extend([
            (rows[0], FocusTarget::Dialog(0)),
            (rows[1], FocusTarget::Dialog(1)),
        ]);
        let block = field_block(" Text ", 0);
        draw_input(
            frame,
            &dialog.text,
            rows[0],
            block,
            dialog.field == 0 && !self.pane_mode,
        );
        frame.render_widget(
            Paragraph::new(if dialog.today { "Today" } else { "All history" })
                .block(field_block(" Date ", 1)),
            rows[1],
        );
        // Give the longer Cancel label enough room at the minimum 24-column size.
        let buttons =
            Layout::horizontal([Constraint::Min(7), Constraint::Min(7), Constraint::Min(8)])
                .split(rows[2]);
        for (index, label) in ["Apply", "Clear", "Cancel"].into_iter().enumerate() {
            self.focus_regions
                .push((buttons[index], FocusTarget::Dialog(index + 2)));
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
