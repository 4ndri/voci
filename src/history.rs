//! Immutable lookup events. Reads join start/outcome events into one encounter.
use crate::domain::{Language, LookupError, LookupRequest, LookupResult};
use caseless::Caseless;
use chrono::{DateTime, Days, Local, TimeZone, Utc};
use rusqlite::{Connection, OpenFlags, functions::FunctionFlags, params};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use unicode_normalization::UnicodeNormalization;

pub type AttemptId = String;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AttemptOutcome {
    Success,
    NotFound,
    Ambiguous,
    Undetermined,
    Failure,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Finished {
    pub outcome: AttemptOutcome,
    pub result: Option<LookupResult>,
    pub error_code: Option<String>,
    pub message: Option<String>,
}
impl Finished {
    pub fn from_result(result: &Result<LookupResult, LookupError>) -> Self {
        match result {
            Ok(result) => Self {
                outcome: AttemptOutcome::Success,
                result: Some(result.clone()),
                error_code: None,
                message: None,
            },
            Err(error) => {
                let (outcome, code) = match error {
                    LookupError::NotFound { .. } => (AttemptOutcome::NotFound, "not_found"),
                    LookupError::Ambiguous(_) => (AttemptOutcome::Ambiguous, "ambiguous"),
                    LookupError::Undetermined(_) => (AttemptOutcome::Undetermined, "undetermined"),
                    LookupError::Cancelled => (AttemptOutcome::Cancelled, "cancelled"),
                    LookupError::Configuration(_) => (AttemptOutcome::Failure, "configuration"),
                    LookupError::Dictionary(_) => (AttemptOutcome::Failure, "dictionary"),
                    LookupError::DictionaryDownload(_) => {
                        (AttemptOutcome::Failure, "dictionary_download")
                    }
                    LookupError::Authentication => (AttemptOutcome::Failure, "authentication"),
                    LookupError::RateLimited => (AttemptOutcome::Failure, "rate_limited"),
                    LookupError::Network => (AttemptOutcome::Failure, "network"),
                    LookupError::Timeout => (AttemptOutcome::Failure, "timeout"),
                    LookupError::InvalidResponse => (AttemptOutcome::Failure, "invalid_response"),
                    LookupError::ProviderRejected => (AttemptOutcome::Failure, "provider_rejected"),
                    LookupError::ProviderUnavailable => {
                        (AttemptOutcome::Failure, "provider_unavailable")
                    }
                    _ => (AttemptOutcome::Failure, "invalid_request"),
                };
                Self {
                    outcome,
                    result: None,
                    error_code: Some(code.into()),
                    message: Some(crate::presentation::safe_text(&error.to_string())),
                }
            }
        }
    }
    pub fn label(&self) -> &'static str {
        match self.outcome {
            AttemptOutcome::Success => "success",
            AttemptOutcome::NotFound => "not found",
            AttemptOutcome::Ambiguous => "ambiguous",
            AttemptOutcome::Undetermined => "undetermined",
            AttemptOutcome::Failure => "failed",
            AttemptOutcome::Cancelled => "cancelled",
        }
    }
}

#[derive(Clone, Debug)]
pub struct HistoryEntry {
    pub id: AttemptId,
    pub sequence: i64,
    pub query: String,
    pub from: Option<Language>,
    pub to: Option<Language>,
    pub provider: Option<String>,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub finished: Option<Finished>,
}
impl HistoryEntry {
    pub fn result(&self) -> Option<&LookupResult> {
        self.finished.as_ref().and_then(|f| f.result.as_ref())
    }
    pub fn status(&self) -> &'static str {
        self.finished
            .as_ref()
            .map_or("unfinished · outcome not recorded", Finished::label)
    }
    pub fn cursor(&self) -> Cursor {
        Cursor {
            timestamp: self.started_at,
            sequence: self.sequence,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Cursor {
    pub timestamp: i64,
    pub sequence: i64,
}
#[derive(Clone, Debug, Default)]
pub struct HistoryFilter {
    pub text: String,
    pub today: bool,
}
#[derive(Clone, Debug)]
pub struct HistoryPage {
    pub entries: Vec<HistoryEntry>,
    pub has_more: bool,
}
#[derive(Clone, Debug)]
pub struct HistoryStore {
    path: PathBuf,
}
#[derive(Debug, thiserror::Error)]
#[error(
    "History unavailable at {path}: {reason}. Check the path, permissions, and database; existing data was not replaced."
)]
pub struct HistoryError {
    path: PathBuf,
    reason: String,
}

pub fn default_path() -> Result<PathBuf, String> {
    #[cfg(windows)]
    if let Some(path) = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    {
        return Ok(path.join("voci/data/history.sqlite3"));
    }

    directories::ProjectDirs::from("", "", "voci")
        .map(|d| d.data_local_dir().join("history.sqlite3"))
        .ok_or_else(|| {
            "Cannot resolve the user application data directory for history.sqlite3.".into()
        })
}
pub fn fold(value: &str) -> String {
    value.nfd().default_case_fold().nfc().collect()
}
pub fn display_time(timestamp: i64) -> String {
    DateTime::from_timestamp_micros(timestamp)
        .map(|t| {
            t.with_timezone(&Local)
                .format("%Y-%m-%d %H:%M:%S %:z")
                .to_string()
        })
        .unwrap_or_else(|| "invalid timestamp".into())
}
fn day_bounds() -> (i64, i64) {
    local_day_bounds(Local::now().date_naive())
}
fn local_day_bounds(date: chrono::NaiveDate) -> (i64, i64) {
    let midnight = |date: chrono::NaiveDate| {
        // Some zones change offset at midnight. Find the first representable instant of that date.
        let mut time = date.and_hms_opt(0, 0, 0).unwrap();
        loop {
            if let Some(value) = Local.from_local_datetime(&time).earliest() {
                break value.timestamp_micros();
            }
            time += chrono::Duration::minutes(1);
        }
    };
    (
        midnight(date),
        midnight(date.checked_add_days(Days::new(1)).unwrap()),
    )
}
impl HistoryStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    fn error(&self, error: impl std::fmt::Display) -> HistoryError {
        HistoryError {
            path: self.path.clone(),
            reason: error.to_string(),
        }
    }
    async fn worker<T: Send + 'static>(
        &self,
        task: impl FnOnce(Self) -> Result<T, HistoryError> + Send + 'static,
    ) -> Result<T, HistoryError> {
        let store = self.clone();
        tokio::task::spawn_blocking(move || task(store))
            .await
            .map_err(|e| self.error(e))?
    }
    fn open(&self, write: bool) -> Result<Option<Connection>, HistoryError> {
        if !write && !self.path.try_exists().map_err(|e| self.error(e))? {
            return Ok(None);
        }
        if write && !self.path.try_exists().map_err(|e| self.error(e))? {
            if let Some(parent) = self.path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| self.error(e))?;
            }
            // Publish a fully initialized database atomically. Concurrent first launches
            // cannot see an empty schema or race a journal-mode transition.
            let temporary = tempfile::NamedTempFile::new_in(
                self.path.parent().unwrap_or_else(|| Path::new(".")),
            )
            .map_err(|e| self.error(e))?;
            let connection = Connection::open(temporary.path()).map_err(|e| self.error(e))?;
            connection.execute_batch("PRAGMA journal_mode=WAL;
                BEGIN IMMEDIATE;
                CREATE TABLE events (
                    event_id INTEGER PRIMARY KEY AUTOINCREMENT,
                    attempt_id TEXT NOT NULL, phase INTEGER NOT NULL CHECK(phase IN (0,1)),
                    format_version INTEGER NOT NULL DEFAULT 1 CHECK(format_version=1),
                    timestamp INTEGER NOT NULL, query TEXT NOT NULL,
                    source_language TEXT, target_language TEXT, provider TEXT, data_json TEXT,
                    UNIQUE(attempt_id, phase));
                CREATE INDEX history_order ON events(phase,timestamp DESC,event_id DESC);
                CREATE TRIGGER immutable_update BEFORE UPDATE ON events BEGIN SELECT RAISE(ABORT,'History events are immutable'); END;
                CREATE TRIGGER immutable_delete BEFORE DELETE ON events BEGIN SELECT RAISE(ABORT,'History events are immutable'); END;
                PRAGMA user_version=1; COMMIT;").map_err(|e|self.error(e))?;
            connection.close().map_err(|(_, e)| self.error(e))?;
            temporary.as_file().sync_all().map_err(|e| self.error(e))?;
            match temporary.persist_noclobber(&self.path) {
                Ok(_) => {}
                Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(self.error(e.error)),
            }
        }
        let flags = if write {
            OpenFlags::SQLITE_OPEN_READ_WRITE
        } else {
            OpenFlags::SQLITE_OPEN_READ_ONLY
        };
        let connection =
            Connection::open_with_flags(&self.path, flags).map_err(|e| self.error(e))?;
        connection
            .busy_timeout(Duration::from_secs(1))
            .map_err(|e| self.error(e))?;
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(|e| self.error(e))?;
        if version != 1 {
            return Err(self.error("Unsupported history schema version"));
        }
        connection
            .create_scalar_function(
                "voci_fold",
                1,
                FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
                |ctx| Ok(fold(&ctx.get::<String>(0)?)),
            )
            .map_err(|e| self.error(e))?;
        Ok(Some(connection))
    }
    pub async fn start(
        &self,
        request: LookupRequest,
        provider: Option<String>,
    ) -> Result<AttemptId, HistoryError> {
        self.worker(move |store| {
            let id = uuid::Uuid::new_v4().to_string();
            let connection = store.open(true)?.unwrap();
            connection.execute("INSERT INTO events(attempt_id,phase,timestamp,query,source_language,target_language,provider) VALUES (?1,0,?2,?3,?4,?5,?6)", params![id,Utc::now().timestamp_micros(),request.query,request.from.map(|l|l.code()),request.to.map(|l|l.code()),provider]).map_err(|e| store.error(e))?;
            Ok(id)
        }).await
    }
    pub async fn finish(
        &self,
        id: AttemptId,
        outcome: Finished,
        provider: Option<String>,
    ) -> Result<(), HistoryError> {
        self.worker(move |store| {
            let connection = store.open(true)?.unwrap();
            let pair = outcome.result.as_ref().map(|r|r.pair);
            let provider = outcome.result.as_ref().map(|r|r.provider.clone()).or(provider);
            let json = serde_json::to_string(&outcome).map_err(|e| store.error(e))?;
            let count = connection.execute("INSERT INTO events(attempt_id,phase,timestamp,query,source_language,target_language,provider,data_json)
                SELECT attempt_id,1,?2,query,?3,?4,?5,?6 FROM events WHERE attempt_id=?1 AND phase=0",params![id,Utc::now().timestamp_micros(),pair.map(|p|p.from.code()),pair.map(|p|p.to.code()),provider,json]).map_err(|e| store.error(e))?;
            if count != 1 { return Err(store.error("Attempt start is missing")); }
            Ok(())
        }).await
    }
    pub async fn page(
        &self,
        filter: HistoryFilter,
        cursor: Option<Cursor>,
        limit: usize,
        oldest: bool,
    ) -> Result<HistoryPage, HistoryError> {
        self.worker(move |store| {
            let Some(connection) = store.open(false)? else { return Ok(HistoryPage { entries: vec![], has_more: false }); };
            let (begin,end) = if filter.today { day_bounds() } else { (i64::MIN,i64::MAX) };
            let order = if oldest { "ASC" } else { "DESC" };
            let comparison = if oldest { ">" } else { "<" };
            let sql = format!("SELECT s.attempt_id,s.event_id,s.query,coalesce(f.source_language,s.source_language),coalesce(f.target_language,s.target_language),coalesce(f.provider,s.provider),s.timestamp,f.timestamp,f.data_json
                FROM events s LEFT JOIN events f ON f.attempt_id=s.attempt_id AND f.phase=1
                WHERE s.phase=0 AND s.timestamp>=?1 AND s.timestamp<?2
                AND (?3='' OR instr(voci_fold(s.query),?3)>0 OR EXISTS(SELECT 1 FROM json_each(f.data_json,'$.result.candidates') c WHERE instr(voci_fold(json_extract(c.value,'$.text')),?3)>0))
                AND (?4 IS NULL OR (s.timestamp,s.event_id){comparison}(?4,?5))
                ORDER BY s.timestamp {order},s.event_id {order} LIMIT ?6");
            let mut stmt = connection.prepare(&sql).map_err(|e| store.error(e))?;
            let mut rows = stmt.query(params![begin,end,fold(&filter.text),cursor.map(|c|c.timestamp),cursor.map(|c|c.sequence),i64::try_from(limit.saturating_add(1)).unwrap_or(i64::MAX)]).map_err(|e| store.error(e))?;
            let mut entries=Vec::new();
            while let Some(row) = rows.next().map_err(|e| store.error(e))? {
                let get = |i| row.get::<_,Option<String>>(i).map_err(|e|store.error(e));
                let language = |i| -> Result<Option<Language>,HistoryError> { get(i)?.map(|s|s.parse().map_err(|e|store.error(e))).transpose() };
                entries.push(HistoryEntry { id: row.get(0).map_err(|e|store.error(e))?, sequence: row.get(1).map_err(|e|store.error(e))?, query: row.get(2).map_err(|e|store.error(e))?, from: language(3)?,to:language(4)?,provider:get(5)?,started_at:row.get(6).map_err(|e|store.error(e))?,finished_at:row.get(7).map_err(|e|store.error(e))?,finished:get(8)?.map(|s|serde_json::from_str(&s).map_err(|e|store.error(e))).transpose()? });
            }
            let has_more=entries.len()>limit;
            entries.truncate(limit);
            if oldest { entries.reverse(); }
            Ok(HistoryPage {entries,has_more})
        }).await
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[test]
    fn local_days_account_for_dst() {
        if std::env::var_os("VOCI_TEST_DST_CHILD").is_some() {
            for (date, hours) in [("2026-03-29", 23), ("2026-10-25", 25), ("2026-09-19", 24)] {
                let (start, end) = super::local_day_bounds(date.parse().unwrap());
                assert_eq!((end - start) / 3_600_000_000, hours);
            }
        } else {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "history::tests::local_days_account_for_dst"])
                .env("TZ", "Europe/Zurich")
                .env("VOCI_TEST_DST_CHILD", "1")
                .status()
                .unwrap();
            assert!(status.success());
        }
    }
}
