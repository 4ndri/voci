//! Own the terminal, background jobs, cancellation, and external I/O.

use crate::lookup::LookupRequest;

#[cfg(test)]
mod tests;

use super::{App, Effect, HistoryRead, HistorySelection, HistoryTarget, Tab};
use crate::{
    app::{Completion, Coordinator},
    domain::Language,
    history::{HistoryFilter, HistoryPage},
    text::safe_text,
    tui::clipboard::DesktopClipboard,
    tui::keybindings::Keybindings,
};
use crossterm::{
    cursor::SetCursorStyle,
    event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        EventStream,
    },
    execute,
};
use futures_util::StreamExt;
use std::{
    io::{self, IsTerminal},
    sync::Arc,
    time::Duration,
};
use tokio::sync::{mpsc, watch};

pub fn check_terminal() -> io::Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(io::Error::other(
            "voci shell requires an interactive terminal on stdin and stdout. Use voci <word> for redirected output.",
        ));
    }
    Ok(())
}

struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(
            io::stdout(),
            DisableBracketedPaste,
            DisableMouseCapture,
            SetCursorStyle::DefaultUserShape
        );
        ratatui::restore();
    }
}

pub(super) enum Message {
    Lookup(u64, Completion),
    History {
        generation: u64,
        target: HistoryTarget,
        selection: HistorySelection,
        result: Result<HistoryPage, String>,
    },
    Suggestions(u64, Result<HistoryPage, String>),
}

pub(super) fn read_suggestions(
    jobs: &mut tokio::task::JoinSet<Message>,
    coordinator: &Coordinator,
    request: LookupRequest,
    generation: u64,
) -> tokio::task::AbortHandle {
    let store = coordinator.history().cloned().map_err(str::to_owned);
    jobs.spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        let result = match store {
            Ok(store) => store
                .suggestions(request, 5)
                .await
                .map_err(|e| e.to_string()),
            Err(error) => Err(error),
        };
        Message::Suggestions(generation, result)
    })
}

fn read_history(
    jobs: &mut tokio::task::JoinSet<Message>,
    coordinator: &Coordinator,
    filter: HistoryFilter,
    request: HistoryRead,
    generation: u64,
    target: HistoryTarget,
) {
    let store = coordinator.history().cloned().map_err(str::to_owned);
    jobs.spawn(async move {
        let result = match store {
            Ok(store) => {
                let limit = match target {
                    HistoryTarget::Recent => 5,
                    HistoryTarget::History => 50,
                };
                let mut page = store
                    .page(filter.clone(), request.cursor, limit, request.oldest)
                    .await;
                if request.selection == HistorySelection::Preserve
                    && request.cursor.is_some()
                    && page.as_ref().is_ok_and(|page| page.entries.is_empty())
                {
                    page = store.page(filter, None, 50, false).await;
                }
                page.map_err(|e| e.to_string())
            }
            Err(e) => Err(e),
        };
        Message::History {
            generation,
            target,
            selection: request.selection,
            result,
        }
    });
}

pub async fn run(
    coordinator: Arc<Coordinator>,
    from: Option<Language>,
    to: Option<Language>,
    bindings: Keybindings,
    warnings: Vec<String>,
) -> io::Result<()> {
    check_terminal()?;
    let _guard = TerminalGuard;
    let mut terminal = ratatui::try_init()?;
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(
            io::stdout(),
            DisableBracketedPaste,
            DisableMouseCapture,
            SetCursorStyle::DefaultUserShape
        );
        previous_hook(info);
    }));
    execute!(io::stdout(), EnableBracketedPaste, EnableMouseCapture)?;
    let mut app = App::new(from, to, bindings);
    app.notice = warnings.join(" · ");
    let mut clipboard = DesktopClipboard::default();
    let mut events = EventStream::new();
    let mut jobs = tokio::task::JoinSet::new();
    let mut cancellations = std::collections::HashMap::<u64, watch::Sender<bool>>::new();
    let (progress_tx, mut progress_rx) = mpsc::unbounded_channel::<String>();
    read_history(
        &mut jobs,
        &coordinator,
        HistoryFilter::default(),
        HistoryRead::default(),
        app.lookup.recent_generation,
        HistoryTarget::Recent,
    );
    let mut cursor_style = None;
    let mut suggestion_job: Option<tokio::task::AbortHandle> = None;
    let outcome = loop {
        if app.update_suggestion_query() {
            if let Some(job) = suggestion_job.take() {
                job.abort();
            }
            if let Some((query, from, to)) = &app.lookup.suggestion_query {
                suggestion_job = Some(read_suggestions(
                    &mut jobs,
                    &coordinator,
                    LookupRequest {
                        query: query.clone(),
                        from: *from,
                        to: *to,
                    },
                    app.lookup.suggestion_generation,
                ));
            }
        }
        let desired_style = app
            .focused_input()
            .map_or(SetCursorStyle::SteadyBlock, |input| {
                input.mode().cursor_style()
            });
        if cursor_style != Some(desired_style) {
            if let Err(error) = execute!(terminal.backend_mut(), desired_style) {
                break Err(error);
            }
            cursor_style = Some(desired_style);
        }
        if let Err(error) = terminal.draw(|frame| app.draw(frame)) {
            break Err(error);
        }
        let effect = tokio::select! {
            event = events.next() => match event {
                Some(Ok(event)) => app.event(event),
                Some(Err(error)) => break Err(error),
                None => Effect::Quit,
            },
            _ = tokio::signal::ctrl_c() => Effect::Quit,
            Some(message) = progress_rx.recv() => {
                app.notice = safe_text(&message);
                Effect::None
            },
            Some(result) = jobs.join_next(), if !jobs.is_empty() => {
                match result {
                    Ok(Message::Lookup(id, completion)) => {
                        cancellations.remove(&id);
                        if id == app.lookup.generation {
                            app.complete(id, completion);
                        } else if !completion.warnings.is_empty() {
                            app.notice = completion.warnings.join(" · ");
                        }
                        app.lookup.recent_generation += 1;
                        read_history(
                            &mut jobs, &coordinator, HistoryFilter::default(),
                            HistoryRead::default(), app.lookup.recent_generation, HistoryTarget::Recent,
                        );
                        if app.tab == Tab::History { app.refresh() } else { Effect::None }
                    },
                    Ok(Message::History { generation, target, selection, result }) => {
                        app.apply_history(generation, target, selection, result);
                        Effect::None
                    },
                    Ok(Message::Suggestions(generation, result)) => {
                        app.apply_suggestions(generation, result);
                        Effect::None
                    },
                    Err(error) if error.is_cancelled() => Effect::None,
                    Err(error) => {
                        app.notice = format!("Background task failed: {error}");
                        Effect::None
                    }
                }
            }
        };
        match effect {
            Effect::Submit => {
                for cancel in cancellations.values() {
                    let _ = cancel.send(true);
                }
                let (id, request) = app.begin();
                let (cancel, receiver) = watch::channel(false);
                cancellations.insert(id, cancel);
                let coordinator = Arc::clone(&coordinator);
                let progress = progress_tx.clone();
                jobs.spawn(async move {
                    Message::Lookup(
                        id,
                        coordinator
                            .record_lookup(request, receiver, Some(progress))
                            .await,
                    )
                });
            }
            Effect::Cancel => {
                for cancel in cancellations.values() {
                    let _ = cancel.send(true);
                }
            }
            Effect::Quit => break Ok(()),
            Effect::Read(request) => {
                if request.cursor.is_none() {
                    app.history.entries.clear();
                    app.history.state.select(None);
                }
                app.history.generation += 1;
                read_history(
                    &mut jobs,
                    &coordinator,
                    app.history.filter.clone(),
                    request,
                    app.history.generation,
                    HistoryTarget::History,
                );
            }
            effect @ (Effect::Copy(_) | Effect::CopySelection(..) | Effect::Paste(_)) => {
                app.clipboard_effect(&mut clipboard, effect);
            }
            Effect::None => {}
        }
    };
    for cancel in cancellations.values() {
        let _ = cancel.send(true);
    }
    let mut shutdown_warnings = Vec::new();
    if tokio::time::timeout(Duration::from_secs(2), async {
        while let Some(result) = jobs.join_next().await {
            if let Ok(Message::Lookup(_, completion)) = result {
                shutdown_warnings.extend(completion.warnings);
            }
        }
    })
    .await
    .is_err()
    {
        shutdown_warnings
            .push("History shutdown timed out; an attempt may remain unfinished.".into());
    }
    jobs.abort_all();
    drop(terminal);
    drop(_guard);
    for warning in shutdown_warnings {
        eprintln!("{}", safe_text(&warning));
    }
    outcome
}
