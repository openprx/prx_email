use rusqlite::Connection;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

pub struct EmailStore {
    conn: Connection,
}

impl EmailStore {
    pub fn open(path: &str) -> Result<Self, StorageError> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(Self { conn })
    }

    pub fn open_in_memory() -> Result<Self, StorageError> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(Self { conn })
    }

    pub fn migrate(&self) -> Result<(), StorageError> {
        let sql_0001 = include_str!("../../migrations/0001_init.sql");
        let sql_0002 = include_str!("../../migrations/0002_outbox.sql");
        self.conn.execute_batch(sql_0001)?;
        self.conn.execute_batch(sql_0002)?;
        Ok(())
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}
