use std::future::Future;
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use futures_channel::oneshot;
use rusqlite::{Connection, OptionalExtension as _, params};

pub const VIEWED_RETENTION_SECS: i64 = 180 * 24 * 60 * 60;
pub const AVATAR_RETENTION_SECS: i64 = 90 * 24 * 60 * 60;

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const THREAD_NAME: &str = "nitpick-db";
const WAL_ATTEMPTS: usize = 100;
const WAL_RETRY: Duration = Duration::from_millis(10);

const MIGRATIONS: &[&str] = &["
    CREATE TABLE viewed (
        repo TEXT NOT NULL,
        path TEXT NOT NULL,
        fingerprint TEXT NOT NULL,
        viewed_at INTEGER NOT NULL,
        PRIMARY KEY (repo, path, fingerprint)
    ) WITHOUT ROWID;
    CREATE INDEX viewed_age ON viewed (viewed_at);
    CREATE TABLE avatars (
        url TEXT PRIMARY KEY,
        png BLOB NOT NULL,
        fetched_at INTEGER NOT NULL
    );
"];

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("database folder: {0}")]
    Io(#[from] std::io::Error),
    #[error("the database was written by a newer version (schema {0})")]
    TooNew(i64),
    #[error("the database thread has stopped")]
    Stopped,
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewedMark {
    pub path: String,
    pub fingerprint: String,
    pub viewed_at: i64,
}

type Job = Box<dyn FnOnce(&mut Connection) + Send>;

#[derive(Clone)]
pub struct Db {
    jobs: mpsc::Sender<Job>,
}

impl Db {
    pub fn open(path: &Path) -> Result<Db> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        enable_wal(&conn)?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Db::start(conn)
    }

    pub fn open_in_memory() -> Result<Db> {
        Db::start(Connection::open_in_memory()?)
    }

    fn start(mut conn: Connection) -> Result<Db> {
        conn.busy_timeout(BUSY_TIMEOUT)?;
        migrate(&mut conn)?;
        let (jobs, queue) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name(THREAD_NAME.into())
            .spawn(move || {
                for job in queue {
                    let run = std::panic::AssertUnwindSafe(|| job(&mut conn));
                    if std::panic::catch_unwind(run).is_err() {
                        log::error!("a database job panicked");
                    }
                }
            })?;
        Ok(Db { jobs })
    }

    fn call<T, F>(&self, work: F) -> impl Future<Output = Result<T>> + Send + 'static + use<T, F>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    {
        let (reply, answer) = oneshot::channel();
        let queued = self
            .jobs
            .send(Box::new(move |conn| {
                let _ = reply.send(work(conn));
            }))
            .is_ok();
        async move {
            if !queued {
                return Err(Error::Stopped);
            }
            answer.await.map_err(|_| Error::Stopped)?
        }
    }

    pub fn barrier(&self) -> impl Future<Output = Result<()>> + Send + 'static + use<> {
        self.call(|_| Ok(()))
    }

    pub fn load_viewed(
        &self,
        repo: &str,
        now: i64,
    ) -> impl Future<Output = Result<Vec<ViewedMark>>> + Send + 'static + use<> {
        let repo = repo.to_string();
        self.call(move |conn| {
            conn.execute(
                "DELETE FROM viewed WHERE repo = ?1 AND viewed_at < ?2",
                params![repo, now - VIEWED_RETENTION_SECS],
            )?;
            let mut query =
                conn.prepare("SELECT path, fingerprint, viewed_at FROM viewed WHERE repo = ?1")?;
            let rows = query.query_map([&repo], |row| {
                Ok(ViewedMark {
                    path: row.get(0)?,
                    fingerprint: row.get(1)?,
                    viewed_at: row.get(2)?,
                })
            })?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    pub fn set_viewed(
        &self,
        repo: &str,
        marks: Vec<(String, String)>,
        viewed: bool,
        at: i64,
    ) -> impl Future<Output = Result<()>> + Send + 'static + use<> {
        let repo = repo.to_string();
        self.call(move |conn| {
            let tx = conn.transaction()?;
            {
                let mut statement = if viewed {
                    tx.prepare(
                        "INSERT OR REPLACE INTO viewed (repo, path, fingerprint, viewed_at)
                         VALUES (?1, ?2, ?3, ?4)",
                    )?
                } else {
                    tx.prepare(
                        "DELETE FROM viewed WHERE repo = ?1 AND path = ?2 AND fingerprint = ?3",
                    )?
                };
                for (path, fingerprint) in &marks {
                    if viewed {
                        statement.execute(params![repo, path, fingerprint, at])?;
                    } else {
                        statement.execute(params![repo, path, fingerprint])?;
                    }
                }
            }
            tx.commit()?;
            Ok(())
        })
    }

    pub fn avatar(
        &self,
        url: &str,
    ) -> impl Future<Output = Result<Option<(Vec<u8>, i64)>>> + Send + 'static + use<> {
        let url = url.to_string();
        self.call(move |conn| {
            Ok(conn
                .query_row(
                    "SELECT png, fetched_at FROM avatars WHERE url = ?1",
                    [&url],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?)
        })
    }

    pub fn put_avatar(
        &self,
        url: &str,
        png: Vec<u8>,
        at: i64,
    ) -> impl Future<Output = Result<()>> + Send + 'static + use<> {
        let url = url.to_string();
        self.call(move |conn| {
            conn.execute(
                "INSERT OR REPLACE INTO avatars (url, png, fetched_at) VALUES (?1, ?2, ?3)",
                params![url, png, at],
            )?;
            Ok(())
        })
    }

    pub fn prune_avatars(
        &self,
        now: i64,
    ) -> impl Future<Output = Result<usize>> + Send + 'static + use<> {
        self.call(move |conn| {
            Ok(conn.execute(
                "DELETE FROM avatars WHERE fetched_at < ?1",
                [now - AVATAR_RETENTION_SECS],
            )?)
        })
    }
}

fn enable_wal(conn: &Connection) -> Result<()> {
    for _ in 0..WAL_ATTEMPTS {
        match conn.pragma_update(None, "journal_mode", "WAL") {
            Err(rusqlite::Error::SqliteFailure(err, _))
                if err.code == rusqlite::ErrorCode::DatabaseBusy =>
            {
                std::thread::sleep(WAL_RETRY);
            }
            other => return Ok(other?),
        }
    }
    Ok(conn.pragma_update(None, "journal_mode", "WAL")?)
}

fn migrate(conn: &mut Connection) -> Result<()> {
    let latest = MIGRATIONS.len() as i64;
    loop {
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let version: i64 =
            tx.query_row("SELECT user_version FROM pragma_user_version", [], |row| {
                row.get(0)
            })?;
        if version > latest {
            return Err(Error::TooNew(version));
        }
        let Some(sql) = MIGRATIONS.get(version as usize) else {
            return Ok(());
        };
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", version + 1)?;
        tx.commit()?;
    }
}

#[cfg(test)]
mod tests;
