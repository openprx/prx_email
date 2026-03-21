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
        let sql_0005 = include_str!("../../migrations/0005_m41.sql");
        let sql_0006 = include_str!("../../migrations/0006_m42_perf.sql");
        self.conn.execute_batch(sql_0001)?;
        self.conn.execute_batch(sql_0002)?;
        self.conn.execute_batch(sql_0003)?;
        self.conn.execute_batch(sql_0005)?;
        self.conn.execute_batch(sql_0006)?;
        ensure_column(&self.conn, "messages", "body_html", "TEXT")?;
        ensure_column(&self.conn, "messages", "attachments_json", "TEXT")?;
        ensure_column(&self.conn, "messages", "references_header", "TEXT")?;
        Ok(())
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}

/// Validate that a string is a safe SQL identifier (table/column name).
/// Allows only ASCII letters, digits, and underscores; must start with a
/// letter or underscore. Returns `Err` for anything else to prevent SQL injection.
fn validate_sql_identifier(name: &str) -> Result<(), StorageError> {
    if name.is_empty() || name.len() > 64 {
        return Err(StorageError::Sqlite(rusqlite::Error::InvalidParameterName(
            format!("invalid SQL identifier length: {}", name.len()),
        )));
    }
    let first = name.as_bytes()[0];
    if !(first.is_ascii_alphabetic() || first == b'_') {
        return Err(StorageError::Sqlite(rusqlite::Error::InvalidParameterName(
            format!("SQL identifier must start with letter or underscore: {name}"),
        )));
    }
    if !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return Err(StorageError::Sqlite(rusqlite::Error::InvalidParameterName(
            format!("SQL identifier contains invalid characters: {name}"),
        )));
    }
    Ok(())
}

/// Validate a SQL type token — must only contain safe ASCII characters
/// (letters, digits, underscores, spaces, parentheses). No semicolons,
/// quotes, or comment markers allowed.
fn validate_sql_type(type_str: &str) -> Result<(), StorageError> {
    if type_str.is_empty() || type_str.len() > 64 {
        return Err(StorageError::Sqlite(rusqlite::Error::InvalidParameterName(
            format!("invalid SQL type length: {}", type_str.len()),
        )));
    }
    if !type_str
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b' ' || b == b'(' || b == b')')
    {
        return Err(StorageError::Sqlite(rusqlite::Error::InvalidParameterName(
            format!("SQL type contains invalid characters: {type_str}"),
        )));
    }
    Ok(())
}

fn ensure_column(
    conn: &Connection,
    table: &str,
    column: &str,
    column_sql_type: &str,
) -> Result<(), StorageError> {
    // Validate identifiers/types to prevent SQL injection via format!
    validate_sql_identifier(table)?;
    validate_sql_identifier(column)?;
    validate_sql_type(column_sql_type)?;

    let pragma = format!("PRAGMA table_info({})", table);
    let mut stmt = conn.prepare(&pragma)?;
    let exists = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .any(|name| name == column);

    if !exists {
        let alter = format!(
            "ALTER TABLE {} ADD COLUMN {} {}",
            table, column, column_sql_type
        );
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
