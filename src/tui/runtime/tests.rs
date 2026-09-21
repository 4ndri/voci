use super::*;
use crate::lookup::LookupRequest;
use crate::tui::Focus;
use crate::{history::Cursor, tui::keybindings::Action};

async fn apply_read(app: &mut App, coordinator: &Coordinator, effect: Effect) {
    let Effect::Read(request) = effect else {
        panic!("expected a history read");
    };
    let mut jobs = tokio::task::JoinSet::new();
    read_history(
        &mut jobs,
        coordinator,
        app.history.filter.clone(),
        request,
        app.history.generation,
        HistoryTarget::History,
    );
    let Message::History {
        generation,
        target,
        selection,
        result,
    } = jobs.join_next().await.unwrap().unwrap()
    else {
        panic!("expected a history response");
    };
    app.apply_history(generation, target, selection, result);
}

#[tokio::test]
async fn history_navigation_and_refresh_keep_the_expected_saved_selection() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config.toml");
    std::fs::write(&config, "provider='invalid'").unwrap();
    let coordinator = Coordinator::new(Some(config), None, Some(root.path().join("voci.db")));
    let store = coordinator.history().unwrap();
    for query in ["first", "middle", "last"] {
        store
            .start(
                LookupRequest {
                    query: query.into(),
                    from: None,
                    to: None,
                },
                None,
            )
            .await
            .unwrap();
    }

    let mut app = App::new(None, None, Keybindings::default());
    app.tab = Tab::History;
    app.focus = Focus::History;
    let effect = app.navigate(Action::End);
    apply_read(&mut app, &coordinator, effect).await;
    assert_eq!(app.selected().unwrap().query, "first");

    let effect = app.navigate(Action::Home);
    apply_read(&mut app, &coordinator, effect).await;
    assert_eq!(app.selected().unwrap().query, "last");
    app.navigate(Action::Down);
    assert_eq!(app.selected().unwrap().query, "middle");
    let effect = app.refresh();
    apply_read(&mut app, &coordinator, effect).await;
    assert_eq!(app.selected().unwrap().query, "middle");

    // A stale cursor with no remaining rows falls back to the newest page,
    // retaining the active filter instead of showing an empty history.
    app.history.filter.text = "last".into();
    let effect = Effect::Read(HistoryRead {
        cursor: Some(Cursor {
            timestamp: i64::MIN,
            sequence: 0,
        }),
        oldest: false,
        selection: HistorySelection::Preserve,
    });
    apply_read(&mut app, &coordinator, effect).await;
    assert_eq!(app.history.entries.len(), 1);
    assert_eq!(app.selected().unwrap().query, "last");
}
