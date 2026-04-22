//! Typed schema for `<plan>/memory.yaml`.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoryFile {
    #[serde(default)]
    pub entries: Vec<MemoryEntry>,
    /// Preserve unknown top-level keys across a read/write cycle so future
    /// schema extensions are not dropped by an older reader.
    #[serde(flatten)]
    pub extra: IndexMap<String, serde_yaml::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_round_trips_through_yaml() {
        let entry = MemoryEntry {
            id: "example".into(),
            title: "Example entry".into(),
            body: "Line one.\n\nLine two.\n".into(),
        };
        let yaml = serde_yaml::to_string(&entry).unwrap();
        let decoded: MemoryEntry = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(decoded.id, entry.id);
        assert_eq!(decoded.title, entry.title);
        assert_eq!(decoded.body, entry.body);
    }

    #[test]
    fn memory_file_preserves_unknown_top_level_keys() {
        let input = r#"
entries: []
schema_version: 1
"#;
        let parsed: MemoryFile = serde_yaml::from_str(input).unwrap();
        assert!(parsed.extra.contains_key("schema_version"));
        let re_emitted = serde_yaml::to_string(&parsed).unwrap();
        assert!(
            re_emitted.contains("schema_version"),
            "extra keys must round-trip: {re_emitted}"
        );
    }
}
