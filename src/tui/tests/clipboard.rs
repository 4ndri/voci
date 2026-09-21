use super::*;

#[test]
fn delete_line_copies_before_deleting_and_is_one_undo_step_in_both_inputs() {
    for (profile, sequence) in [
        (
            include_str!("../../../assets/keybindings/qwerty.keybinding.toml"),
            "dd",
        ),
        (
            include_str!("../../../assets/keybindings/neo-noted.keybinding.toml"),
            "dd",
        ),
        ("[actions]\ndelete_line=['zz']", "zz"),
    ] {
        for filter in [false, true] {
            let original = "a e\u{301}👩‍💻猫";
            for cursor in [0, 2, original.len()] {
                let mut app = App::new(None, None, Keybindings::parse(profile).unwrap());
                if filter {
                    app.tab = Tab::History;
                    app.focus = Focus::History;
                    app.event(key('/'));
                }
                let input = app.focused_input_mut().unwrap();
                *input = Input::with_text(original);
                input.set_mode(InputMode::Normal);
                input.set_cursor(cursor);
                let mut clipboard = TestClipboard {
                    text: "previous".into(),
                    unavailable: true,
                };
                for unavailable in [true, false] {
                    clipboard.unavailable = unavailable;
                    let mut chars = sequence.chars();
                    assert!(matches!(
                        app.event(key(chars.next().unwrap())),
                        Effect::None
                    ));
                    assert_eq!(app.focused_input().unwrap().text(), original);
                    let effect = app.event(key(chars.next().unwrap()));
                    app.clipboard_effect(&mut clipboard, effect);
                    let input = app.focused_input().unwrap();
                    if unavailable {
                        assert_eq!(input.text(), original);
                        assert_eq!(input.cursor(), cursor);
                        assert_eq!(clipboard.text, "previous");
                        app.event(key('u')); // Failed cut must not add an undo step.
                        assert_eq!(app.focused_input().unwrap().text(), original);
                    } else {
                        assert_eq!(input.text(), "");
                        assert_eq!(input.cursor(), 0);
                        assert!(input.mode() == InputMode::Normal);
                        assert_eq!(clipboard.text, original);
                    }
                }
                // Deleting an empty line preserves the register and undo history.
                for c in sequence.chars() {
                    assert!(matches!(app.event(key(c)), Effect::None));
                }
                assert_eq!(clipboard.text, original);
                app.event(key('u'));
                assert_eq!(app.focused_input().unwrap().text(), original);
                assert_eq!(app.focused_input().unwrap().cursor(), cursor);
                app.event(Event::Key(KeyEvent::new(
                    KeyCode::Char('r'),
                    KeyModifiers::CONTROL,
                )));
                assert_eq!(app.focused_input().unwrap().text(), "");
                let effect = app.event(key('p'));
                app.clipboard_effect(&mut clipboard, effect);
                assert_eq!(app.focused_input().unwrap().text(), original);
            }
        }
    }
}

#[test]
fn delete_line_prefix_can_be_cancelled_and_insert_mode_keeps_literal_keys() {
    let mut app = App::new(None, None, Keybindings::default());
    for c in "ddpP".chars() {
        app.event(key(c));
    }
    assert_eq!(app.lookup.input.text(), "ddpP");
    app.escape();
    app.event(key('d'));
    assert!(app.resolver.pending());
    app.escape();
    assert!(!app.resolver.pending());
    assert!(matches!(app.event(key('d')), Effect::None));
    assert_eq!(app.lookup.input.text(), "ddpP");
    app.event(key('h')); // An unrelated key cancels the operator prefix.
    assert!(!app.resolver.pending());
    assert!(matches!(app.event(key('d')), Effect::None));
    assert_eq!(app.lookup.input.text(), "ddpP");
}

#[test]
fn paste_before_and_after_respect_graphemes_failures_and_undo_in_both_inputs() {
    use unicode_segmentation::UnicodeSegmentation;

    for (profile, after, before) in [
        (
            include_str!("../../../assets/keybindings/qwerty.keybinding.toml"),
            'p',
            'P',
        ),
        (
            include_str!("../../../assets/keybindings/neo-noted.keybinding.toml"),
            'p',
            'P',
        ),
        ("[actions]\npaste=['s']\npaste_before=['Shift-s']", 's', 'S'),
    ] {
        for filter in [false, true] {
            for original in ["", "ae\u{301}👩‍💻猫z"] {
                for cursor in [
                    0,
                    1.min(original.len()),
                    "ae\u{301}".len().min(original.len()),
                    original.len(),
                ] {
                    for (binding, paste_after) in [(after, true), (before, false)] {
                        let mut app = App::new(None, None, Keybindings::parse(profile).unwrap());
                        if filter {
                            app.tab = Tab::History;
                            app.focus = Focus::History;
                            app.event(key('/'));
                        }
                        let input = app.focused_input_mut().unwrap();
                        *input = Input::with_text(original);
                        input.set_mode(InputMode::Normal);
                        input.set_cursor(cursor);
                        let mut clipboard = TestClipboard::default();
                        for (text, unavailable) in
                            [("猫", true), ("two\nlines", false), ("", false)]
                        {
                            clipboard.text = text.into();
                            clipboard.unavailable = unavailable;
                            let effect = app.event(key(binding));
                            app.clipboard_effect(&mut clipboard, effect);
                            assert_eq!(app.focused_input().unwrap().text(), original);
                            assert_eq!(app.focused_input().unwrap().cursor(), cursor);
                        }
                        clipboard.text = "ö".into();
                        let effect = app.event(Event::Key(KeyEvent::new(
                            KeyCode::Char(binding),
                            if paste_after {
                                KeyModifiers::NONE
                            } else {
                                KeyModifiers::SHIFT
                            },
                        )));
                        app.clipboard_effect(&mut clipboard, effect);
                        let mut expected = original.to_owned();
                        let position = cursor
                            + if paste_after {
                                original[cursor..]
                                    .graphemes(true)
                                    .next()
                                    .map_or(0, str::len)
                            } else {
                                0
                            };
                        expected.insert(position, 'ö');
                        assert_eq!(app.focused_input().unwrap().text(), expected);
                        app.event(key('u'));
                        assert_eq!(app.focused_input().unwrap().text(), original);
                        assert_eq!(app.focused_input().unwrap().cursor(), cursor);
                        app.event(Event::Key(KeyEvent::new(
                            KeyCode::Char('r'),
                            KeyModifiers::CONTROL,
                        )));
                        assert_eq!(app.focused_input().unwrap().text(), expected);
                    }
                }
            }
        }
    }
}

#[test]
fn visual_d_and_x_cut_only_after_clipboard_success() {
    for key_char in ['d', 'x'] {
        for visual in [false, true] {
            if key_char == 'd' && !visual {
                continue;
            }
            let mut app = App::new(None, None, Keybindings::default());
            let original = "a e\u{301}👩‍💻z";
            app.lookup.input.insert(original);
            app.escape();
            app.lookup.input.set_cursor(2);
            if visual {
                app.event(key('v'));
                app.event(key('l'));
            }
            let mut clipboard = TestClipboard {
                text: "previous".into(),
                unavailable: true,
            };
            let effect = app.event(key(key_char));
            app.clipboard_effect(&mut clipboard, effect);
            assert_eq!(app.lookup.input.text(), original);
            assert_eq!(clipboard.text, "previous");
            clipboard.unavailable = false;
            let effect = app.event(key(key_char));
            app.clipboard_effect(&mut clipboard, effect);
            assert_eq!(
                clipboard.text,
                if visual {
                    "e\u{301}👩‍💻"
                } else {
                    "e\u{301}"
                }
            );
            assert_eq!(
                app.lookup.input.text(),
                if visual { "a z" } else { "a 👩‍💻z" }
            );
            let effect = app.event(key('P'));
            app.clipboard_effect(&mut clipboard, effect);
            assert_eq!(app.lookup.input.text(), original);
            app.lookup.input.set_cursor(app.lookup.input.text().len());
            assert!(matches!(app.event(key(key_char)), Effect::None));
        }
    }
}

#[test]
fn visual_selection_copies_cuts_and_replaces_whole_graphemes() {
    let mut app = App::new(
        None,
        None,
        Keybindings::parse(include_str!(
            "../../../assets/keybindings/neo-noted.keybinding.toml"
        ))
        .unwrap(),
    );
    app.lookup.input.insert("ae\u{301}猫z");
    app.escape();
    app.event(key('b')); // Home in Neo Noted.
    app.event(key('r'));
    app.event(key('v'));
    app.event(key('r')); // Include the combining grapheme and CJK character.
    let mut clipboard = TestClipboard::default();
    let effect = app.event(key('y'));
    app.clipboard_effect(&mut clipboard, effect);
    assert_eq!(clipboard.text, "e\u{301}猫");
    assert_eq!(app.lookup.input.text(), "ae\u{301}猫z");
    assert!(app.lookup.input.mode() == InputMode::Normal);
    app.event(key('v'));
    app.event(key('t')); // Reverse selection is also inclusive.
    let effect = app.event(key('d'));
    app.clipboard_effect(&mut clipboard, effect);
    assert_eq!(app.lookup.input.text(), "az");
    assert_eq!(app.lookup.input.cursor(), 1);
    app.event(key('v'));
    let effect = app.event(key('p'));
    app.clipboard_effect(&mut clipboard, effect);
    assert_eq!(app.lookup.input.text(), "ae\u{301}猫");
    assert!(app.lookup.input.mode() == InputMode::Normal);
}

#[test]
fn rejected_clipboard_actions_preserve_input_and_selection() {
    let mut app = App::new(None, None, Keybindings::default());
    app.lookup.input.insert("word");
    app.escape();
    app.lookup.input.set_cursor(0);
    app.event(key('v'));
    let mut clipboard = TestClipboard {
        text: "two\nlines".into(),
        ..Default::default()
    };
    let effect = app.event(key('p'));
    app.clipboard_effect(&mut clipboard, effect);
    assert_eq!(app.lookup.input.text(), "word");
    assert!(app.lookup.input.mode() == InputMode::Visual);
    assert!(app.notice.contains("single line"));
    clipboard.unavailable = true;
    for action in ['p', 'd'] {
        let effect = app.event(key(action));
        app.clipboard_effect(&mut clipboard, effect);
        assert_eq!(app.lookup.input.text(), "word");
        assert_eq!(app.lookup.input.selection(), Some(0..1));
        assert!(app.notice.contains("unavailable"));
    }
}

#[test]
fn filter_input_uses_the_same_modes_and_clipboard() {
    let mut app = App::new(None, None, Keybindings::default());
    app.tab = Tab::History;
    app.focus = Focus::History;
    app.event(key('/'));
    app.event(key('a'));
    app.event(key('b'));
    app.escape();
    assert!(app.history.dialog.as_ref().unwrap().text.mode() == InputMode::Normal);
    app.event(key('h'));
    let mut clipboard = TestClipboard {
        text: "猫".into(),
        ..Default::default()
    };
    let effect = app.event(key('p'));
    app.clipboard_effect(&mut clipboard, effect);
    assert_eq!(app.history.dialog.as_ref().unwrap().text.text(), "ab猫");
    assert!(matches!(app.event(enter()), Effect::None));
    assert!(app.history.dialog.as_ref().unwrap().text.mode() == InputMode::Insert);
    assert!(matches!(app.event(enter()), Effect::Read(_)));
    assert_eq!(app.history.filter.text, "ab猫");
}
