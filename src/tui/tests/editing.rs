use super::*;
use crossterm::cursor::SetCursorStyle;

#[test]
fn normal_input_enters_insert_and_pastes_after_unicode_cursor() {
    let mut app = App::new(None, None, Keybindings::default());
    app.lookup.input.insert("ae\u{301}猫z");
    app.escape();
    app.event(key('h'));
    app.event(key('h'));
    let mut clipboard = TestClipboard {
        text: "ö".into(),
        ..Default::default()
    };
    let effect = app.event(key('p'));
    assert!(matches!(effect, Effect::Paste(_)));
    app.clipboard_effect(&mut clipboard, effect);
    assert_eq!(app.lookup.input.text(), "ae\u{301}猫öz");
    assert_eq!(app.lookup.input.cursor(), "ae\u{301}猫ö".len());
    assert!(matches!(app.event(enter()), Effect::None));
    assert!(app.lookup.input.mode() == InputMode::Insert);
    app.event(key('p'));
    assert_eq!(app.lookup.input.text(), "ae\u{301}猫öpz");
    assert!(matches!(app.event(enter()), Effect::Submit));
}

#[test]
fn change_selection_copies_before_editing_and_groups_replacement_for_undo() {
    for (profile, binding) in [
        (
            include_str!("../../../assets/keybindings/qwerty.keybinding.toml"),
            'c',
        ),
        (
            include_str!("../../../assets/keybindings/neo-noted.keybinding.toml"),
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
                input.set_mode(InputMode::Normal);
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
                assert!(input.mode() == InputMode::Visual);
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
                assert!(input.mode() == InputMode::Insert);
                assert_eq!(input.mode().cursor_style(), SetCursorStyle::SteadyBar);
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
    app.lookup.input = Input::with_text("word");
    app.lookup.input.set_mode(InputMode::Normal);
    app.lookup.input.set_cursor(0);
    assert!(matches!(app.event(key('c')), Effect::None));
    assert_eq!(app.lookup.input.text(), "word");
    assert!(app.lookup.input.mode() == InputMode::Normal);
    for text in ["", "word"] {
        app.lookup.input = Input::with_text(text); // Cursor at the end gap.
        app.lookup.input.set_mode(InputMode::Visual);
        assert!(matches!(app.event(key('c')), Effect::None));
        assert_eq!(app.lookup.input.text(), text);
        assert!(app.lookup.input.mode() == InputMode::Visual);
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
            include_str!("../../../assets/keybindings/qwerty.keybinding.toml"),
            key('u'),
            ctrl_r(),
        ),
        (
            include_str!("../../../assets/keybindings/neo-noted.keybinding.toml"),
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
                app.history.filter.text = "saved ".into();
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
            assert!(app.focused_input().unwrap().mode() == InputMode::Normal);
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
            app.history.dialog = None;
            app.tab = Tab::History;
            app.focus = Focus::History;
            assert!(matches!(app.event(ctrl_r()), Effect::Read(_)));
        }
    }
}

#[test]
fn append_enters_insert_after_a_whole_grapheme_in_both_inputs() {
    for (profile, binding) in [
        (
            include_str!("../../../assets/keybindings/qwerty.keybinding.toml"),
            'a',
        ),
        (
            include_str!("../../../assets/keybindings/neo-noted.keybinding.toml"),
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
                input.set_mode(InputMode::Normal);
                assert!(matches!(app.event(key(binding)), Effect::None));
                assert!(app.focused_input().unwrap().mode() == InputMode::Insert);
                // Subsequent a presses must type text, not move the cursor.
                app.event(key('!'));
                app.event(key('a'));
                assert_eq!(app.focused_input().unwrap().text(), expected);
                assert!(app.focused_input().unwrap().selection().is_none());
                let effect = app.event(enter());
                if filter {
                    assert!(matches!(effect, Effect::Read(_)));
                    assert_eq!(app.history.filter.text, expected);
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
                "../../../assets/keybindings/neo-noted.keybinding.toml"
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
        assert!(app.focused_input().unwrap().mode() == InputMode::Insert);
        app.event(ctrl('s'));
        assert!(!app.lookup.loading);
        app.event(ctrl('x'));
        let effect = app.event(ctrl('s'));
        if filter {
            assert!(matches!(effect, Effect::Read(_)));
            assert_eq!(app.history.filter.text, "first slast");
            app.event(key('/'));
            app.event(ctrl('x'));
            app.event(ctrl('f'));
            assert_eq!(app.history.dialog.as_ref().unwrap().field, 1);
            app.event(ctrl('x'));
            app.event(ctrl('f'));
            assert_eq!(app.history.dialog.as_ref().unwrap().field, 2);
            app.event(ctrl('x'));
            assert!(matches!(app.event(ctrl('s')), Effect::Read(_)));
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
            matches!(effect, Effect::Read(_))
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
    app.lookup.input.insert("word");
    app.event(ctrl('x'));
    assert!(matches!(app.event(ctrl('w')), Effect::Submit));
    assert!(!app.pane_mode);
    app.event(ctrl('w'));
    assert!(app.pane_mode);
}
