#[path = "support/commands.rs"]
mod support;
use predicates::prelude::*;
use support::command;

#[test]
fn help_version_and_bare_command_do_not_require_config() {
    for arguments in [
        vec![],
        vec!["--help"],
        vec!["--version"],
        vec!["shell", "--help"],
        vec!["--config", "missing.toml", "--help"],
    ] {
        command()
            .args(arguments)
            .assert()
            .success()
            .stdout(predicate::str::contains("voci"));
    }
}

#[test]
fn rejects_french_and_invalid_inputs_before_loading_credentials() {
    command()
        .args(["--to", "fr", "Verbindlichkeit"])
        .assert()
        .code(2)
        .stdout("")
        .stderr(predicate::str::contains("Unsupported language"));
    command()
        .args(["--from", "de", "--to", "de", "word"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("Unsupported language pair"));
    command()
        .arg("")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("Enter a word"));
    command()
        .arg("word\x1b[2J")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("\x1b").not());
    command()
        .args(["--to", "\x1b[2J", "word"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("\x1b").not());
}

#[test]
fn explicit_missing_config_and_missing_key_have_actionable_errors() {
    let dir = tempfile::tempdir().unwrap();
    let absent = dir.path().join("missing.toml");
    command()
        .arg("--config")
        .arg(&absent)
        .arg("liability")
        .assert()
        .code(1)
        .stdout("")
        .stderr(predicate::str::contains("Cannot read"));
    let config = dir.path().join("config.toml");
    std::fs::write(&config, "provider = 'microsoft'\ntarget_language = 'en'\n").unwrap();
    command()
        .arg("--config")
        .arg(&config)
        .arg("liability")
        .assert()
        .code(1)
        .stdout("")
        .stderr(predicate::str::contains("VOCI_MICROSOFT_KEY"));
    command()
        .arg("--config")
        .arg(&config)
        .args(["--", "shell"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("VOCI_MICROSOFT_KEY"));
}

#[test]
fn tui_rejects_nonterminal_before_credential_loading() {
    command()
        .arg("shell")
        .assert()
        .code(1)
        .stdout("")
        .stderr(predicate::str::contains("requires an interactive terminal"));
}
