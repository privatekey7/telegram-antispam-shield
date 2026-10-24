//! Durable state in a small SQLite file (see ADR 0007).
//!
//! The bot works without it: with no data directory, state lives in memory
//! and resets on restart. With one, trusted members survive restarts, so a
//! redeploy does not put every regular back under the spam model.

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

/// File name inside the data directory.
pub const DB_FILE: &str = "antispam.db";

/// Schema migrations, applied in order; `PRAGMA user_version` counts them.
const MIGRATIONS: &[&str] = &["CREATE TABLE members (
        chat_id        INTEGER NOT NULL,
        user_id        INTEGER NOT NULL,
        clean_messages INTEGER NOT NULL DEFAULT 0,
        trusted        INTEGER NOT NULL DEFAULT 0,
        updated_at     INTEGER NOT NULL DEFAULT (unixepoch()),
        PRIMARY KEY (chat_id, user_id)
    ) WITHOUT ROWID;"];

/// One member's standing in one chat.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemberRecord {
    pub clean_messages: u32,
    pub trusted: bool,
}

/// Handle to the SQLite database.
pub struct Store {
    conn: Connection,
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
        let store = Store { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<()> {
        let applied: i64 = self
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))?;
        for (version, sql) in (1..).zip(MIGRATIONS).skip(applied.max(0) as usize) {
            self.conn
                .execute_batch(sql)
                .with_context(|| format!("migration {version} failed"))?;
            self.conn.pragma_update(None, "user_version", version)?;
        }
        Ok(())
    }

    /// Every stored member, keyed by `(chat_id, user_id)`.
    pub fn members(&self) -> Result<Vec<((i64, i64), MemberRecord)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT chat_id, user_id, clean_messages, trusted FROM members")?;
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
        self.conn.execute(
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn members_round_trip() {
        let store = Store::in_memory().unwrap();
        let record = MemberRecord {
            clean_messages: 2,
            trusted: false,
        };

        store.save_member(-100, 42, record).unwrap();
        store
            .save_member(
                -100,
                42,
                MemberRecord {
                    clean_messages: 3,
                    trusted: true,
                },
            )
            .unwrap();

        let members = store.members().unwrap();
        assert_eq!(
            members,
            vec![(
                (-100, 42),
                MemberRecord {
                    clean_messages: 3,
                    trusted: true
                }
            )]
        );
    }

    #[test]
    fn reopening_keeps_data_and_skips_applied_migrations() {
        let dir = std::env::temp_dir().join(format!("antispam-store-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        {
            let store = Store::open(&dir).unwrap();
            store
                .save_member(
                    1,
                    2,
                    MemberRecord {
                        clean_messages: 1,
                        trusted: false,
                    },
                )
                .unwrap();
        }
        let store = Store::open(&dir).unwrap();
        assert_eq!(store.members().unwrap().len(), 1);
        drop(store);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
