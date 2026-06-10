use std::collections::HashMap;
use std::path::Path;

/// Push/business names from the gateway's whatsmeow DB (`whatsmeow_contacts`).
/// The REST chat list often stores the wrong `name` for business accounts; these
/// fields match what WhatsApp shows on the phone.
pub fn load_push_names(db_path: &Path) -> HashMap<String, String> {
    if !db_path.exists() {
        return HashMap::new();
    }
    let Ok(conn) = rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ) else {
        return HashMap::new();
    };
    let mut stmt = match conn.prepare(
        "SELECT their_jid, business_name, push_name, full_name, first_name
         FROM whatsmeow_contacts",
    ) {
        Ok(s) => s,
        Err(_) => return HashMap::new(),
    };
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
        ))
    });
    let Ok(rows) = rows else {
        return HashMap::new();
    };

    let mut out: HashMap<String, String> = HashMap::new();
    for row in rows.flatten() {
        let (jid, business, push, full, first) = row;
        let name = pick_name(business, push, full, first);
        if name.is_empty() {
            continue;
        }
        // Prefer business_name-bearing entries when multiple our_jid rows exist.
        match out.get(&jid) {
            Some(existing) if !existing.is_empty() => {}
            _ => {
                out.insert(jid, name);
            }
        }
    }
    out
}

fn pick_name(
    business: Option<String>,
    push: Option<String>,
    full: Option<String>,
    first: Option<String>,
) -> String {
    for candidate in [business, push, full, first] {
        if let Some(s) = candidate {
            let s = s.trim().to_string();
            if !s.is_empty() {
                return s;
            }
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_business_name_from_whatsmeow_db() {
        let dir = std::env::temp_dir().join(format!("whatsatui-contacts-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("wa.db");
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            conn.execute_batch(
                "CREATE TABLE whatsmeow_contacts (
                    our_jid TEXT, their_jid TEXT,
                    first_name TEXT, full_name TEXT,
                    push_name TEXT, business_name TEXT, redacted_phone TEXT,
                    PRIMARY KEY (our_jid, their_jid));
                 INSERT INTO whatsmeow_contacts VALUES
                    ('me@s.whatsapp.net', '916366800400@s.whatsapp.net',
                     '', '', '', 'Acer India', '');",
            )
            .unwrap();
        }
        let names = load_push_names(&db);
        assert_eq!(
            names.get("916366800400@s.whatsapp.net").map(String::as_str),
            Some("Acer India")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prefers_business_name_over_push_name() {
        assert_eq!(
            pick_name(
                Some("Acer India".into()),
                Some("Other".into()),
                None,
                None,
            ),
            "Acer India"
        );
    }
}
