use crate::config::ProviderName;
use crate::domain::{Language, LanguagePair, LookupError, LookupRequest, validate_query};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    version = option_env!("VOCI_BUILD_VERSION").unwrap_or(env!("CARGO_PKG_VERSION")),
    about = "Quick German ↔ English dictionary lookup",
    disable_help_subcommand = true,
    args_conflicts_with_subcommands = true
)]
pub struct Cli {
    /// Word to look up (use -- shell for the literal word "shell")
    #[arg(value_name = "WORD")]
    pub word: Option<String>,
    #[command(subcommand)]
    pub command: Option<Command>,
    /// Source language: de or en (default: automatic dictionary evidence)
    #[arg(long, global = true, value_name = "LANG")]
    pub from: Option<Language>,
    /// Target language: de or en (default: configured preference)
    #[arg(long, global = true, value_name = "LANG")]
    pub to: Option<Language>,
    /// Read a specific TOML configuration file
    #[arg(long, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,
    /// Dictionary source (default: wikdict, or the configured provider)
    #[arg(long, global = true, value_enum)]
    pub provider: Option<ProviderName>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Open the interactive lookup TUI
    Shell,
}

impl Cli {
    pub fn validate(&self) -> Result<(), LookupError> {
        if let (Some(from), Some(to)) = (self.from, self.to)
            && from == to
        {
            return Err(LookupError::UnsupportedPair(LanguagePair { from, to }));
        }
        if let Some(query) = &self.word {
            validate_query(query)?;
        }
        Ok(())
    }

    pub fn request(&self) -> Option<LookupRequest> {
        self.word.as_ref().map(|query| LookupRequest {
            query: query.clone(),
            from: self.from,
            to: self.to,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_points_and_literal_shell() {
        assert!(matches!(
            Cli::try_parse_from(["voci", "shell"]).unwrap().command,
            Some(Command::Shell)
        ));
        let literal = Cli::try_parse_from(["voci", "--", "shell"]).unwrap();
        assert_eq!(literal.word.as_deref(), Some("shell"));
        assert!(literal.command.is_none());
        assert_eq!(
            Cli::try_parse_from(["voci", "Verbindlichkeit"])
                .unwrap()
                .word
                .as_deref(),
            Some("Verbindlichkeit")
        );
        assert_eq!(
            Cli::try_parse_from(["voci", "shell", "--from", "de"])
                .unwrap()
                .from,
            Some(Language::German)
        );
        assert_eq!(
            Cli::try_parse_from(["voci", "--to", "en", "shell"])
                .unwrap()
                .to,
            Some(Language::English)
        );
        assert!(Cli::try_parse_from(["voci", "word", "another"]).is_err());
        assert!(Cli::try_parse_from(["voci", "shell", "word"]).is_err());
        assert!(Cli::try_parse_from(["voci", "--to", "fr", "word"]).is_err());
    }
}
