use clap::{CommandFactory, Parser};
use std::{
    io::{self, IsTerminal, Write},
    process::ExitCode,
    sync::Arc,
};
use voci::{
    cli::{Cli, Command},
    config::TuiConfig,
    coordinator::Coordinator,
    history::{HistoryFilter, HistoryStore},
    keybindings::Keybindings,
    presentation::{render_result, safe_text},
    tui,
};
#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
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
    let fail = |code, message: &str| {
        if cli.json {
            eprintln!(
                "{}",
                serde_json::json!({"error": {"code": code, "message": safe_text(message)}})
            );
            ExitCode::from(code)
        } else {
            fail(code, message)
        }
    };
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
        let values = voci::completion::complete(words).await;
        return if cli.json {
            output_json(&values)
        } else {
            output(&values.join("\n"))
        };
    }
    if let Some(Command::History(options) | Command::Search { options, .. }) = &cli.command {
        let store = match voci::config::history_path(cli.config.as_deref(), cli.database.as_deref())
        {
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
        return match store.page(filter, None, limit, false).await {
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
                        .map(|entry| voci::presentation::render_history_at_width(entry, width))
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                output(&text)
            }
            Err(e) => fail(1, &e.to_string()),
        };
    }
    if let Err(e) = cli.validate() {
        return fail(e.exit_code(), &e.to_string());
    }
    let coordinator = Coordinator::new(cli.config.clone(), cli.provider, cli.database.clone());
    if let Some(request) = cli.request() {
        if !cli.fresh
            && let Ok(store) = &coordinator.history
        {
            match store.saved_result(request.clone()).await {
                Ok(Some(entry)) => {
                    let result = entry
                        .result()
                        .expect("saved_result returns successful results");
                    return if cli.json {
                        output_json(result)
                    } else {
                        output(&render_result(result))
                    };
                }
                Ok(None) => {}
                Err(error) => eprintln!("{}", safe_text(&error.to_string())),
            }
        }
        let (cancel, receiver) = tokio::sync::watch::channel(false);
        let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let operation = coordinator.run(request, receiver, Some(progress_tx));
        tokio::pin!(operation);
        let completion = loop {
            tokio::select! {
                completion=&mut operation=>break completion,
                _=tokio::signal::ctrl_c()=>{let _=cancel.send(true);},
                Some(message)=progress_rx.recv()=>eprintln!("{}",safe_text(&message)),
            }
        };
        for warning in completion.warnings {
            eprintln!("{}", safe_text(&warning));
        }
        match completion.result {
            Ok(result) if cli.json => output_json(&result),
            Ok(result) => output(&render_result(&result)),
            Err(e) => fail(e.exit_code(), &e.to_string()),
        }
    } else {
        if let Err(e) = tui::check_terminal() {
            return fail(1, &e.to_string());
        }
        let config = match TuiConfig::load(cli.config.as_deref()) {
            Ok(config) => config,
            Err(e) => return fail(e.exit_code(), &e.to_string()),
        };
        let config_path = match cli
            .config
            .clone()
            .map(Ok)
            .unwrap_or_else(voci::config::default_path)
        {
            Ok(path) => path,
            Err(e) => return fail(e.exit_code(), &e.to_string()),
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
fn output_json(value: &impl serde::Serialize) -> ExitCode {
    match serde_json::to_string_pretty(value) {
        Ok(json) => output(&format!("{json}\n")),
        Err(_) => fail(1, "Cannot serialize results as JSON."),
    }
}
fn output(text: &str) -> ExitCode {
    match io::stdout().lock().write_all(text.as_bytes()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(_) => fail(1, "Cannot write results to standard output."),
    }
}
fn fail(code: u8, message: &str) -> ExitCode {
    eprintln!("{}", safe_text(message));
    ExitCode::from(code)
}
