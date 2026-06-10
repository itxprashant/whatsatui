use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// Directory for whatsatui's on-disk caches (`WHATSATUI_CACHE_DIR` or `~/.cache/whatsatui`).
pub fn cache_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("WHATSATUI_CACHE_DIR") {
        if !dir.trim().is_empty() {
            return Some(PathBuf::from(dir));
        }
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache/whatsatui"))
}

fn groups_cache_file() -> Option<PathBuf> {
    cache_dir().map(|d| d.join("groups.json"))
}

/// Load cached group subjects (jid -> name). Returns empty on missing or corrupt cache.
pub fn load_group_names() -> HashMap<String, String> {
    let Some(path) = groups_cache_file() else {
        return HashMap::new();
    };
    let Ok(bytes) = fs::read(&path) else {
        return HashMap::new();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// Persist group subjects for instant display on the next startup.
pub fn save_group_names(groups: &HashMap<String, String>) {
    let Some(path) = groups_cache_file() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_vec(groups) {
        let _ = fs::write(path, json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_group_cache() {
        let dir = std::env::temp_dir().join(format!("whatsatui-cache-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        std::env::set_var("WHATSATUI_CACHE_DIR", &dir);

        let mut groups = HashMap::new();
        groups.insert("120@g.us".to_string(), "Test Group".to_string());
        save_group_names(&groups);
        let loaded = load_group_names();
        assert_eq!(loaded.get("120@g.us").map(String::as_str), Some("Test Group"));

        let _ = fs::remove_dir_all(&dir);
        std::env::remove_var("WHATSATUI_CACHE_DIR");
    }
}
