//! Local WikDict SQLite dictionaries. No queries or credentials are sent to WikDict.
use crate::{
    domain::*,
    provider::{DictionaryProvider, ProviderCapabilities},
};
use caseless::Caseless;
use reqwest::Client;
use rusqlite::{Connection, OpenFlags, functions::FunctionFlags};
use std::{
    collections::HashSet,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};
use tempfile::NamedTempFile;
use unicode_normalization::UnicodeNormalization;

pub const RELEASE: &str = "2_2026-06";
pub const DOWNLOAD_BASE: &str = "https://download.wikdict.com/dictionaries/sqlite";
pub const ATTRIBUTION: &str = "WikDict by Karl Bartel · Wiktionary contributors via DBnary · CC BY-SA 4.0 · https://www.wikdict.com/page/download · https://creativecommons.org/licenses/by-sa/4.0/ · Selected and formatted by voci";
const MAX_DOWNLOAD: u64 = 64 * 1024 * 1024;
const SELECT: &str = "SELECT written_rep, sense, trans_list FROM translation
    WHERE voci_fold_v2(written_rep) = ?1
    ORDER BY is_good DESC, score DESC, importance DESC, written_rep, sense_num, sense, trans_list";

pub struct WikDictProvider {
    directory: PathBuf,
    download_base: String,
}

impl WikDictProvider {
    pub fn new(data_dir: PathBuf) -> Self {
        Self::with_download_base(data_dir, DOWNLOAD_BASE.to_owned())
    }

    #[doc(hidden)]
    pub fn with_download_base(data_dir: PathBuf, download_base: String) -> Self {
        Self {
            directory: data_dir.join(RELEASE),
            download_base,
        }
    }

    /// Prepare both directions before starting the first lookup deadline.
    /// The coordinator reuses successful preparation within a shell session.
    pub async fn prepare(&self, progress: impl Fn(&str)) -> Result<(), LookupError> {
        std::fs::create_dir_all(&self.directory)
            .map_err(|_| storage_error(&self.directory, "Cannot create dictionary directory"))?;
        for pair in INITIAL_PAIRS {
            let path = self.path(pair);
            if path.exists() {
                let check_path = path.clone();
                tokio::task::spawn_blocking(move || prepare_dictionary(&check_path))
                    .await
                    .map_err(|_| storage_error(&path, "Dictionary validation failed"))??;
                continue;
            }
            progress(&format!(
                "Downloading WikDict {}–{} ({RELEASE}) for offline lookup…",
                pair.from, pair.to
            ));
            self.download(pair, &path).await?;
        }
        Ok(())
    }

    fn path(&self, pair: LanguagePair) -> PathBuf {
        self.directory
            .join(format!("{}-{}.sqlite3", pair.from, pair.to))
    }

    async fn download(&self, pair: LanguagePair, destination: &Path) -> Result<(), LookupError> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .build()
            .map_err(|_| LookupError::DictionaryDownload("unable to initialize HTTPS".into()))?;
        let url = format!(
            "{}/{RELEASE}/{}-{}.sqlite3",
            self.download_base.trim_end_matches('/'),
            pair.from,
            pair.to
        );
        let mut response = client.get(&url).send().await.map_err(download_error)?;
        if !response.status().is_success() {
            return Err(LookupError::DictionaryDownload(format!(
                "server returned HTTP {}",
                response.status().as_u16()
            )));
        }
        if response
            .content_length()
            .is_some_and(|size| size > MAX_DOWNLOAD)
        {
            return Err(LookupError::DictionaryDownload(
                "dictionary exceeds the 64 MiB size limit".into(),
            ));
        }
        // Temporary files share the destination filesystem; interrupted downloads are never installed.
        let mut temporary = NamedTempFile::new_in(&self.directory)
            .map_err(|_| storage_error(destination, "Cannot create dictionary download"))?;
        let mut received = 0;
        while let Some(chunk) = response.chunk().await.map_err(download_error)? {
            received += chunk.len() as u64;
            if received > MAX_DOWNLOAD {
                return Err(LookupError::DictionaryDownload(
                    "dictionary exceeds the 64 MiB size limit".into(),
                ));
            }
            temporary
                .write_all(&chunk)
                .map_err(|_| storage_error(destination, "Cannot write dictionary download"))?;
        }
        temporary
            .flush()
            .map_err(|_| storage_error(destination, "Cannot flush dictionary download"))?;
        let temporary = tokio::task::spawn_blocking(move || {
            let connection = open_dictionary(temporary.path())?;
            let check: String = connection
                .query_row("PRAGMA quick_check(1)", [], |row| row.get(0))
                .map_err(|_| storage_error(temporary.path(), "Invalid downloaded dictionary"))?;
            if check != "ok" {
                return Err(storage_error(
                    temporary.path(),
                    "Corrupt downloaded dictionary",
                ));
            }
            drop(connection);
            prepare_dictionary(temporary.path())?;
            temporary
                .as_file()
                .sync_all()
                .map_err(|_| storage_error(temporary.path(), "Cannot save dictionary"))?;
            Ok::<_, LookupError>(temporary)
        })
        .await
        .map_err(|_| storage_error(destination, "Dictionary preparation failed"))??;
        match temporary.persist_noclobber(destination) {
            Ok(_) => Ok(()),
            // Another voci process may have completed the same download concurrently.
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                prepare_dictionary(destination)
            }
            Err(_) => Err(storage_error(destination, "Cannot install dictionary")),
        }
    }
}

impl DictionaryProvider for WikDictProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            dictionary_pairs: INITIAL_PAIRS.to_vec(),
            translation_pairs: vec![],
        }
    }

    async fn lookup(&self, query: &str, pair: LanguagePair) -> Result<LookupResult, LookupError> {
        let query = validate_query(query)?;
        if !INITIAL_PAIRS.contains(&pair) {
            return Err(LookupError::UnsupportedPair(pair));
        }
        let path = self.path(pair);
        tokio::task::spawn_blocking(move || lookup_local(&path, query, pair))
            .await
            .map_err(|_| {
                LookupError::Dictionary("Local lookup task failed. Retry the lookup.".into())
            })?
    }
}

fn lookup_local(
    path: &Path,
    query: String,
    pair: LanguagePair,
) -> Result<LookupResult, LookupError> {
    let connection = open_dictionary(path)?;
    let mut statement = connection
        .prepare(SELECT)
        .map_err(|_| storage_error(path, "Incompatible dictionary schema"))?;
    let rows = statement
        .query_map([fold(&query)], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|_| storage_error(path, "Cannot query dictionary"))?;
    let mut headword = query.clone();
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    for row in rows {
        let (source, sense, translations) =
            row.map_err(|_| storage_error(path, "Invalid dictionary entry"))?;
        if candidates.is_empty() {
            headword = source;
        }
        let sense = sense.filter(|value| !value.is_empty());
        for translation in translations
            .split(" | ")
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let normalized = fold(translation);
            if seen.insert((normalized.clone(), sense.clone())) {
                candidates.push(TranslationCandidate {
                    text: translation.into(),
                    normalized,
                    part_of_speech: None,
                    sense: sense.clone(),
                    prefix: String::new(),
                    back_translations: vec![],
                });
            }
        }
    }
    Ok(LookupResult {
        query,
        normalized_headword: fold(&headword),
        headword,
        pair,
        candidates,
        provider: format!("WikDict {RELEASE}"),
        attribution: Some(ATTRIBUTION.into()),
        kind: ResultKind::Dictionary,
    })
}

fn configure(connection: &Connection) -> rusqlite::Result<()> {
    let flags = FunctionFlags::SQLITE_UTF8
        | FunctionFlags::SQLITE_DETERMINISTIC
        | FunctionFlags::SQLITE_INNOCUOUS;
    // Keep the old function's meaning intact while validating/migrating legacy indexes.
    connection.create_scalar_function("voci_fold", 1, flags, |context| {
        Ok(context
            .get::<String>(0)?
            .nfc()
            .collect::<String>()
            .to_lowercase()
            .nfc()
            .collect::<String>())
    })?;
    connection.create_scalar_function("voci_fold_v2", 1, flags, |context| {
        Ok(fold(&context.get::<String>(0)?))
    })?;
    connection.execute_batch("PRAGMA trusted_schema=OFF;")
}

fn open_dictionary(path: &Path) -> Result<Connection, LookupError> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| storage_error(path, "Cannot open dictionary"))?;
    configure(&connection).map_err(|_| storage_error(path, "Cannot initialize dictionary"))?;
    connection
        .prepare(SELECT)
        .map_err(|_| storage_error(path, "Corrupt or incompatible dictionary"))?;
    Ok(connection)
}

fn prepare_dictionary(path: &Path) -> Result<(), LookupError> {
    let connection = open_dictionary(path)?;
    let indexed: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='index' AND name='voci_lookup_v2')",
            [],
            |row| row.get(0),
        )
        .map_err(|_| storage_error(path, "Cannot inspect dictionary index"))?;
    if indexed {
        return Ok(());
    }
    drop(connection);
    let mut connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|_| storage_error(path, "Cannot upgrade dictionary index"))?;
    configure(&connection).map_err(|_| storage_error(path, "Cannot initialize dictionary"))?;
    // A new SQL function prevents stale expression indexes from serving new queries.
    // Serialize concurrent upgrades and commit both index changes atomically, retaining source rows.
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|_| storage_error(path, "Cannot lock dictionary for indexing"))?;
    transaction
        .execute_batch(
            "CREATE INDEX IF NOT EXISTS voci_lookup_v2 ON translation(voci_fold_v2(written_rep));
             DROP INDEX IF EXISTS voci_lookup;",
        )
        .map_err(|_| storage_error(path, "Cannot index dictionary"))?;
    transaction
        .commit()
        .map_err(|_| storage_error(path, "Cannot save dictionary index"))
}

fn fold(value: &str) -> String {
    // Keep the caseless data version pinned in Cargo.toml; changing these mappings
    // requires a new SQL function/index version for dictionaries already on disk.
    value.nfd().default_case_fold().nfc().collect()
}

fn storage_error(path: &Path, reason: &str) -> LookupError {
    LookupError::Dictionary(format!(
        "{reason}: {}. Check permissions; for a corrupt file, remove it and retry to download a fresh copy.",
        path.display()
    ))
}

fn download_error(error: reqwest::Error) -> LookupError {
    LookupError::DictionaryDownload(
        if error.is_timeout() {
            "request timed out"
        } else {
            "connection interrupted or unavailable"
        }
        .into(),
    )
}
