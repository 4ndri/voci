//! History and search command adapter.

use super::{
    args::{Cli, Command, HistoryArgs},
    output::{fail, output, output_json},
};
use crate::history::{HistoryFilter, HistoryStore};
use std::{
    io::{self, IsTerminal},
    process::ExitCode,
};

pub(super) async fn run(cli: &Cli, options: &HistoryArgs) -> ExitCode {
    let fail = |code, message: &str| fail(cli.json, code, message);
    let store = match crate::config::history_path(cli.config.as_deref(), cli.database.as_deref()) {
        Ok(path) => HistoryStore::new(path),
        Err(e) => return fail(1, &e.to_string()),
    };
    let filter = HistoryFilter {
        today: options.today,
        text: match &cli.command {
            Some(Command::Search { text, .. }) => text.clone(),
            _ => String::new(),
        },
    };
    let limit = if options.all {
        usize::MAX
    } else {
        options.limit
    };
    match store.page(filter, None, limit, false).await {
        Ok(page) => {
            if cli.json {
                return output_json(&page);
            }
            let width = if io::stdout().is_terminal() {
                crossterm::terminal::size().map_or(100, |(width, _)| usize::from(width))
            } else {
                100
            };
            let text = if page.entries.is_empty() {
                "No saved lookups match.\n".to_owned()
            } else {
                page.entries
                    .iter()
                    .map(|entry| crate::cli::output::render_history_at_width(entry, width))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            output(&text)
        }
        Err(e) => fail(1, &e.to_string()),
    }
}
