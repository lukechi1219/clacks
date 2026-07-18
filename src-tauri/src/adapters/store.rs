//! MessageStore adapter: rusqlite 去重落地。
//! nexus 對照實證(findings): 去重狀態必須落地——骨架只放記憶體,
//! 重啟即重收 backlog。adapter 保持愚蠢: 只有 insert-or-ignore,
//! 無清理政策、無 TTL(政策屬 core/orchestrator, 目前不需要)

use crate::ports::{MessageStore, StoreError};
use rusqlite::Connection;
use std::path::Path;

pub struct SqliteStore {
    conn: Connection,
}

impl SqliteStore {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open(path).map_err(|e| StoreError(e.to_string()))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS seen_updates (update_id INTEGER PRIMARY KEY);",
        )
        .map_err(|e| StoreError(e.to_string()))?;
        Ok(Self { conn })
    }
}

impl MessageStore for SqliteStore {
    fn first_seen(&mut self, update_id: i64) -> Result<bool, StoreError> {
        let inserted = self
            .conn
            .execute(
                "INSERT OR IGNORE INTO seen_updates (update_id) VALUES (?1)",
                rusqlite::params![update_id],
            )
            .map_err(|e| StoreError(e.to_string()))?;
        Ok(inserted == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::MessageStore;

    #[test]
    fn dedups_within_one_session() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = SqliteStore::open(&dir.path().join("clacks.db")).unwrap();
        assert!(store.first_seen(7).unwrap());
        assert!(!store.first_seen(7).unwrap());
        assert!(store.first_seen(8).unwrap());
    }

    #[test]
    fn dedup_survives_reopen() {
        // 落地的意義: 重啟不得重收 backlog(nexus 對照實證)
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("clacks.db");
        {
            let mut store = SqliteStore::open(&db_path).unwrap();
            assert!(store.first_seen(7).unwrap());
        }
        let mut reopened = SqliteStore::open(&db_path).unwrap();
        assert!(!reopened.first_seen(7).unwrap());
        assert!(reopened.first_seen(8).unwrap());
    }

    #[test]
    fn unopenable_path_reports_error() {
        let result = SqliteStore::open(Path::new("/nonexistent-dir/clacks.db"));
        assert!(result.is_err());
    }
}
