use crate::domain::{Language, LookupError};
use clap::ValueEnum;
use directories::ProjectDirs;
use serde::Deserialize;
use std::{
    env, fs,
    path::{Path, PathBuf},
};

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FileConfig {
    provider: ProviderName,
    target_language: Option<String>,
    microsoft: MicrosoftConfig,
    wikdict: WikDictConfig,
    tui: TuiConfig,
    history: HistoryConfig,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct HistoryConfig {
    database: Option<PathBuf>,
}

/// Read only history settings, without validating or initializing a lookup provider.
pub fn history_path(
    config: Option<&Path>,
    database: Option<&Path>,
) -> Result<PathBuf, LookupError> {
    if let Some(path) = database {
        return checked_database_path(path.to_owned());
    }
    #[derive(Default, Deserialize)]
    #[serde(default)]
    struct HistoryOnly {
        history: HistoryConfig,
    }
    let path = config
        .map(Path::to_owned)
        .map(Ok)
        .unwrap_or_else(default_path)?;
    let settings: HistoryOnly = match fs::read_to_string(&path) {
        Ok(text) => toml::from_str(&text).map_err(|_| LookupError::Configuration(
            "Cannot read history settings from config.toml. Expected [history].database as a path; use --database PATH to override.".into()
        ))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && config.is_none() => HistoryOnly::default(),
        Err(_) => return Err(LookupError::Configuration(format!(
            "Cannot read {}. Check the path and file permissions, or use --database PATH.", path.display()
        ))),
    };
    if let Some(database) = settings.history.database {
        let database = checked_database_path(database)?;
        return Ok(if database.is_absolute() {
            database
        } else {
            path.parent()
                .unwrap_or_else(|| Path::new("."))
                .join(database)
        });
    }
    crate::history::default_path().map_err(LookupError::Configuration)
}

fn checked_database_path(path: PathBuf) -> Result<PathBuf, LookupError> {
    if path.as_os_str().is_empty() {
        return Err(LookupError::Configuration(
            "History database path cannot be empty.".into(),
        ));
    }
    Ok(path)
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct TuiConfig {
    keybindings: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum ProviderName {
    #[default]
    Wikdict,
    Microsoft,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct WikDictConfig {
    data_dir: Option<PathBuf>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct MicrosoftConfig {
    region: Option<String>,
}

// Intentionally not Debug: credentials must not appear in diagnostics.
pub struct Config {
    pub provider: ProviderName,
    pub target_language: Language,
    pub key: Option<String>,
    pub region: Option<String>,
    pub wikdict_data_dir: Option<PathBuf>,
    pub keybindings: Option<PathBuf>,
}

pub fn default_path() -> Result<PathBuf, LookupError> {
    #[cfg(windows)]
    if let Some(path) = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    {
        return Ok(path.join("voci/config/config.toml"));
    }

    ProjectDirs::from("", "", "voci")
        .map(|dirs| dirs.config_dir().join("config.toml"))
        .ok_or_else(|| {
            LookupError::Configuration(
                "Cannot find your user configuration directory; use --config PATH.".into(),
            )
        })
}

impl Config {
    pub fn wikdict_dir(&self) -> Result<PathBuf, LookupError> {
        if let Some(path) = &self.wikdict_data_dir {
            return Ok(path.clone());
        }
        ProjectDirs::from("", "", "voci")
            .map(|dirs| dirs.data_local_dir().join("wikdict"))
            .ok_or_else(|| {
                LookupError::Configuration(
                    "Cannot find a dictionary directory; set [wikdict].data_dir in config.toml."
                        .into(),
                )
            })
    }

    pub fn microsoft(&self) -> Result<crate::provider::MicrosoftProvider, LookupError> {
        let key = self.key.as_deref().ok_or_else(|| LookupError::Configuration("Set VOCI_MICROSOFT_KEY to use Microsoft, or select --provider wikdict for lookup without a key.".into()))?;
        if self.region.as_ref().is_some_and(|value| {
            value.is_empty() || !value.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        }) {
            return Err(LookupError::Configuration(
                "Microsoft region must be a nonempty Azure region name such as 'westeurope'."
                    .into(),
            ));
        }
        crate::provider::MicrosoftProvider::new(key, self.region.as_deref())
    }
    pub fn load(explicit: Option<&Path>) -> Result<Self, LookupError> {
        let path = match explicit {
            Some(path) => path.to_owned(),
            None => default_path()?,
        };
        let content = match fs::read_to_string(&path) {
            Ok(content) => Some(content),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && explicit.is_none() => {
                None
            }
            Err(_) => {
                return Err(LookupError::Configuration(format!(
                    "Cannot read {}. Check the path and file permissions.",
                    path.display()
                )));
            }
        };
        Self::from_sources(
            content.as_deref(),
            env::var("VOCI_MICROSOFT_KEY").ok(),
            env::var("VOCI_MICROSOFT_REGION").ok(),
        )
    }

    pub fn from_sources(
        content: Option<&str>,
        key: Option<String>,
        region_override: Option<String>,
    ) -> Result<Self, LookupError> {
        let file: FileConfig = match content {
            // Don't echo the parser's source snippet: the file could contain a misplaced secret.
            Some(content) => toml::from_str(content).map_err(|_| LookupError::Configuration("Invalid config.toml. Expected provider, target_language, optional [wikdict].data_dir, [microsoft].region, [history].database and [tui].keybindings; keys belong only in VOCI_MICROSOFT_KEY.".into()))?,
            None => FileConfig::default(),
        };
        let target_language = file
            .target_language
            .as_deref()
            .unwrap_or("en")
            .parse()
            .map_err(|_| {
                LookupError::Configuration("target_language must be 'de' or 'en'.".into())
            })?;
        let key = key.filter(|key| !key.trim().is_empty());
        let region = region_override.or(file.microsoft.region);
        if let Some(path) = file.history.database {
            checked_database_path(path)?;
        }
        Ok(Self {
            provider: file.provider,
            target_language,
            key,
            region,
            wikdict_data_dir: file.wikdict.data_dir,
            keybindings: file.tui.keybindings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_precedence() {
        let defaults = Config::from_sources(None, Some("key".into()), None).unwrap();
        assert_eq!(defaults.target_language, Language::English);
        assert_eq!(defaults.region, None);
        let config = Config::from_sources(
            Some("target_language = 'de'\n[microsoft]\nregion = 'westus'"),
            Some("key".into()),
            Some("westeurope".into()),
        )
        .unwrap();
        assert_eq!(config.target_language, Language::German);
        assert_eq!(config.region.as_deref(), Some("westeurope"));
    }

    #[test]
    fn invalid_configuration_does_not_echo_secrets() {
        for content in [
            "key = 'secret-value'",
            "[microsoft]\nkey = 'secret-value'",
            "target_language = 'fr'",
            "malformed = '",
        ] {
            let error = Config::from_sources(Some(content), Some("secret-value".into()), None)
                .err()
                .unwrap();
            assert!(!error.to_string().contains("secret-value"));
        }
        let config = Config::from_sources(None, None, None).unwrap();
        assert_eq!(config.provider, ProviderName::Wikdict);
        assert!(config.key.is_none());
        assert!(config.microsoft().is_err());
        assert!(
            Config::from_sources(None, Some("".into()), None)
                .unwrap()
                .microsoft()
                .is_err()
        );
    }
}
