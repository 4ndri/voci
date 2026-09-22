use super::*;

fn selection_row(app: &mut App, width: u16, height: u16) -> usize {
    let rendered = screen(app, width, height);
    let marker = format!("> {}. value", app.details.selection.selected().unwrap() + 1);
    let position = rendered
        .find(&marker)
        .expect("selected candidate must be visible");
    rendered[..position].chars().count() / width as usize
}

#[test]
fn details_selection_moves_between_directional_margins_and_list_boundaries() {
    for view in 0..3 {
        // Live lookup, recent preview, and History details.
        for (width, height) in [(80, 30), (120, 32)] {
            let mut app = scrolling_app(view, 40);
            let first_row = selection_row(&mut app, width, height);
            let pane_height = app.details.viewport.1 as usize;
            let lower = (pane_height - 1) * 7 / 10;
            let upper = (pane_height - 1) * 3 / 10;
            assert!(lower > upper);
            // Selection initially moves down without scrolling.
            for index in 1..=lower {
                app.navigate(Action::Down);
                assert_eq!(selection_row(&mut app, width, height), first_row + index);
                assert_eq!(app.details.scroll, 0);
            }
            for _ in 0..12 {
                app.navigate(Action::Down);
                assert_eq!(selection_row(&mut app, width, height), first_row + lower);
            }
            // Reversing direction must not jump straight to the upper margin.
            let offset = app.details.scroll;
            for row in (upper..lower).rev() {
                app.navigate(Action::Up);
                assert_eq!(selection_row(&mut app, width, height), first_row + row);
                assert_eq!(app.details.scroll, offset);
            }
            app.navigate(Action::Up);
            assert_eq!(selection_row(&mut app, width, height), first_row + upper);
            assert_eq!(app.details.scroll, offset - 1);
            // At the end, scrolling stops and selection reaches the bottom row.
            for _ in 0..40 {
                app.navigate(Action::Down);
            }
            assert_eq!(app.details.selection.selected(), Some(39));
            assert_eq!(
                selection_row(&mut app, width, height),
                first_row + pane_height - 1
            );
            assert_eq!(app.details.scroll, 40 - pane_height);
            // At the beginning, selection can reach the top row again.
            for _ in 0..40 {
                app.navigate(Action::Up);
            }
            assert_eq!(selection_row(&mut app, width, height), first_row);
            assert_eq!(app.details.selection.selected(), Some(0));
            assert_eq!(app.details.scroll, 0);
        }
    }
}

#[test]
fn details_paging_short_lists_and_resize_keep_selection_visible() {
    for view in 0..3 {
        let mut short = scrolling_app(view, 3);
        let first_row = selection_row(&mut short, 120, 32);
        short.navigate(Action::End);
        assert_eq!(selection_row(&mut short, 120, 32), first_row + 2);
        assert_eq!(short.details.scroll, 0);
        short.navigate(Action::Up);
        assert_eq!(selection_row(&mut short, 120, 32), first_row + 1);
        let mut app = scrolling_app(view, 40);
        selection_row(&mut app, 120, 32);
        app.navigate(Action::PageDown);
        app.navigate(Action::PageDown);
        assert_eq!(app.details.selection.selected(), Some(10));
        selection_row(&mut app, 120, 32);
        // Shrink and expand, retaining the selected candidate.
        for (width, height) in [(40, 18), (120, 40)] {
            selection_row(&mut app, width, height);
            assert_eq!(app.details.selection.selected(), Some(10));
            assert!(app.details.scroll <= 10);
            assert!(10 < app.details.scroll + app.details.viewport.1 as usize);
        }
        app.navigate(Action::PageUp);
        assert_eq!(app.details.selection.selected(), Some(5));
        selection_row(&mut app, 120, 40);
        app.navigate(Action::Home);
        assert_eq!(app.details.scroll, 0);
    }
}

#[test]
fn oversized_candidates_scroll_to_their_last_line_in_live_and_saved_details() {
    let result = LookupResult {
        query: "word".into(),
        headword: "word".into(),
        normalized_headword: "word".into(),
        pair: INITIAL_PAIRS[0],
        candidates: vec![TranslationCandidate {
            text: format!("FIRST {} LAST", "translation ".repeat(100)),
            normalized: "word".into(),
            part_of_speech: None,
            sense: None,
            prefix: String::new(),
            back_translations: vec![],
        }],
        provider: "fixture".into(),
        attribution: None,
        kind: ResultKind::Dictionary,
    };
    for saved in [false, true] {
        for (width, height) in [(120, 32), (80, 24), (40, 18)] {
            let mut app = App::new(None, None, Keybindings::default());
            app.focus = Focus::Details;
            if saved {
                app.tab = Tab::History;
                let mut entry = saved_entry(1);
                entry.finished = Some(crate::app::history_outcome(&Ok(result.clone())));
                app.history.entries.push(entry);
                app.history.state.select(Some(0));
            } else {
                app.lookup.live = Some(result.clone());
            }
            assert!(screen(&mut app, width, height).contains("FIRST"));
            app.navigate(Action::Up); // Start boundary must not jump to the bottom.
            assert_eq!(app.details.line, 0);
            for _ in 0..200 {
                app.navigate(Action::Down);
            }
            assert!(screen(&mut app, width, height).contains("LAST"));
            assert_eq!(app.details.selection.selected(), Some(0));
            // Copy still addresses the entire candidate, not the visible line.
            app.event(key('y'));
            assert!(
                matches!(app.event(key('y')), Effect::Copy(text) if text == result.candidates[0].text)
            );
            app.navigate(Action::Home);
            assert!(screen(&mut app, width, height).contains("FIRST"));
            app.navigate(Action::End);
            assert!(screen(&mut app, width, height).contains("LAST"));
            // Resizing preserves selection and every line remains reachable.
            screen(&mut app, 40, 18);
            app.navigate(Action::End);
            assert!(screen(&mut app, 40, 18).contains("LAST"));
            app.navigate(Action::PageUp);
            assert!(app.details.line < app.details.max_candidate_line(app.result(), 0));
        }
    }
}

#[test]
fn details_navigation_moves_between_candidates_after_scrolling() {
    let mut app = App::new(None, None, Keybindings::default());
    app.focus = Focus::Details;
    app.lookup.live = Some(LookupResult {
        query: "word".into(),
        headword: "word".into(),
        normalized_headword: "word".into(),
        pair: INITIAL_PAIRS[0],
        candidates: (0..2)
            .map(|index| TranslationCandidate {
                text: format!("{} END{index}", "translation ".repeat(100)),
                normalized: "word".into(),
                part_of_speech: None,
                sense: None,
                prefix: String::new(),
                back_translations: vec![],
            })
            .collect(),
        provider: "fixture".into(),
        attribution: None,
        kind: ResultKind::Dictionary,
    });
    screen(&mut app, 80, 24);
    for _ in 0..app.details.max_candidate_line(app.result(), 0) {
        app.navigate(Action::Down);
    }
    assert_eq!(app.details.selection.selected(), Some(0));
    assert!(screen(&mut app, 80, 24).contains("END0"));
    app.navigate(Action::Down);
    assert_eq!(app.details.selection.selected(), Some(1));
    assert_eq!(app.details.line, 0);
    app.navigate(Action::Up);
    assert_eq!(app.details.selection.selected(), Some(0));
    assert!(screen(&mut app, 80, 24).contains("END0"));
    app.navigate(Action::End);
    assert_eq!(app.details.selection.selected(), Some(1));
    assert!(screen(&mut app, 80, 24).contains("END1"));
}
