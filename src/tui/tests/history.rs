use super::*;
use crate::lookup::LookupError;

#[test]
fn history_updates_ignore_stale_reads_and_preserve_selection() {
    let mut app = App::new(None, None, Keybindings::default());
    app.history.generation = 2;
    app.apply_history(
        2,
        HistoryTarget::History,
        HistorySelection::First,
        page(&[3, 2, 1]),
    );
    app.history.state.select(Some(1));
    app.history.generation = 3;
    app.apply_history(
        2,
        HistoryTarget::History,
        HistorySelection::First,
        page(&[9]),
    );
    app.apply_history(
        2,
        HistoryTarget::History,
        HistorySelection::First,
        Err("stale error".into()),
    );
    assert_eq!(app.history.entries.len(), 3);
    assert!(app.history.error.is_none());
    app.apply_history(
        3,
        HistoryTarget::History,
        HistorySelection::Preserve,
        page(&[4, 3, 2, 1]),
    );
    assert_eq!(app.history.state.selected(), Some(2));
    app.apply_history(
        3,
        HistoryTarget::History,
        HistorySelection::First,
        page(&[]),
    );
    assert_eq!(app.history.entries.len(), 4); // Paging past the boundary keeps the page.
    app.apply_history(
        3,
        HistoryTarget::History,
        HistorySelection::Last,
        page(&[8, 7]),
    );
    assert_eq!(app.history.state.selected(), Some(1));
    app.apply_history(
        3,
        HistoryTarget::History,
        HistorySelection::Preserve,
        page(&[]),
    );
    assert!(app.history.entries.is_empty());
    assert_eq!(app.history.state.selected(), None);
}

#[test]
fn history_read_errors_recover_without_interference_from_recent_reads() {
    let mut app = App::new(None, None, Keybindings::default());
    app.apply_history(
        0,
        HistoryTarget::History,
        HistorySelection::First,
        page(&[1]),
    );
    app.apply_history(
        0,
        HistoryTarget::History,
        HistorySelection::Preserve,
        Err("read failed".into()),
    );
    assert!(app.history.entries.is_empty());
    app.lookup.recent_generation = 1;
    app.apply_history(
        1,
        HistoryTarget::Recent,
        HistorySelection::First,
        page(&[3, 2]),
    );
    app.lookup.recent_state.select(Some(1));
    app.lookup.preview = Some(app.lookup.recent[1].clone());
    assert_eq!(app.history.error.as_deref(), Some("read failed"));
    app.apply_history(
        0,
        HistoryTarget::Recent,
        HistorySelection::First,
        page(&[9]),
    );
    assert_eq!(app.lookup.recent.len(), 2);
    let mut updated = page(&[4, 3, 2]).unwrap();
    updated.entries[2].finished = Some(crate::app::history_outcome(&Err(LookupError::Cancelled)));
    app.apply_history(
        1,
        HistoryTarget::Recent,
        HistorySelection::First,
        Ok(updated),
    );
    assert_eq!(app.lookup.recent_state.selected(), Some(2));
    assert_eq!(app.lookup.preview.as_ref().unwrap().status(), "cancelled");
    app.history.generation += 1;
    app.apply_history(
        1,
        HistoryTarget::History,
        HistorySelection::Preserve,
        page(&[3, 2, 1]),
    );
    assert!(app.history.error.is_none());
    assert!(app.notice.is_empty());
    assert_eq!(app.history.state.selected(), Some(0));
}
