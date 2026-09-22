use super::super::runtime::{Message, read_suggestions};
use super::*;
use crate::lookup::LookupRequest;

#[tokio::test]
async fn typing_can_open_saved_results_without_provider_setup_or_a_new_attempt() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config.toml");
    std::fs::write(&config, "provider='invalid'").unwrap();
    let coordinator = Coordinator::new(Some(config), None, Some(root.path().join("voci.db")));
    let store = coordinator.history().unwrap();
    let result = scrolling_app(0, 12).lookup.live.unwrap();
    let id = store
        .start(
            LookupRequest {
                query: "word".into(),
                from: None,
                to: None,
            },
            None,
        )
        .await
        .unwrap();
    store
        .finish(id.clone(), crate::app::history_outcome(&Ok(result)), None)
        .await
        .unwrap();

    let mut app = App::new(None, None, Keybindings::default());
    app.event(Event::Paste("wor".into()));
    assert!(app.update_suggestion_query());
    let mut jobs = tokio::task::JoinSet::new();
    read_suggestions(
        &mut jobs,
        &coordinator,
        LookupRequest {
            query: app.lookup.input.text(),
            from: None,
            to: None,
        },
        app.lookup.suggestion_generation,
    );
    let Message::Suggestions(generation, result) = jobs.join_next().await.unwrap().unwrap() else {
        panic!("expected saved suggestions")
    };
    app.apply_suggestions(generation, result);
    for (width, height) in [(24, 12), (60, 20), (120, 32)] {
        assert!(screen(&mut app, width, height).contains("From history"));
    }
    assert!(matches!(app.event(enter()), Effect::Submit));
    app.event(Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)));
    assert!(matches!(app.event(enter()), Effect::None));
    assert_eq!(app.selected().unwrap().id, id);
    assert_eq!(app.result().unwrap().candidates.len(), 12);
    assert_eq!(app.lookup.input.text(), "word");
    assert_eq!(app.lookup.input.cursor(), "word".len());
    assert_eq!(app.focus, Focus::Details);
    assert!(!app.lookup.loading);
    assert_eq!(app.lookup.generation, 0);
    assert!(app.update_suggestion_query());
    assert!(!screen(&mut app, 120, 32).contains("From history"));
    assert_eq!(
        store
            .page(HistoryFilter::default(), None, 50, false)
            .await
            .unwrap()
            .entries
            .len(),
        1
    );
}

#[test]
fn suggestion_selection_resets_on_edits_and_ignores_outdated_reads() {
    let mut app = App::new(None, None, Keybindings::default());
    app.event(key('w'));
    app.update_suggestion_query();
    let old = app.lookup.suggestion_generation;
    app.apply_suggestions(old, page(&[2, 1]));
    app.input_command(Action::Down);
    assert_eq!(app.lookup.suggestion_state.selected(), Some(0));
    app.input_command(Action::Up);
    assert_eq!(app.lookup.suggestion_state.selected(), None);
    app.input_command(Action::Up);
    assert_eq!(app.lookup.suggestion_state.selected(), Some(1));
    app.event(key('x'));
    assert!(app.update_suggestion_query());
    app.apply_suggestions(old, page(&[2, 1]));
    app.apply_suggestions(old, Err("stale error".into()));
    assert!(app.lookup.suggestions.is_empty());
    assert!(app.notice.is_empty());
    assert_eq!(app.lookup.suggestion_state.selected(), None);
    assert!(matches!(app.event(enter()), Effect::Submit));
    app.apply_suggestions(app.lookup.suggestion_generation, page(&[1]));
    app.event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
    assert!(!app.has_suggestions());
    assert!(app.update_suggestion_query());
    app.event(enter());
    assert!(app.update_suggestion_query());
    app.lookup.source = Some(Language::German);
    assert!(app.update_suggestion_query());
    app.lookup.input = Input::default();
    assert!(app.update_suggestion_query());
    assert!(app.lookup.suggestion_query.is_none());
}

#[test]
fn suggestion_navigation_honors_command_remaps_and_keeps_letters_as_text() {
    let bindings =
        Keybindings::parse("[navigation]\ndown=['Alt-n','j']\nup=['Alt-p','k']").unwrap();
    let mut app = App::new(None, None, bindings);
    app.lookup.input.insert("w");
    app.update_suggestion_query();
    app.apply_suggestions(app.lookup.suggestion_generation, page(&[1]));
    app.event(key('j'));
    assert_eq!(app.lookup.input.text(), "wj");
    assert_eq!(app.lookup.suggestion_state.selected(), None);
    app.update_suggestion_query();
    app.apply_suggestions(app.lookup.suggestion_generation, page(&[1]));
    app.event(Event::Key(KeyEvent::new(
        KeyCode::Char('n'),
        KeyModifiers::ALT,
    )));
    assert_eq!(app.lookup.suggestion_state.selected(), Some(0));
    assert!(matches!(app.event(enter()), Effect::None));
    assert!(app.lookup.preview.is_some());
}
