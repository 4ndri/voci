use super::*;
use crate::lookup::LookupError;

#[test]
fn pane_mode_stays_active_until_explicitly_closed() {
    let mut app = App::new(None, None, Keybindings::default());
    let toggle = || Event::Key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
    app.event(toggle());
    app.event(key('j'));
    app.resolver.reset(); // Clearing a pending key sequence must not exit pane mode.
    app.event(key('l'));
    app.event(key('j'));
    assert!(app.pane_mode);
    assert_eq!(app.focus, Focus::Details);
    assert!(matches!(app.event(enter()), Effect::None));
    assert!(!app.pane_mode);
    assert_eq!(app.focus, Focus::Details);
    app.event(toggle());
    app.event(key('q'));
    assert!(app.pane_mode);
    app.event(toggle());
    assert!(!app.pane_mode);
    app.event(toggle());
    app.escape();
    assert!(!app.pane_mode);
}

#[test]
fn enter_leaves_pane_mode_without_editing_submitting_or_applying() {
    for filter in [false, true] {
        for field in 0..5 {
            let mut app = App::new(None, None, Keybindings::default());
            if filter {
                app.tab = Tab::History;
                app.focus = Focus::History;
                app.event(key('/'));
                app.history.dialog.as_mut().unwrap().field = field;
            }
            app.event(Event::Key(KeyEvent::new(
                KeyCode::Char('w'),
                KeyModifiers::CONTROL,
            )));
            app.event(key('g')); // Enter also clears a pending key sequence.
            assert!(matches!(app.event(enter()), Effect::None));
            assert!(!app.pane_mode);
            assert!(!app.resolver.pending());
            if filter {
                let dialog = app.history.dialog.as_ref().unwrap();
                assert_eq!(dialog.field, field);
                assert!(dialog.text.text().is_empty());
                assert!(app.history.filter.text.is_empty());
            } else {
                assert_eq!(app.focus, Focus::Input);
            }
            if let Some(input) = app.focused_input() {
                assert!(input.mode() == InputMode::Normal);
                assert!(matches!(app.event(enter()), Effect::None));
                assert!(app.focused_input().unwrap().mode() == InputMode::Insert);
            }
        }
    }
}

#[test]
fn typing_shortcuts_and_graphemes_does_not_navigate() {
    let mut app = App::new(None, None, Keybindings::default());
    for c in "qgtjkmnblG/".chars() {
        assert!(matches!(app.event(key(c)), Effect::None));
    }
    assert_eq!(app.lookup.input.text(), "qgtjkmnblG/");
    assert_eq!(app.tab, Tab::Lookup);
    let mut input = Input::default();
    input.insert("äe\u{301}猫");
    input.left();
    input.backspace();
    assert_eq!(input.text(), "ä猫");
    input.delete();
    assert_eq!(input.text(), "ä");
}

#[test]
fn tabs_filters_and_cancellation_preserve_draft() {
    let mut app = App::new(None, None, Keybindings::default());
    app.lookup.input.insert("word");
    let (old, _) = app.begin();
    app.escape();
    app.complete(
        old,
        Completion {
            result: Err(LookupError::Network),
            warnings: vec![],
        },
    );
    assert_eq!(app.lookup.problem.as_deref(), Some("Lookup cancelled."));
    app.escape();
    app.event(key('g'));
    assert!(matches!(app.event(key('t')), Effect::Read(_)));
    assert_eq!(app.tab, Tab::History);
    app.event(key('/'));
    app.event(key('b'));
    app.escape();
    assert!(app.history.filter.text.is_empty());
    assert_eq!(app.lookup.input.text(), "word");
}

#[test]
fn recent_preview_copy_and_filter_apply_do_not_replace_the_draft() {
    let mut app = App::new(
        Some(Language::German),
        Some(Language::English),
        Keybindings::default(),
    );
    app.lookup.input.insert("unfinished word");
    app.lookup.recent.push(HistoryEntry {
        id: "old".into(),
        sequence: 1,
        query: "saved word".into(),
        from: Some(Language::German),
        to: Some(Language::English),
        provider: None,
        started_at: 0,
        finished_at: None,
        finished: None,
    });
    app.lookup.input.set_mode(InputMode::Normal);
    app.focus = Focus::Details;
    app.move_focus(1);
    assert_eq!(app.lookup.preview.as_ref().unwrap().query, "saved word");
    assert_eq!(app.lookup.input.text(), "unfinished word");
    app.event(key('y'));
    assert!(matches!(app.event(key('q')),Effect::Copy(text) if text=="saved word"));
    app.escape();
    assert!(app.lookup.preview.is_none());
    app.tab = Tab::History;
    app.focus = Focus::History;
    app.event(key('/'));
    app.event(key('x'));
    assert!(matches!(
        app.event(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE
        ))),
        Effect::Read(_)
    ));
    assert_eq!(app.history.filter.text, "x");
    assert!(app.history.dialog.is_none());
    assert_eq!(app.lookup.input.text(), "unfinished word");
}

#[test]
fn letter_remaps_never_steal_text_input() {
    let bindings = Keybindings::parse(
        "[actions]\nsubmit=['s']\ncancel=['c']\nnext_focus=['f']\nchange_selection=['C']",
    )
    .unwrap();
    let mut app = App::new(None, None, bindings);
    for c in "scf".chars() {
        assert!(matches!(app.event(key(c)), Effect::None));
    }
    assert_eq!(app.lookup.input.text(), "scf");
}

#[test]
fn pane_directions_follow_the_lookup_layout_without_wrapping() {
    let bindings = Keybindings::parse(include_str!(
        "../../../assets/keybindings/neo-noted.keybinding.toml"
    ))
    .unwrap();
    let mut app = App::new(None, None, bindings);
    app.event(Event::Key(KeyEvent::new(
        KeyCode::Char('w'),
        KeyModifiers::CONTROL,
    )));
    assert!(app.pane_mode);
    let mut direction = |key_code, expected| {
        app.event(Event::Key(KeyEvent::new(key_code, KeyModifiers::NONE)));
        assert_eq!(app.focus, expected);
    };
    direction(KeyCode::Left, Focus::Input);
    direction(KeyCode::Char('n'), Focus::Source);
    direction(KeyCode::Char('r'), Focus::Target);
    direction(KeyCode::Down, Focus::Details);
    direction(KeyCode::Down, Focus::Recent);
    direction(KeyCode::Down, Focus::Recent);
    direction(KeyCode::Char('m'), Focus::Details);
    direction(KeyCode::Up, Focus::Source);
    direction(KeyCode::Up, Focus::Input);
}

#[test]
fn numbered_panes_follow_titles_and_preserve_insert_text() {
    for profile in [
        include_str!("../../../assets/keybindings/qwerty.keybinding.toml"),
        include_str!("../../../assets/keybindings/neo-noted.keybinding.toml"),
    ] {
        let mut app = App::new(None, None, Keybindings::parse(profile).unwrap());
        for digit in "12345".chars() {
            app.event(key(digit));
        }
        assert_eq!(app.lookup.input.text(), "12345");
        assert_eq!(app.focus, Focus::Input);
        app.lookup.input.set_mode(InputMode::Normal);
        for pane_mode in [false, true] {
            app.pane_mode = pane_mode;
            for (digit, focus, title) in [
                ('5', Focus::Recent, "[5] Recent lookups"),
                ('4', Focus::Details, "[4] Lookup"),
                ('3', Focus::Target, "[3] Target"),
                ('2', Focus::Source, "[2] Source"),
                ('1', Focus::Input, "[1] Word"),
            ] {
                assert!(screen(&mut app, 120, 32).contains(title));
                assert!(matches!(app.event(key(digit)), Effect::None));
                assert_eq!(app.focus, focus);
                assert_eq!(app.pane_mode, pane_mode);
            }
        }
        app.pane_mode = false;
        app.event(key('g'));
        app.event(key('t'));
        assert_eq!(app.tab, Tab::History);
        let rendered = screen(&mut app, 120, 32);
        assert!(rendered.contains("[1] Encounters"));
        assert!(rendered.contains("[2] Lookup"));
        app.event(key('2'));
        assert_eq!(app.focus, Focus::Details);
        app.event(key('5'));
        assert_eq!(app.focus, Focus::Details);
        app.event(key('1'));
        assert_eq!(app.focus, Focus::History);
    }
}

#[test]
fn pane_shortcut_remaps_update_titles_and_support_insert_commands() {
    let bindings = Keybindings::parse("[actions]\nfocus_pane_3=['Alt-3']").unwrap();
    let mut app = App::new(None, None, bindings);
    assert!(screen(&mut app, 120, 32).contains("[Alt-3] Target"));
    app.event(Event::Key(KeyEvent::new(
        KeyCode::Char('3'),
        KeyModifiers::ALT,
    )));
    assert_eq!(app.focus, Focus::Target);
    assert!(app.lookup.input.text().is_empty());
    app.event(key('1'));
    app.event(key('3'));
    assert_eq!(app.focus, Focus::Input);
    assert!(Keybindings::parse("[actions]\nfocus_pane_3=['2']").is_err());
}

#[test]
fn clicks_focus_lookup_borders_and_contents_without_editing() {
    let mut app = App::new(None, None, Keybindings::default());
    app.lookup.input.insert("draft");
    app.lookup.recent = vec![saved_entry(1)];
    screen(&mut app, 120, 32);
    app.event(click(3, 2));
    assert!(app.lookup.input.mode() == InputMode::Insert);
    for (x, y, focus) in [
        (60, 4, Focus::Target),
        (1, 5, Focus::Source),
        (0, 7, Focus::Details),
        (2, 23, Focus::Recent),
        (0, 1, Focus::Input),
    ] {
        app.event(ctrl('w'));
        app.event(click(x, y));
        assert_eq!(app.focus, focus);
        assert!(!app.pane_mode);
        assert_eq!(app.lookup.input.text(), "draft");
    }
    assert!(app.lookup.preview.is_some());
    for event in [
        click(0, 0),
        click(2, 31),
        mouse(MouseEventKind::Down(MouseButton::Right), 60, 4),
        mouse(MouseEventKind::Up(MouseButton::Left), 60, 4),
        mouse(MouseEventKind::Drag(MouseButton::Left), 60, 4),
        mouse(MouseEventKind::ScrollDown, 60, 4),
    ] {
        app.event(event);
        assert_eq!(app.focus, Focus::Input);
    }
    app.lookup.input.set_mode(InputMode::Normal);
    app.event(key('d'));
    assert!(app.resolver.pending());
    app.event(click(0, 1));
    assert!(!app.resolver.pending());
    screen(&mut app, 8, 3);
    app.event(click(60, 4));
    assert_eq!(app.focus, Focus::Input);
}

#[test]
fn clicks_follow_history_layout_after_resize() {
    let mut app = App::new(None, None, Keybindings::default());
    app.tab = Tab::History;
    app.focus = Focus::History;
    for (width, height, details_x, details_y) in [(120, 32, 60, 3), (80, 32, 0, 20)] {
        screen(&mut app, width, height);
        app.event(click(details_x, details_y));
        assert_eq!(app.focus, Focus::Details);
        app.event(click(0, 3));
        assert_eq!(app.focus, Focus::History);
    }
    for digit in ['2', '1'] {
        app.event(key(digit));
        let focus = app.focus;
        screen(&mut app, 40, 18);
        app.event(click(20, 10));
        assert_eq!(app.focus, focus);
    }
}

#[test]
fn overlays_own_mouse_and_numbered_focus() {
    let mut app = App::new(None, None, Keybindings::default());
    app.lookup.suggestions = vec![saved_entry(1)];
    screen(&mut app, 120, 32);
    app.event(click(60, 4));
    assert_eq!(app.focus, Focus::Input);
    assert!(app.lookup.input.mode() == InputMode::Insert);

    app.tab = Tab::History;
    app.focus = Focus::History;
    app.event(key('/'));
    app.event(key('2'));
    assert_eq!(app.history.dialog.as_ref().unwrap().text.text(), "2");
    screen(&mut app, 120, 32);
    app.event(click(0, 3));
    assert_eq!(app.history.dialog.as_ref().unwrap().field, 0);
    app.event(click(60, 3));
    assert_eq!(app.focus, Focus::History);
    // Centered dialog: text at y=10, date at y=13, buttons at y=16.
    for (x, y, field) in [
        (27, 13, 1),
        (27, 16, 2),
        (49, 16, 3),
        (71, 16, 4),
        (27, 10, 0),
    ] {
        assert!(matches!(app.event(click(x, y)), Effect::None));
        assert_eq!(app.history.dialog.as_ref().unwrap().field, field);
        assert_eq!(app.history.dialog.as_ref().unwrap().text.text(), "2");
    }
    for pane_mode in [false, true] {
        app.pane_mode = pane_mode;
        for digit in ['5', '4', '3', '2', '1'] {
            assert!(matches!(app.event(key(digit)), Effect::None));
            assert_eq!(
                app.history.dialog.as_ref().unwrap().field,
                (digit as u8 - b'1') as usize
            );
            assert_eq!(app.pane_mode, pane_mode);
        }
    }
}

#[test]
fn adaptive_layouts_and_dialogs_render() {
    let mut app = App::new(None, None, Keybindings::default());
    for (width, height) in [(120, 32), (80, 24), (40, 18), (24, 12), (8, 3), (1, 1)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        for tab in [Tab::Lookup, Tab::History] {
            app.tab = tab;
            for focus in [Focus::History, Focus::Details] {
                app.focus = focus;
                terminal.draw(|f| app.draw(f)).unwrap();
            }
        }
        app.history.dialog = Some(Dialog {
            text: Input::default(),
            today: true,
            field: 0,
        });
        terminal.draw(|f| app.draw(f)).unwrap();
        app.history.dialog = None;
    }
}
