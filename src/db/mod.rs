mod models;
mod repository;
mod storage;

pub use models::{Account, Folder, Message, NewAccount, NewFolder, NewMessage, SyncState, UpsertSyncState};
pub use repository::EmailRepository;
pub use storage::EmailStore;
