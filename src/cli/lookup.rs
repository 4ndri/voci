//! Lookup command adapter: progress, cancellation signals, and output format.

use super::{
    args::Cli,
    output::{exit_code, fail, output, output_json, render_result},
};
use crate::{
    app::{Coordinator, LookupPolicy},
    lookup::LookupRequest,
    presentation::lookup_error_text,
    text::safe_text,
};
use std::process::ExitCode;

pub(super) async fn run(cli: &Cli, coordinator: &Coordinator, request: LookupRequest) -> ExitCode {
    let policy = if cli.fresh {
        LookupPolicy::Fresh
    } else {
        LookupPolicy::PreferSaved
    };
    let (cancel, receiver) = tokio::sync::watch::channel(false);
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let operation = coordinator.lookup(request, policy, receiver, Some(progress_tx));
    tokio::pin!(operation);
    let completion = loop {
        tokio::select! {
            completion = &mut operation => break completion,
            _ = tokio::signal::ctrl_c() => { let _ = cancel.send(true); },
            Some(message) = progress_rx.recv() => eprintln!("{}", safe_text(&message)),
        }
    };
    for warning in completion.warnings {
        eprintln!("{}", safe_text(&warning));
    }
    match completion.result {
        Ok(result) if cli.json => output_json(&result),
        Ok(result) => output(&render_result(&result)),
        Err(error) => fail(cli.json, exit_code(&error), &lookup_error_text(&error)),
    }
}
