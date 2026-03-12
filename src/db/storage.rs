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

#[derive(Debug, Clone)]
pub struct StoreConfig {
    pub enable_wal: bool,
    pub busy_timeout_ms: u64,
    pub wal_autocheckpoint_pages: i64,
    pub synchronous: SynchronousMode,
}

#[derive(Debug, Clone, Copy)]
pub enum SynchronousMode {
    Full,
    Normal,
    Off,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            enable_wal: true,
            busy_timeout_ms: 5_000,
            wal_autocheckpoint_pages: 1_000,
            synchronous: SynchronousMode::Normal,
        }
    }
}

impl EmailStore {
    pub fn open(path: &str) -> Result<Self, StorageError> {
        Self::open_with_config(path, &StoreConfig::default())
    }

    pub fn open_with_config(path: &str, config: &StoreConfig) -> Result<Self, StorageError> {
        let conn = Connection::open(path)?;
        configure_connection(&conn, config)?;
        Ok(Self { conn })
    }

    pub fn open_in_memory() -> Result<Self, StorageError> {
        Self::open_in_memory_with_config(&StoreConfig::default())
    }

    pub fn open_in_memory_with_config(config: &StoreConfig) -> Result<Self, StorageError> {
        let conn = Connection::open_in_memory()?;
        configure_connection(&conn, config)?;
        Ok(Self { conn })
    }

    pub fn migrate(&self) -> Result<(), StorageError> {
        let sql_0001 = include_str!("../../migrations/0001_init.sql");
        let sql_0002 = include_str!("../../migrations/0002_outbox.sql");
        let sql_0003 = include_str!("../../migrations/0003_rollout.sql");
        self.conn.execute_batch(sql_0001)?;
        self.conn.execute_batch(sql_0002)?;
        self.conn.execute_batch(sql_0003)?;
        ensure_column(&self.conn, "messages", "body_html", "TEXT")?;
        ensure_column(&self.conn, "messages", "attachments_json", "TEXT")?;
        Ok(())
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}

fn ensure_column(
    conn: &Connection,
    table: &str,
    column: &str,
    column_sql_type: &str,
) -> Result<(), StorageError> {
    let pragma = format!("PRAGMA table_info({})", table);
    let mut stmt = conn.prepare(&pragma)?;
    let exists = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .any(|name| name == column);

    if !exists {
        let alter = format!("ALTER TABLE {} ADD COLUMN {} {}", table, column, column_sql_type);
        conn.execute_batch(&alter)?;
    }
    Ok(())
}

fn configure_connection(conn: &Connection, config: &StoreConfig) -> Result<(), StorageError> {
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    conn.busy_timeout(std::time::Duration::from_millis(config.busy_timeout_ms))?;

    if config.enable_wal {
        conn.execute_batch("PRAGMA journal_mode = WAL;")?;
        conn.execute_batch(&format!(
            "PRAGMA wal_autocheckpoint = {};",
            config.wal_autocheckpoint_pages
        ))?;
    }

    let sync_mode = match config.synchronous {
        SynchronousMode::Full => "FULL",
        SynchronousMode::Normal => "NORMAL",
        SynchronousMode::Off => "OFF",
    };
    conn.execute_batch(&format!("PRAGMA synchronous = {};", sync_mode))?;

    Ok(())
}
