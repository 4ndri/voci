//! Shell adapters use this read-only endpoint; completing never prepares a provider.

use crate::lookup::LookupRequest;
use crate::{history::HistoryStore, text::safe_text};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum Shell {
    Bash,
    #[value(alias = "nu")]
    Nushell,
}

impl Shell {
    pub fn script(self) -> &'static str {
        match self {
            Self::Bash => include_str!("../../assets/completions/voci.bash"),
            Self::Nushell => include_str!("../../assets/completions/voci.nu"),
        }
    }
}
const FLAGS: &[&str] = &[
    "--from",
    "--to",
    "--config",
    "--database",
    "--provider",
    "--json",
    "--fresh",
    "--help",
    "--version",
];
const COMMANDS: &[&str] = &["shell", "history", "search", "completions"];

fn matching(values: &[&str], prefix: &str) -> Vec<String> {
    values
        .iter()
        .filter(|value| value.starts_with(prefix))
        .map(|value| (*value).into())
        .collect()
}

fn paths(prefix: &str) -> Vec<String> {
    let split = prefix.rfind(std::path::is_separator).map_or(0, |i| i + 1);
    let (base, name) = prefix.split_at(split);
    let directory = Path::new(if base.is_empty() { "." } else { base });
    let Ok(entries) = std::fs::read_dir(directory) else {
        return vec![];
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let filename = entry.file_name().into_string().ok()?;
            if !filename.starts_with(name) {
                return None;
            }
            let mut value = format!("{base}{filename}");
            if entry.file_type().ok()?.is_dir() {
                value.push(std::path::MAIN_SEPARATOR);
            }
            (safe_text(&value) == value).then_some(value)
        })
        .collect()
}

pub async fn complete(words: &[String]) -> Vec<String> {
    let Some((prefix, previous)) = words.split_last() else {
        return vec![];
    };
    let mut request = LookupRequest {
        query: prefix.clone(),
        from: None,
        to: None,
    };
    let mut config = None;
    let mut database = None;
    let mut expects = None;
    let mut literal = false;
    let mut positional = None;
    for word in previous {
        if literal {
            positional = Some(word.as_str());
            continue;
        }
        let (flag, inline) = word
            .split_once('=')
            .map_or((word.as_str(), None), |(a, b)| (a, Some(b)));
        let (option, value) = if let Some(option) = expects.take() {
            (option, Some(word.as_str()))
        } else {
            (flag, inline)
        };
        match option {
            "--from" | "--to" | "--config" | "--database" | "--provider" | "--limit" => {
                let Some(value) = value else {
                    expects = Some(option);
                    continue;
                };
                match option {
                    "--from" => match value.parse() {
                        Ok(language) => request.from = Some(language),
                        Err(_) => return vec![],
                    },
                    "--to" => match value.parse() {
                        Ok(language) => request.to = Some(language),
                        Err(_) => return vec![],
                    },
                    "--config" => config = Some(PathBuf::from(value)),
                    "--database" => database = Some(PathBuf::from(value)),
                    _ => {}
                }
            }
            "--" => literal = true,
            "--fresh" | "--json" | "--today" | "--all" => {}
            _ if word.starts_with('-') => return vec![],
            _ => positional = Some(word.as_str()),
        }
    }
    if let Some(option) = expects {
        return match option {
            "--from" | "--to" => matching(&["de", "en"], prefix),
            "--provider" => matching(&["wikdict", "microsoft"], prefix),
            "--config" | "--database" => paths(prefix),
            _ => vec![],
        };
    }
    if !literal && prefix.starts_with('-') {
        return matching(FLAGS, prefix);
    }
    if !literal && positional == Some("completions") {
        return matching(&["bash", "nushell"], prefix);
    }
    if positional.is_some() && (literal || positional != Some("search")) {
        return vec![];
    }
    let mut values = if literal || positional.is_some() {
        vec![]
    } else {
        matching(COMMANDS, prefix)
    };
    if let Ok(path) = crate::config::history_path(config.as_deref(), database.as_deref())
        && let Ok(queries) = HistoryStore::new(path).complete_queries(request).await
    {
        for query in queries {
            if safe_text(&query) == query && !values.contains(&query) {
                values.push(query);
            }
        }
    }
    values
}
