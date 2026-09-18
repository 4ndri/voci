use clap::{CommandFactory, Parser};
use std::{
    io::{self, Write},
    process::ExitCode,
    sync::Arc,
};
use voci::{
    app::LookupService,
    cli::Cli,
    config::{Config, ProviderName},
    presentation::{render_result, safe_text},
    provider::Provider,
    tui,
    wikdict::WikDictProvider,
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            // Clap may echo untrusted arguments; sanitize errors before printing.
            let code = error.exit_code() as u8;
            if code == 0 {
                let _ = error.print();
            } else {
                eprintln!("{}", safe_multiline(&error.to_string()));
            }
            return ExitCode::from(code);
        }
    };
    if cli.word.is_none() && cli.command.is_none() {
        let _ = Cli::command().print_help();
        println!();
        return ExitCode::SUCCESS;
    }
    if let Err(error) = cli.validate() {
        return fail(error.exit_code(), &error.to_string());
    }
    if cli.command.is_some()
        && let Err(error) = tui::check_terminal()
    {
        return fail(1, &error.to_string());
    }
    let config = match Config::load(cli.config.as_deref()) {
        Ok(config) => config,
        Err(error) => return fail(error.exit_code(), &error.to_string()),
    };
    let provider = match cli.provider.unwrap_or(config.provider) {
        ProviderName::Wikdict => {
            let directory = match config.wikdict_dir() {
                Ok(path) => path,
                Err(error) => return fail(error.exit_code(), &error.to_string()),
            };
            let provider = WikDictProvider::new(directory);
            tokio::select! {
                result = provider.prepare(|message| eprintln!("{}", safe_text(message))) => {
                    if let Err(error) = result { return fail(error.exit_code(), &error.to_string()); }
                },
                _ = tokio::signal::ctrl_c() => return ExitCode::from(130),
            }
            Provider::WikDict(provider)
        }
        ProviderName::Microsoft => match config.microsoft() {
            Ok(provider) => Provider::Microsoft(provider),
            Err(error) => return fail(error.exit_code(), &error.to_string()),
        },
    };
    let service = Arc::new(LookupService::new(provider, config.target_language));
    if let Some(request) = cli.request() {
        tokio::select! {
            result = service.lookup(request) => match result {
                Ok(result) => match io::stdout().lock().write_all(render_result(&result).as_bytes()) {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(error) if error.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
                    Err(_) => fail(1, "Cannot write lookup results to standard output."),
                },
                Err(error) => fail(error.exit_code(), &error.to_string()),
            },
            _ = tokio::signal::ctrl_c() => ExitCode::from(130),
        }
    } else {
        match tui::run(service, cli.from, cli.to).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => fail(1, &error.to_string()),
        }
    }
}

fn fail(code: u8, message: &str) -> ExitCode {
    eprintln!("{}", safe_text(message));
    ExitCode::from(code)
}

fn safe_multiline(message: &str) -> String {
    message
        .lines()
        .map(safe_text)
        .collect::<Vec<_>>()
        .join("\n")
}
