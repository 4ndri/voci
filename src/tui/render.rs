//! Tab layouts, footer hints, and shared input/border widgets.

use super::input::{Input, InputMode};
use super::{App, Focus, Tab};
use crate::{text::safe_text, tui::keybindings::Action};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
};

impl App {
    pub(super) fn block(&self, title: &str, focus: Focus) -> Block<'static> {
        let number = self
            .tab
            .panes()
            .iter()
            .position(|pane| *pane == focus)
            .expect("pane titles belong to the active tab") as u8
            + 1;
        bordered(
            &format!(
                " [{}] {} ",
                self.bindings.label(Action::FocusPane(number)),
                title.trim()
            ),
            self.focus == focus,
        )
    }

    pub(super) fn draw(&mut self, frame: &mut Frame) {
        self.focus_regions.clear();
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
        frame.render_widget(Paragraph::new(self.footer()), rows[2]);
        if self.history.dialog.is_some() {
            self.draw_dialog(frame, area);
        }
    }

    fn footer(&self) -> String {
        let mode = if self.pane_mode {
            "PANE"
        } else {
            self.focused_input()
                .map_or("NORMAL", |input| input.mode().label())
        };
        if self.pane_mode {
            format!(
                "PANE · {} left / {} right · {} up / {} down\nTitle shortcuts focus · Enter, Esc or {} finishes\n{}",
                self.bindings.label(Action::Left),
                self.bindings.label(Action::Right),
                self.bindings.label(Action::Up),
                self.bindings.label(Action::Down),
                self.bindings.label(Action::Pane),
                safe_text(&self.notice)
            )
        } else if let Some(input) = self.focused_input() {
            let actions = match input.mode() {
                InputMode::Insert if self.has_suggestions() => format!(
                    "↑/↓ history · {} {} · Esc dismiss",
                    self.bindings.label(Action::Submit),
                    if self.lookup.suggestion_state.selected().is_some() {
                        "opens saved result"
                    } else {
                        "new lookup"
                    }
                ),
                InputMode::Insert => format!(
                    "{} submits · Esc normal",
                    self.bindings.label(Action::Submit)
                ),
                InputMode::Normal => format!(
                    "{}/{} insert · {} append · {}/{} paste after/before · {} select · {} cut line",
                    self.bindings.label(Action::Submit),
                    self.bindings.label(Action::Edit),
                    self.bindings.label(Action::Append),
                    self.bindings.label(Action::Paste),
                    self.bindings.label(Action::PasteBefore),
                    self.bindings.label(Action::Visual),
                    self.bindings.label(Action::DeleteLine)
                ),
                InputMode::Visual => format!(
                    "{} copy · {} cut · {} change · {} replace · Esc normal",
                    self.bindings.label(Action::YankSelection),
                    self.bindings.label(Action::DeleteSelection),
                    self.bindings.label(Action::ChangeSelection),
                    self.bindings.label(Action::Paste)
                ),
            };
            let motions = if input.mode() == InputMode::Insert {
                format!(
                    "Ctrl-Left/Right word · {} panes · {} field",
                    self.bindings.label(Action::Pane),
                    self.bindings.label(Action::NextFocus)
                )
            } else if input.mode() == InputMode::Normal {
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
        }
    }
}

pub(super) fn bordered(title: &str, focused: bool) -> Block<'static> {
    Block::default()
        .title(title.to_owned())
        .borders(Borders::ALL)
        .border_style(if focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        })
}

pub(super) fn draw_input(
    frame: &mut Frame,
    input: &Input,
    area: Rect,
    block: Block<'_>,
    focused: bool,
) {
    let inner = block.inner(area);
    let (visible, cursor) = input.visible(inner.width as usize);
    frame.render_widget(Paragraph::new(visible).block(block), area);
    if focused && inner.width > 0 && inner.height > 0 {
        frame.set_cursor_position((inner.x + cursor, inner.y));
    }
}
