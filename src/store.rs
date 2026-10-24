//! Durable state in a small SQLite file (see ADR 0007).
//!
//! The bot works without it: with no data directory, state lives in memory
//! and resets on restart. With one, trusted members, learned samples,
//! warnings and spam reports survive restarts.
//!
//! [`Store`] is a cheap, cloneable handle; every component that persists
//! something keeps its own clone.

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

/// File name inside the data directory.
pub const DB_FILE: &str = "antispam.db";

/// Schema migrations, applied in order; `PRAGMA user_version` counts them.
const MIGRATIONS: &[&str] = &[
    "CREATE TABLE members (
        chat_id        INTEGER NOT NULL,
        user_id        INTEGER NOT NULL,
        clean_messages INTEGER NOT NULL DEFAULT 0,
        trusted        INTEGER NOT NULL DEFAULT 0,
        updated_at     INTEGER NOT NULL DEFAULT (unixepoch()),
        PRIMARY KEY (chat_id, user_id)
    ) WITHOUT ROWID;",
    "CREATE TABLE samples (
        id         INTEGER PRIMARY KEY,
        kind       TEXT    NOT NULL CHECK (kind IN ('spam', 'ham')),
        text       TEXT    NOT NULL,
        added_at   INTEGER NOT NULL DEFAULT (unixepoch())
    );
    CREATE TABLE warnings (
        chat_id    INTEGER NOT NULL,
        user_id    INTEGER NOT NULL,
        count      INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        PRIMARY KEY (chat_id, user_id)
    ) WITHOUT ROWID;
    CREATE TABLE reports (
        id         INTEGER PRIMARY KEY,
        chat_id    INTEGER NOT NULL,
        sender_id  INTEGER NOT NULL,
        text       TEXT    NOT NULL,
        created_at INTEGER NOT NULL DEFAULT (unixepoch())
    );",
    // Samples per chat (ADR 0011). Older samples have no chat and may have
    // been planted from any chat, so they are set aside, never loaded.
    "ALTER TABLE samples RENAME TO samples_unscoped;
    CREATE TABLE samples (
        id         INTEGER PRIMARY KEY,
        chat_id    INTEGER NOT NULL,
        kind       TEXT    NOT NULL CHECK (kind IN ('spam', 'ham')),
        text       TEXT    NOT NULL,
        added_at   INTEGER NOT NULL DEFAULT (unixepoch()),
        UNIQUE (chat_id, kind, text)
    );",
];

/// One member's standing in one chat.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemberRecord {
    pub clean_messages: u32,
    pub trusted: bool,
}

/// Kind of a learned sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SampleKind {
    Spam,
    Ham,
}

impl SampleKind {
    fn as_str(self) -> &'static str {
        match self {
            SampleKind::Spam => "spam",
            SampleKind::Ham => "ham",
        }
    }
}

/// A learned sample as stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SampleRecord {
    pub id: i64,
    pub chat_id: i64,
    pub text: String,
}

/// Admin warnings for one member: how many, and when the last one was given
/// (Unix seconds).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WarningRecord {
    pub count: u32,
    pub updated_at: i64,
}

/// What a spam report remembers, so its buttons work after a restart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportRecord {
    pub chat_id: i64,
    pub sender_id: i64,
    pub text: String,
}

/// Handle to the SQLite database.
#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

impl Store {
    /// Open (or create) the database in `dir`.
    pub fn open(dir: &Path) -> Result<Store> {
        let path = dir.join(DB_FILE);
        let conn =
            Connection::open(&path).with_context(|| format!("cannot open {}", path.display()))?;
        // WAL keeps reads cheap while a write is in progress.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Store::with_connection(conn)
    }

    /// A throwaway database for tests.
    pub fn in_memory() -> Result<Store> {
        Store::with_connection(Connection::open_in_memory()?)
    }

    fn with_connection(conn: Connection) -> Result<Store> {
        migrate(&conn)?;
        Ok(Store {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// The connection. A poisoned lock only means another thread panicked
    /// mid-call; SQLite itself is still consistent.
    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Every stored member, keyed by `(chat_id, user_id)`.
    pub fn members(&self) -> Result<Vec<((i64, i64), MemberRecord)>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare("SELECT chat_id, user_id, clean_messages, trusted FROM members")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                (row.get(0)?, row.get(1)?),
                MemberRecord {
                    clean_messages: row.get(2)?,
                    trusted: row.get(3)?,
                },
            ))
        })?;
        rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
    }

    /// Insert or replace one member's record.
    pub fn save_member(&self, chat_id: i64, user_id: i64, record: MemberRecord) -> Result<()> {
        self.conn().execute(
            "INSERT INTO members (chat_id, user_id, clean_messages, trusted, updated_at)
             VALUES (?1, ?2, ?3, ?4, unixepoch())
             ON CONFLICT (chat_id, user_id) DO UPDATE SET
                 clean_messages = excluded.clean_messages,
                 trusted = excluded.trusted,
                 updated_at = excluded.updated_at",
            params![chat_id, user_id, record.clean_messages, record.trusted],
        )?;
        Ok(())
    }

    /// Learned samples of `kind` in all chats, oldest first.
    pub fn samples(&self, kind: SampleKind) -> Result<Vec<SampleRecord>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare("SELECT id, chat_id, text FROM samples WHERE kind = ?1 ORDER BY id")?;
        let rows = stmt.query_map([kind.as_str()], |row| {
            Ok(SampleRecord {
                id: row.get(0)?,
                chat_id: row.get(1)?,
                text: row.get(2)?,
            })
        })?;
        rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
    }

    /// Add a sample to `chat_id` and return its ID, or `None` if the chat
    /// already has this exact text. Limits are kept by the caller.
    pub fn add_sample(&self, chat_id: i64, kind: SampleKind, text: &str) -> Result<Option<i64>> {
        self.conn()
            .query_row(
                "INSERT INTO samples (chat_id, kind, text) VALUES (?1, ?2, ?3)
                 ON CONFLICT DO NOTHING RETURNING id",
                params![chat_id, kind.as_str(), text],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    /// Delete the sample with `id`.
    pub fn remove_sample(&self, id: i64) -> Result<()> {
        self.conn()
            .execute("DELETE FROM samples WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Every stored warning record, keyed by `(chat_id, user_id)`.
    pub fn warnings(&self) -> Result<Vec<((i64, i64), WarningRecord)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT chat_id, user_id, count, updated_at FROM warnings")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                (row.get(0)?, row.get(1)?),
                WarningRecord {
                    count: row.get(2)?,
                    updated_at: row.get(3)?,
                },
            ))
        })?;
        rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
    }

    /// Insert or replace one member's warnings.
    pub fn save_warning(&self, chat_id: i64, user_id: i64, record: WarningRecord) -> Result<()> {
        self.conn().execute(
            "INSERT INTO warnings (chat_id, user_id, count, updated_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (chat_id, user_id) DO UPDATE SET
                 count = excluded.count, updated_at = excluded.updated_at",
            params![chat_id, user_id, record.count, record.updated_at],
        )?;
        Ok(())
    }

    /// Save a report and return its ID; only the newest `keep` are kept.
    pub fn add_report(&self, report: &ReportRecord, keep: usize) -> Result<i64> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO reports (chat_id, sender_id, text) VALUES (?1, ?2, ?3)",
            params![report.chat_id, report.sender_id, report.text],
        )?;
        let id = conn.last_insert_rowid();
        conn.execute(
            "DELETE FROM reports WHERE id <= ?1",
            params![id - keep as i64],
        )?;
        Ok(id)
    }

    /// The report with `id`, if it is still kept.
    pub fn report(&self, id: i64) -> Result<Option<ReportRecord>> {
        self.conn()
            .query_row(
                "SELECT chat_id, sender_id, text FROM reports WHERE id = ?1",
                [id],
                |row| {
                    Ok(ReportRecord {
                        chat_id: row.get(0)?,
                        sender_id: row.get(1)?,
                        text: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }
}

fn migrate(conn: &Connection) -> Result<()> {
    let applied: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    for (version, sql) in (1..).zip(MIGRATIONS).skip(applied.max(0) as usize) {
        conn.execute_batch(sql)
            .with_context(|| format!("migration {version} failed"))?;
        conn.pragma_update(None, "user_version", version)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn members_round_trip() {
        let store = Store::in_memory().unwrap();
        let first = MemberRecord {
            clean_messages: 2,
            trusted: false,
        };
        let second = MemberRecord {
            clean_messages: 3,
            trusted: true,
        };

        store.save_member(-100, 42, first).unwrap();
        store.save_member(-100, 42, second).unwrap();

        assert_eq!(store.members().unwrap(), vec![((-100, 42), second)]);
    }

    #[test]
    fn samples_are_kept_per_chat_and_kind() {
        let store = Store::in_memory().unwrap();

        let first = store.add_sample(-1, SampleKind::Spam, "a b c").unwrap();
        let again = store.add_sample(-1, SampleKind::Spam, "a b c").unwrap();
        let other_chat = store.add_sample(-2, SampleKind::Spam, "a b c").unwrap();
        store.add_sample(-1, SampleKind::Ham, "a b c").unwrap();
        store.remove_sample(other_chat.unwrap()).unwrap();

        assert!(first.is_some());
        assert_eq!(again, None, "a chat keeps one copy of a text");
        let spam = store.samples(SampleKind::Spam).unwrap();
        assert_eq!(
            spam,
            vec![SampleRecord {
                id: first.unwrap(),
                chat_id: -1,
                text: "a b c".to_string()
            }]
        );
        assert_eq!(store.samples(SampleKind::Ham).unwrap().len(), 1);
    }

    #[test]
    fn migration_sets_unscoped_samples_aside() {
        let conn = Connection::open_in_memory().unwrap();
        for sql in &MIGRATIONS[..2] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 2).unwrap();
        conn.execute(
            "INSERT INTO samples (kind, text) VALUES ('spam', 'old global sample')",
            [],
        )
        .unwrap();

        let store = Store::with_connection(conn).unwrap();

        let conn = store.conn();
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        let unscoped: i64 = conn
            .query_row("SELECT count(*) FROM samples_unscoped", [], |row| {
                row.get(0)
            })
            .unwrap();
        drop(conn);
        assert_eq!(version, MIGRATIONS.len() as i64);
        assert_eq!(unscoped, 1);
        assert!(store.samples(SampleKind::Spam).unwrap().is_empty());
        assert!(store
            .add_sample(-1, SampleKind::Spam, "old global sample")
            .unwrap()
            .is_some());
    }

    #[test]
    fn warnings_round_trip() {
        let store = Store::in_memory().unwrap();
        let record = WarningRecord {
            count: 2,
            updated_at: 1_700_000_000,
        };
        store.save_warning(-1, 5, record).unwrap();
        assert_eq!(store.warnings().unwrap(), vec![((-1, 5), record)]);
    }

    #[test]
    fn old_reports_are_dropped() {
        let store = Store::in_memory().unwrap();
        let report = |text: &str| ReportRecord {
            chat_id: -1,
            sender_id: 7,
            text: text.to_string(),
        };
        let first = store.add_report(&report("one"), 2).unwrap();
        let second = store.add_report(&report("two"), 2).unwrap();
        let third = store.add_report(&report("three"), 2).unwrap();

        assert_eq!(store.report(first).unwrap(), None);
        assert_eq!(store.report(second).unwrap(), Some(report("two")));
        assert_eq!(store.report(third).unwrap(), Some(report("three")));
    }

    #[test]
    fn reopening_keeps_data_and_skips_applied_migrations() {
        let dir = std::env::temp_dir().join(format!("antispam-store-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        {
            let store = Store::open(&dir).unwrap();
            store.save_member(1, 2, MemberRecord::default()).unwrap();
        }
        let store = Store::open(&dir).unwrap();
        assert_eq!(store.members().unwrap().len(), 1);
        drop(store);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
