//! Single-line edtui adapter shared by lookup and history filters.
//! The application resolves configurable keys and performs fallible clipboard I/O.
//! edtui owns the buffer, modes, selections and editing actions. Its scalar offsets
//! are adapted to graphemes so combining accents and emoji remain indivisible.
use crate::{keybindings::Action, presentation::safe_text};
use crossterm::{
    cursor::SetCursorStyle,
    event::{KeyCode, KeyEvent, KeyModifiers},
};
use edtui::{
    EditorMode, EditorState, Index2, Lines,
    actions::{self, Execute},
};
use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

pub(super) enum InputEffect {
    None,
    Paste,
    CopySelection(String, SelectionAction),
}

#[derive(Clone, Copy)]
pub(super) enum SelectionAction {
    Yank,
    Cut,
    Change,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum InputMode {
    Normal,
    #[default]
    Insert,
    Visual,
}
impl InputMode {
    pub(super) fn cursor_style(self) -> SetCursorStyle {
        match self {
            Self::Insert => SetCursorStyle::SteadyBar,
            Self::Normal | Self::Visual => SetCursorStyle::SteadyBlock,
        }
    }
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Normal => "NORMAL",
            Self::Insert => "INSERT",
            Self::Visual => "VISUAL",
        }
    }
}
pub(super) struct Input {
    state: EditorState,
    insert_group: bool,
    undo_available: usize,
    redo_available: usize,
}
impl Default for Input {
    fn default() -> Self {
        let mut state = EditorState::default();
        state.set_single_line(true);
        state.mode = EditorMode::Insert;
        Self {
            state,
            insert_group: false,
            undo_available: 0,
            redo_available: 0,
        }
    }
}
impl Input {
    pub(super) fn with_text(text: &str) -> Self {
        let mut input = Self::default();
        let text = safe_text(text);
        input.state.lines = Lines::from(text.as_str());
        input.set_cursor(text.len());
        input
    }
    // Execute a whole edit on a working buffer. edtui's individual deletion
    // actions capture their own checkpoints; discard those intermediate snapshots
    // and capture just one application-level change in the persistent editor.
    fn change(&mut self, edit: impl FnOnce(&mut EditorState)) {
        let mut working = EditorState::new(self.state.lines.clone());
        working.set_single_line(true);
        working.cursor = self.state.cursor;
        working.mode = self.state.mode;
        working.selection = self.state.selection.clone();
        edit(&mut working);
        if working.lines != self.state.lines {
            if !self.insert_group {
                // edtui 0.11 has no public checkpoint method. A zero-length
                // DeleteChar captures the buffer/cursor without changing either.
                self.state.execute(actions::DeleteChar(0));
                self.undo_available = (self.undo_available + 1).min(100);
            }
            self.insert_group = working.mode == EditorMode::Insert;
            // edtui 0.11 does not clear its redo stack on new changes. Gate access
            // to that stack so abandoned branches can never be replayed.
            self.redo_available = 0;
        }
        self.state.lines = working.lines;
        self.state.cursor = working.cursor;
        self.state.mode = working.mode;
        self.state.selection = working.selection;
        self.align_cursor();
    }
    fn restore_change(&mut self, redo: bool) {
        if redo && self.redo_available > 0 {
            self.state.execute(actions::Redo);
            self.redo_available -= 1;
            self.undo_available += 1;
        } else if !redo && self.undo_available > 0 {
            self.state.execute(actions::Undo);
            self.undo_available -= 1;
            self.redo_available += 1;
        }
        self.mode(InputMode::Normal);
        self.align_cursor();
    }
    pub(super) fn text(&self) -> String {
        self.state.lines.to_string()
    }
    pub(super) fn cursor(&self) -> usize {
        self.text()
            .chars()
            .take(self.state.cursor.col)
            .map(char::len_utf8)
            .sum()
    }
    pub(super) fn get_mode(&self) -> InputMode {
        match self.state.mode {
            EditorMode::Insert => InputMode::Insert,
            EditorMode::Visual => InputMode::Visual,
            _ => InputMode::Normal,
        }
    }
    pub(super) fn mode(&mut self, mode: InputMode) {
        // Voci keeps the insertion position on Escape, including the end gap.
        let cursor = self.state.cursor;
        if mode != InputMode::Insert {
            self.insert_group = false;
        }
        // Checkpoints are created on actual edits, not mode changes.
        if mode == InputMode::Insert {
            self.state.mode = EditorMode::Insert;
        }
        self.state.execute(actions::SwitchMode(match mode {
            InputMode::Normal => EditorMode::Normal,
            InputMode::Insert => EditorMode::Insert,
            InputMode::Visual => EditorMode::Visual,
        }));
        self.state.cursor = cursor;
        if mode == InputMode::Visual {
            if let Some(selection) = &mut self.state.selection {
                selection.start = cursor;
                selection.end = cursor;
            }
        } else {
            self.state.selection = None;
        }
    }
    pub(super) fn set_cursor(&mut self, byte: usize) {
        let text = self.text();
        let byte = text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .chain(std::iter::once(text.len()))
            .find(|i| *i >= byte)
            .unwrap_or(text.len());
        self.state.cursor = Index2::new(0, text[..byte].chars().count());
        if let Some(selection) = &mut self.state.selection {
            selection.end = self.state.cursor;
        }
    }
    fn align_cursor(&mut self) {
        self.set_cursor(self.cursor());
    }
    pub(super) fn selection(&self) -> Option<std::ops::Range<usize>> {
        let selection = self.state.selection.as_ref()?;
        let text = self.text();
        let start = text
            .chars()
            .take(selection.start().col)
            .map(char::len_utf8)
            .sum();
        let end: usize = text
            .chars()
            .take(selection.end().col)
            .map(char::len_utf8)
            .sum();
        Some(start..end + text[end..].graphemes(true).next().map_or(0, str::len))
    }
    pub(super) fn cut(&mut self) {
        if !self.cut_selection(EditorMode::Normal) {
            self.delete();
        }
    }
    pub(super) fn change_selection(&mut self) {
        self.cut_selection(EditorMode::Insert);
    }
    fn cut_selection(&mut self, mode: EditorMode) -> bool {
        if let Some(range) = self.selection().filter(|r| !r.is_empty()) {
            let text = self.text();
            self.change(|state| {
                // Expand edtui's scalar selection to whole graphemes.
                if let Some(selection) = &mut state.selection {
                    selection.start = Index2::new(0, text[..range.start].chars().count());
                    selection.end = Index2::new(0, text[..range.end].chars().count() - 1);
                }
                state.execute(actions::DeleteSelection);
                // Change starts an insert session in the same undo group as the cut.
                state.mode = mode;
            });
            true
        } else {
            false
        }
    }
    pub(super) fn paste(&mut self, text: &str) -> Result<(), String> {
        if text.chars().any(char::is_control) {
            return Err("Paste must contain a single line without control characters.".into());
        }
        let text = safe_text(text);
        if !text.is_empty() {
            let original = self.text();
            let selection = self.selection();
            self.change(|state| {
                if let Some(range) = selection.filter(|r| !r.is_empty()) {
                    if let Some(selection) = &mut state.selection {
                        selection.start = Index2::new(0, original[..range.start].chars().count());
                        selection.end = Index2::new(0, original[..range.end].chars().count() - 1);
                    }
                    state.execute(actions::DeleteSelection);
                }
                if state.mode == EditorMode::Visual {
                    state.mode = EditorMode::Normal;
                    state.selection = None;
                }
                for c in text.chars() {
                    state.execute(actions::InsertChar(c));
                }
            });
        }
        Ok(())
    }
    pub(super) fn action(&mut self, action: Action) -> InputEffect {
        match action {
            Action::Edit | Action::Submit => self.mode(InputMode::Insert),
            Action::Append => {
                self.right();
                self.mode(InputMode::Insert);
            }
            Action::Undo => self.restore_change(false),
            Action::Redo => self.restore_change(true),
            Action::Left => self.left(),
            Action::Right => self.right(),
            Action::WordBegin => self.word_begin(),
            Action::WordEnd => self.word_end(),
            Action::Home => self.set_cursor(0),
            Action::End => self.set_cursor(self.text().len()),
            Action::Paste => return InputEffect::Paste,
            Action::Visual => self.mode(if self.get_mode() == InputMode::Visual {
                InputMode::Normal
            } else {
                InputMode::Visual
            }),
            Action::YankSelection | Action::DeleteSelection | Action::ChangeSelection => {
                let text = self.text();
                let cursor = self.cursor();
                let range = self.selection().or_else(|| {
                    (action == Action::DeleteSelection).then(|| {
                        cursor..cursor + text[cursor..].graphemes(true).next().map_or(0, str::len)
                    })
                });
                if let Some(range) = range.filter(|r| !r.is_empty()) {
                    return InputEffect::CopySelection(
                        text[range].to_owned(),
                        match action {
                            Action::ChangeSelection => SelectionAction::Change,
                            Action::DeleteSelection => SelectionAction::Cut,
                            _ => SelectionAction::Yank,
                        },
                    );
                }
            }
            _ => {}
        }
        InputEffect::None
    }
    pub(super) fn insert(&mut self, text: &str) {
        self.change(|state| {
            for c in safe_text(text).chars().filter(|c| !c.is_control()) {
                state.execute(actions::InsertChar(c));
            }
        });
    }
    // Execute with insert bounds to preserve Voci's end-gap cursor in every mode.
    fn at_insertion_bounds(&mut self, action: impl Execute) {
        let mode = self.state.mode;
        self.state.mode = EditorMode::Insert;
        self.state.execute(action);
        self.state.mode = mode;
        self.align_cursor();
    }
    pub(super) fn left(&mut self) {
        let count = self.text()[..self.cursor()]
            .graphemes(true)
            .next_back()
            .map_or(0, |g| g.chars().count());
        self.at_insertion_bounds(actions::MoveBackward(count));
    }
    pub(super) fn right(&mut self) {
        let count = self.text()[self.cursor()..]
            .graphemes(true)
            .next()
            .map_or(0, |g| g.chars().count());
        self.at_insertion_bounds(actions::MoveForward(count));
    }
    pub(super) fn backspace(&mut self) {
        let count = self.text()[..self.cursor()]
            .graphemes(true)
            .next_back()
            .map_or(0, |g| g.chars().count());
        self.change(|state| {
            state.execute(actions::DeleteChar(count));
        });
    }
    pub(super) fn delete(&mut self) {
        let count = self.text()[self.cursor()..]
            .graphemes(true)
            .next()
            .map_or(0, |g| g.chars().count());
        self.change(|state| {
            let mode = state.mode;
            state.mode = EditorMode::Insert;
            state.execute(actions::DeleteCharForward(count));
            state.mode = mode;
        });
    }
    // Run edtui's word motions on one scalar per grapheme. This preserves its
    // word/punctuation rules without treating combining marks as punctuation.
    fn word_motion(&mut self, end: bool) {
        let text = self.text();
        let graphemes: Vec<_> = text.grapheme_indices(true).collect();
        let classes: String = graphemes
            .iter()
            .map(|(_, g)| {
                if g.chars().all(char::is_whitespace) {
                    ' '
                } else if g.chars().any(|c| c.is_alphanumeric() || c == '_') {
                    'a'
                } else {
                    '.'
                }
            })
            .collect();
        let cursor = graphemes.partition_point(|(i, _)| *i < self.cursor());
        let mut motion = EditorState::new(Lines::from(classes.as_str()));
        motion.mode = EditorMode::Insert;
        motion.cursor.col = cursor;
        if end {
            if classes[cursor..].trim().is_empty() {
                self.set_cursor(text.len());
                return;
            }
            let insertion = self.get_mode() == InputMode::Insert;
            let at_end = classes.as_bytes().get(cursor) != Some(&b' ')
                && classes.as_bytes().get(cursor) != classes.as_bytes().get(cursor + 1);
            if !insertion
                && at_end
                && cursor + 1 < classes.len()
                && classes[cursor + 1..].trim().is_empty()
            {
                self.set_cursor(text.len());
                return;
            }
            if !insertion || !at_end {
                motion.execute(actions::MoveWordForwardToEndOfWord(1));
            }
            if insertion {
                motion.cursor.col += 1;
            }
        } else {
            motion.execute(actions::MoveWordBackward(1));
        }
        self.set_cursor(
            graphemes
                .get(motion.cursor.col)
                .map_or(text.len(), |(i, _)| *i),
        );
    }
    pub(super) fn word_begin(&mut self) {
        self.word_motion(false);
    }
    pub(super) fn word_end(&mut self) {
        self.word_motion(true);
    }
    pub(super) fn edit(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Left if key.modifiers == KeyModifiers::CONTROL => self.word_begin(),
            KeyCode::Right if key.modifiers == KeyModifiers::CONTROL => self.word_end(),
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                    && !c.is_control() =>
            {
                self.insert(&c.to_string())
            }
            KeyCode::Left => self.left(),
            KeyCode::Right => self.right(),
            KeyCode::Home => self.set_cursor(0),
            KeyCode::End => self.set_cursor(self.text().len()),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete(),
            _ => {}
        }
    }
    // edtui 0.11's viewport counts scalar widths (e.g. each half of a ZWJ emoji).
    // Keep this small grapheme viewport until upstream supports grapheme scrolling.
    pub(super) fn visible(&self, width: usize) -> (Line<'static>, u16) {
        let text = self.text();
        let cursor = self.cursor();
        if width == 0 {
            return (Line::default(), 0);
        }
        let mut start = 0;
        while text[start..cursor].width() >= width && start < cursor {
            start += text[start..].graphemes(true).next().unwrap().len();
        }
        let column = text[start..cursor].width() as u16;
        let mut visible = Vec::new();
        let mut used = 0;
        let selection = self.selection();
        for (index, grapheme) in text[start..].grapheme_indices(true) {
            let next = grapheme.width();
            if used + next > width {
                break;
            }
            let style = if selection
                .as_ref()
                .is_some_and(|range| range.contains(&(start + index)))
            {
                Style::default().bg(Color::Cyan).fg(Color::Black)
            } else {
                Style::default()
            };
            visible.push(Span::styled(grapheme.to_owned(), style));
            used += next;
        }
        (Line::from(visible), column)
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_groups_insert_sessions_and_discards_abandoned_redo() {
        let mut input = Input::default();
        input.insert("e\u{301}猫");
        input.backspace();
        input.insert("👩‍💻");
        input.mode(InputMode::Normal);
        input.action(Action::Undo);
        assert_eq!(input.text(), "");
        assert_eq!(input.cursor(), 0);
        input.action(Action::Redo);
        assert_eq!(input.text(), "e\u{301}👩‍💻");
        assert_eq!(input.cursor(), input.text().len());
        input.paste("old").unwrap();
        input.action(Action::Undo);
        // Motion, mode changes, failed paste and a no-op delete retain redo.
        input.mode(InputMode::Insert);
        input.mode(InputMode::Normal);
        input.delete();
        assert!(input.paste("bad\npaste").is_err());
        input.action(Action::Redo);
        assert_eq!(input.text(), "e\u{301}👩‍💻old");
        input.action(Action::Undo);
        input.paste("new").unwrap();
        input.action(Action::Redo);
        assert_eq!(input.text(), "e\u{301}👩‍💻new");
        input.action(Action::Undo);
        input.action(Action::Undo);
        assert_eq!(input.text(), "");
        input.action(Action::Redo);
        input.action(Action::Redo);
        input.action(Action::Redo); // Never replays the abandoned old branch.
        assert_eq!(input.text(), "e\u{301}👩‍💻new");
    }

    #[test]
    fn undo_restores_cuts_and_visual_replacements_atomically() {
        let mut input = Input::with_text("a e\u{301}👩‍💻z");
        input.mode(InputMode::Normal);
        input.set_cursor(2);
        input.mode(InputMode::Visual);
        input.right();
        input.paste("猫").unwrap();
        assert_eq!(input.text(), "a 猫z");
        input.action(Action::Undo);
        assert_eq!(input.text(), "a e\u{301}👩‍💻z");
        assert!(input.selection().is_none());
        input.action(Action::Redo);
        assert_eq!(input.text(), "a 猫z");
        input.set_cursor(2);
        input.cut();
        assert_eq!(input.text(), "a z");
        input.action(Action::Undo);
        assert_eq!(input.text(), "a 猫z");
        assert_eq!(input.cursor(), 2);
        let mut input = Input::with_text("🇨a🇭");
        input.mode(InputMode::Normal);
        input.set_cursor("🇨".len());
        input.cut();
        input.action(Action::Undo);
        assert_eq!(input.text(), "🇨a🇭");
        assert_eq!(input.cursor(), "🇨".len());
        input.action(Action::Redo);
        assert_eq!(input.text(), "🇨🇭");
        assert_eq!(input.cursor(), input.text().len());
    }

    #[test]
    fn undo_history_stays_bounded_and_filters_start_with_a_clean_baseline() {
        let mut input = Input::with_text("filter");
        input.mode(InputMode::Normal);
        input.action(Action::Undo);
        assert_eq!(input.text(), "filter");
        for _ in 0..120 {
            input.paste("x").unwrap();
        }
        for _ in 0..120 {
            input.action(Action::Undo);
        }
        assert_eq!(input.text(), format!("filter{}", "x".repeat(20)));
        for _ in 0..120 {
            input.action(Action::Redo);
        }
        assert_eq!(input.text(), format!("filter{}", "x".repeat(120)));
    }

    #[test]
    fn word_motions_handle_punctuation_unicode_and_edges() {
        let mut input = Input::default();
        input.insert("  Grüße_2,  e\u{301}lan 👩‍💻猫  ");
        input.mode(InputMode::Normal);
        input.set_cursor(0);
        for expected in ["2,", ",", "n 👩", "👩", "猫"] {
            input.word_end();
            assert_eq!(input.cursor(), input.text().find(expected).unwrap());
        }
        input.word_end();
        assert_eq!(input.cursor(), input.text().len());
        for expected in ["猫", "👩", "e\u{301}lan", ",", "Grüße"] {
            input.word_begin();
            assert_eq!(input.cursor(), input.text().find(expected).unwrap());
        }
        input.word_begin();
        input.word_begin();
        assert_eq!(input.cursor(), 0);
        for text in ["", "   ", "e\u{301}", "👩‍💻", "a"] {
            let mut input = Input::default();
            input.insert(text);
            for mode in [InputMode::Normal, InputMode::Visual, InputMode::Insert] {
                input.mode(mode);
                for _ in 0..3 {
                    input.word_begin();
                    input.word_end();
                }
                assert!(
                    input
                        .text()
                        .grapheme_indices(true)
                        .any(|(i, _)| i == input.cursor())
                        || input.cursor() == text.len()
                );
            }
        }
    }

    #[test]
    fn cuts_that_join_graphemes_leave_a_valid_cursor_boundary() {
        let mut input = Input::default();
        input.insert("🇨a🇭");
        input.mode(InputMode::Normal);
        input.set_cursor("🇨".len());
        input.cut();
        assert_eq!(input.text(), "🇨🇭");
        assert_eq!(input.cursor(), input.text().len());
        input.left();
        input.cut();
        assert!(input.text().is_empty());
        assert_eq!(input.cursor(), 0);
    }
}
