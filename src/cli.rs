//! Command dispatch and terminal frontend startup.

mod args;
mod completion;
mod history;
mod lookup;
mod output;

use args::{Cli, Command};
pub use output::{render_history, render_history_at_width, render_result};

use crate::{
    app::Coordinator,
    config::TuiConfig,
    presentation::lookup_error_text,
    text::safe_text,
    tui::{self, Keybindings},
};
use clap::{CommandFactory, Parser};
use output::{exit_code, fail, output, output_json};
use std::{process::ExitCode, sync::Arc};

pub async fn run() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let code = error.exit_code() as u8;
            if code == 0 {
                let _ = error.print();
            } else {
                eprintln!(
                    "{}",
                    error
                        .to_string()
                        .lines()
                        .map(safe_text)
                        .collect::<Vec<_>>()
                        .join("\n")
                );
            }
            return ExitCode::from(code);
        }
    };
    let fail = |code, message: &str| fail(cli.json, code, message);
    if cli.fresh && cli.word.is_none() {
        return fail(2, "--fresh requires a lookup word.");
    }
    if cli.word.is_none() && cli.command.is_none() {
        let _ = Cli::command().print_help();
        println!();
        return ExitCode::SUCCESS;
    }
    if cli.word.is_some() && cli.command.is_some() {
        return fail(2, "A lookup word cannot be combined with a subcommand.");
    }
    if let Some(Command::Completions { shell }) = &cli.command {
        return output(shell.script());
    }
    if let Some(Command::Complete { words }) = &cli.command {
        let values = crate::cli::completion::complete(words).await;
        return if cli.json {
            output_json(&values)
        } else {
            output(&values.join("\n"))
        };
    }
    if let Some(Command::History(options) | Command::Search { options, .. }) = &cli.command {
        return history::run(&cli, options).await;
    }
    if let Err(error) = cli.validate() {
        return fail(exit_code(&error), &lookup_error_text(&error));
    }
    let coordinator = Coordinator::new(
        cli.config.clone(),
        cli.provider.map(Into::into),
        cli.database.clone(),
    );
    if let Some(request) = cli.request() {
        lookup::run(&cli, &coordinator, request).await
    } else {
        if let Err(e) = tui::check_terminal() {
            return fail(1, &e.to_string());
        }
        let config = match TuiConfig::load(cli.config.as_deref()) {
            Ok(config) => config,
            Err(e) => return fail(1, &e.to_string()),
        };
        let config_path = match cli
            .config
            .clone()
            .map(Ok)
            .unwrap_or_else(crate::config::default_path)
        {
            Ok(path) => path,
            Err(e) => return fail(1, &e.to_string()),
        };
        let (bindings, warnings) =
            match Keybindings::load(&config_path, config.keybindings.as_deref()) {
                Ok(v) => v,
                Err(e) => return fail(1, &e),
            };
        match tui::run(Arc::new(coordinator), cli.from, cli.to, bindings, warnings).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(1, &e.to_string()),
        }
    }
}
