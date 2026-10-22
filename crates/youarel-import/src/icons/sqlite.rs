use super::{CacheError, Candidate, Format, Image, safari_file};
use rusqlite::backup::{Backup, StepResult};
use rusqlite::config::DbConfig;
use rusqlite::limits::Limit;
use rusqlite::{Connection, OpenFlags};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;
use youarel_core::icon_rules::{
    BACKUP_PAGES, MOST_BACKUP_STEPS, MOST_BUSY, MOST_CANDIDATES, quality, snapshot_fits,
};
use youarel_link::Link;

const MOST_IMAGE_BYTES: usize = 512 * 1024;
const MOST_ROW_BYTES: i32 = 1024 * 1024;
const SQL_QUANTUM: i32 = 1000;
const SQL_QUANTA: usize = 5000;
const MOST_SQL_BYTES: i32 = 64 * 1024;
const MOST_COLUMNS: i32 = 64;
const SNAPSHOT_FILE: &str = "icons.sqlite";
const PAGE_COLUMN: usize = 0;
const WIDTH_COLUMN: usize = 1;
const HEIGHT_COLUMN: usize = 2;
const PAYLOAD_COLUMN: usize = 3;
const CHROMIUM: &str = "SELECT m.page_url AS page, b.width AS width, b.height AS height, b.image_data AS payload FROM icon_mapping m JOIN favicons f ON f.id=m.icon_id JOIN favicon_bitmaps b ON b.icon_id=f.id";
const FIREFOX: &str = "SELECT p.page_url AS page, i.width AS width, i.width AS height, i.data AS payload FROM moz_pages_w_icons p JOIN moz_icons_to_pages m ON m.page_id=p.id JOIN moz_icons i ON i.id=m.icon_id";
const SAFARI: &str = "SELECT p.url AS page, i.width AS width, i.height AS height, i.uuid AS payload FROM page_url p JOIN icon_info i ON i.uuid=p.uuid";
const FILTER: &str = "WHERE (page IN (?1,?2,?3) OR page LIKE ?4 ESCAPE '\\' OR page LIKE ?5 ESCAPE '\\' OR page LIKE ?6 ESCAPE '\\' OR page LIKE ?7 ESCAPE '\\') AND width BETWEEN 1 AND 4096 AND height BETWEEN 1 AND 4096 AND length(payload) BETWEEN 1 AND 524288 ORDER BY CASE WHEN max(width,height)<=64 THEN 0 ELSE 1 END, CASE WHEN max(width,height)<=64 THEN max(width,height) ELSE -max(width,height) END DESC, page LIMIT ?8";

#[derive(Debug)]
pub struct Snapshot {
    connection: Connection,
    directory: tempfile::TempDir,
    format: Format,
    cancelled: Arc<AtomicBool>,
    work: Arc<AtomicUsize>,
    bytes: u64,
}

fn configure(
    connection: &Connection,
    cancelled: &Arc<AtomicBool>,
) -> Result<Arc<AtomicUsize>, CacheError> {
    connection.busy_timeout(Duration::ZERO)?;
    connection.set_limit(Limit::SQLITE_LIMIT_LENGTH, MOST_ROW_BYTES)?;
    connection.set_limit(Limit::SQLITE_LIMIT_SQL_LENGTH, MOST_SQL_BYTES)?;
    connection.set_limit(Limit::SQLITE_LIMIT_COLUMN, MOST_COLUMNS)?;
    connection.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
    connection.pragma_update(None, "trusted_schema", false)?;
    let work = Arc::new(AtomicUsize::new(SQL_QUANTA));
    let remaining = Arc::clone(&work);
    let stopped = Arc::clone(cancelled);
    connection.progress_handler(
        SQL_QUANTUM,
        Some(move || {
            stopped.load(Ordering::Relaxed)
                || remaining
                    .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
                    .is_err()
        }),
    )?;
    Ok(work)
}

fn number(connection: &Connection, pragma: &str) -> Result<u64, CacheError> {
    let value: i64 = connection.pragma_query_value(None, pragma, |row| row.get(0))?;
    u64::try_from(value).map_err(|_negative| CacheError::TooLarge)
}

fn size(connection: &Connection) -> Result<u64, CacheError> {
    let pages = number(connection, "page_count")?;
    let page_size = number(connection, "page_size")?;
    if snapshot_fits(pages, page_size) {
        pages.checked_mul(page_size).ok_or(CacheError::TooLarge)
    } else {
        Err(CacheError::TooLarge)
    }
}

fn backup(
    source: &Connection,
    destination: &mut Connection,
    cancelled: &AtomicBool,
) -> Result<(), CacheError> {
    let backup = Backup::new(source, destination)?;
    let mut busy: usize = 0;
    for _ in 0..MOST_BACKUP_STEPS {
        if cancelled.load(Ordering::Relaxed) {
            return Err(CacheError::Stopped);
        }
        match backup.step(BACKUP_PAGES)? {
            StepResult::Done => return Ok(()),
            StepResult::More => {
                let pages = u64::try_from(backup.progress().pagecount)
                    .map_err(|_negative| CacheError::TooLarge)?;
                let page_size = number(source, "page_size")?;
                if !snapshot_fits(pages, page_size) {
                    return Err(CacheError::TooLarge);
                }
            }
            StepResult::Busy | StepResult::Locked => {
                busy = busy.saturating_add(1);
                if busy >= MOST_BUSY {
                    return Err(CacheError::Busy);
                }
            }
            _ => return Err(CacheError::Unsupported),
        }
    }
    Err(CacheError::Busy)
}

impl Snapshot {
    /// Reads a consistent copy through SQLite without opening the browser's database for writing.
    ///
    /// # Errors
    ///
    /// Refuses a busy, malformed, large, unsupported, or cancelled cache.
    pub fn open(
        path: &Path,
        format: Format,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self, CacheError> {
        if cancelled.load(Ordering::Relaxed) {
            return Err(CacheError::Stopped);
        }
        let source = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        let _source_work = configure(&source, &cancelled)?;
        source.pragma_update(None, "query_only", true)?;
        size(&source)?;
        let directory = tempfile::tempdir().map_err(CacheError::Temporary)?;
        let mut connection = Connection::open(directory.path().join(SNAPSHOT_FILE))?;
        backup(&source, &mut connection, &cancelled)?;
        let work = configure(&connection, &cancelled)?;
        connection.pragma_update(None, "query_only", true)?;
        let bytes = size(&connection)?;
        let snapshot = Self {
            connection,
            directory,
            format,
            cancelled,
            work,
            bytes,
        };
        snapshot.schema()?;
        Ok(snapshot)
    }

    fn schema(&self) -> Result<(), CacheError> {
        let tables: &[&str] = match self.format {
            Format::Chromium => &["icon_mapping", "favicons", "favicon_bitmaps"],
            Format::Firefox => &["moz_pages_w_icons", "moz_icons_to_pages", "moz_icons"],
            Format::Safari => &["page_url", "icon_info"],
        };
        for table in tables {
            let count: i64 = self.connection.query_row(
                "SELECT count(*) FROM sqlite_schema WHERE type='table' AND name=?1",
                [table],
                |row| row.get(0),
            )?;
            if count != 1 {
                return Err(CacheError::Unsupported);
            }
        }
        Ok(())
    }

    /// Returns bounded candidates for the exact page or another page on its host, best size first.
    ///
    /// # Errors
    ///
    /// Refuses malformed rows or a stopped SQLite work budget.
    pub fn candidates(&self, link: &Link) -> Result<Vec<Candidate>, CacheError> {
        if self.cancelled.load(Ordering::Relaxed) {
            return Err(CacheError::Stopped);
        }
        self.work.store(SQL_QUANTA, Ordering::Relaxed);
        let table = match self.format {
            Format::Chromium => CHROMIUM,
            Format::Firefox => FIREFOX,
            Format::Safari => SAFARI,
        };
        let query = format!(
            "WITH candidates AS ({table}) SELECT page,width,height,payload FROM candidates {FILTER}"
        );
        let host = link
            .host()
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let plain = format!("http://{host}");
        let secure = format!("https://{host}");
        let mut statement = self.connection.prepare(&query)?;
        let mut rows = statement.query(rusqlite::params![
            link.as_str().trim_end_matches('/'),
            plain,
            secure,
            format!("{plain}/%"),
            format!("{secure}/%"),
            format!("{plain}:%"),
            format!("{secure}:%"),
            i64::try_from(MOST_CANDIDATES).map_err(|_large| CacheError::TooLarge)?,
        ])?;
        let mut found = Vec::new();
        while let Some(row) = rows.next()? {
            let page: String = row.get(PAGE_COLUMN)?;
            if !Link::parse(&page).is_ok_and(|page| page.host() == link.host()) {
                continue;
            }
            let Some(quality) = quality(row.get(WIDTH_COLUMN)?, row.get(HEIGHT_COLUMN)?) else {
                continue;
            };
            let image = if self.format == Format::Safari {
                Image::SafariFile(safari_file(&row.get::<_, String>(PAYLOAD_COLUMN)?))
            } else {
                let value = row.get_ref(PAYLOAD_COLUMN)?;
                let blob = value.as_blob().map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        PAYLOAD_COLUMN,
                        value.data_type(),
                        Box::new(error),
                    )
                })?;
                if blob.is_empty() || blob.len() > MOST_IMAGE_BYTES {
                    continue;
                }
                Image::Bytes(blob.to_vec())
            };
            found.push(Candidate { quality, image });
        }
        found.sort_by_key(|candidate| std::cmp::Reverse(candidate.quality));
        Ok(found)
    }

    #[must_use]
    pub fn temporary_directory(&self) -> &Path {
        self.directory.path()
    }

    #[must_use]
    pub const fn storage_bytes(&self) -> u64 {
        self.bytes
    }
}
