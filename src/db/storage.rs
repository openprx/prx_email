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
        let sql = include_str!("../../migrations/0001_init.sql");
        self.conn.execute_batch(sql)?;
        Ok(())
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}
