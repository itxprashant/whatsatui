use std::collections::HashSet;
use std::path::Path;

/// Load jids WhatsApp marks archived from the gateway's whatsmeow database.
/// The REST `archived=true` filter reads `chatstorage.db`, which is often out of
/// sync; `whatsmeow_chat_settings.archived` matches the phone.
pub fn load_archived_jids(db_path: &Path) -> HashSet<String> {
    if !db_path.exists() {
        return HashSet::new();
    }
    let Ok(conn) = rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ) else {
        return HashSet::new();
    };
    let mut stmt = match conn.prepare(
        "SELECT chat_jid FROM whatsmeow_chat_settings WHERE archived = 1",
    ) {
        Ok(s) => s,
        Err(_) => return HashSet::new(),
    };
    let rows = stmt.query_map([], |row| row.get::<_, String>(0));
    let Ok(rows) = rows else {
        return HashSet::new();
    };
    rows.filter_map(Result::ok).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_archived_jids_from_whatsmeow_db() {
        let dir = std::env::temp_dir().join(format!("whatsatui-archive-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("wa.db");
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            conn.execute_batch(
                "CREATE TABLE whatsmeow_chat_settings (
                    our_jid TEXT, chat_jid TEXT, muted_until BIGINT,
                    pinned BOOLEAN, archived BOOLEAN,
                    PRIMARY KEY (our_jid, chat_jid));
                 INSERT INTO whatsmeow_chat_settings VALUES
                    ('me@s.whatsapp.net', '120@g.us', -1, 0, 1),
                    ('me@s.whatsapp.net', '919@s.whatsapp.net', -1, 0, 0);",
            )
            .unwrap();
        }
        let jids = load_archived_jids(&db);
        assert_eq!(jids.len(), 1);
        assert!(jids.contains("120@g.us"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
