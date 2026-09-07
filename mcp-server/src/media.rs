use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use tracing::warn;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MediaItem {
    pub id: String,
    pub title: String,
    #[serde(rename = "type")]
    pub media_type: String,
    pub topic: String,
    pub keywords: Vec<String>,
    pub mime_type: String,
    pub folder: String,
    pub filename: String,
    pub uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct MediaRegistry {
    items: Vec<MediaItem>,
}

impl MediaRegistry {
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// Load media items from `index.json` files found in subdirectories of `workspace_root`
    /// (e.g. `kuka-movies/index.json`, `kuka-prints/index.json`).
    pub fn load(workspace_root: &Path) -> Result<Self> {
        let mut items = Vec::new();

        if !workspace_root.exists() || !workspace_root.is_dir() {
            return Ok(Self { items });
        }

        let entries = match fs::read_dir(workspace_root) {
            Ok(entries) => entries,
            Err(err) => {
                warn!(path = %workspace_root.display(), "Failed to read workspace dir for media: {err}");
                return Ok(Self { items });
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let index_file = path.join("index.json");
                if index_file.is_file() {
                    match fs::read_to_string(&index_file) {
                        Ok(content) => match serde_json::from_str::<Vec<MediaItem>>(&content) {
                            Ok(mut loaded_items) => {
                                items.append(&mut loaded_items);
                            }
                            Err(err) => {
                                warn!(path = %index_file.display(), "Failed to parse media index JSON: {err}");
                            }
                        },
                        Err(err) => {
                            warn!(path = %index_file.display(), "Failed to read media index file: {err}");
                        }
                    }
                }
            }
        }

        Ok(Self { items })
    }

    pub fn items(&self) -> &[MediaItem] {
        &self.items
    }

    /// Query media resources with optional filters for `topic`, `keyword`, and `media_type`.
    pub fn list_media(
        &self,
        topic: Option<&str>,
        keyword: Option<&str>,
        media_type: Option<&str>,
    ) -> Vec<MediaItem> {
        let topic = topic
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty());
        let keyword = keyword
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty());
        let media_type = media_type
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty());

        self.items
            .iter()
            .filter(|item| {
                if let Some(ref t) = topic {
                    let matches_topic = item.topic.to_lowercase().contains(t)
                        || item.keywords.iter().any(|kw| kw.to_lowercase().contains(t));
                    if !matches_topic {
                        return false;
                    }
                }

                if media_type
                    .as_ref()
                    .is_some_and(|m| !item.media_type.to_lowercase().contains(m))
                {
                    return false;
                }

                if let Some(ref k) = keyword {
                    let matches_keyword =
                        item.keywords.iter().any(|kw| kw.to_lowercase().contains(k))
                            || item.id.to_lowercase().contains(k)
                            || item.title.to_lowercase().contains(k)
                            || item.topic.to_lowercase().contains(k)
                            || item
                                .description
                                .as_ref()
                                .is_some_and(|d| d.to_lowercase().contains(k));
                    if !matches_keyword {
                        return false;
                    }
                }

                true
            })
            .cloned()
            .collect()
    }

    /// Retrieve a specific media item by `id` or `keyword`.
    pub fn get_media(&self, id: Option<&str>, keyword: Option<&str>) -> Option<MediaItem> {
        if let Some(id_str) = id.map(|s| s.trim()).filter(|s| !s.is_empty()) {
            let id_lower = id_str.to_lowercase();
            if let Some(found) = self
                .items
                .iter()
                .find(|item| item.id.to_lowercase() == id_lower)
            {
                return Some(found.clone());
            }
        }

        if let Some(kw_str) = keyword.map(|s| s.trim()).filter(|s| !s.is_empty()) {
            let kw_lower = kw_str.to_lowercase();
            if let Some(found) = self.items.iter().find(|item| {
                item.id.to_lowercase().contains(&kw_lower)
                    || item
                        .keywords
                        .iter()
                        .any(|k| k.to_lowercase().contains(&kw_lower))
                    || item.title.to_lowercase().contains(&kw_lower)
            }) {
                return Some(found.clone());
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_test_media_registry() -> (TempDir, MediaRegistry) {
        let temp_dir = TempDir::new().unwrap();
        let movies_dir = temp_dir.path().join("kuka-movies");
        let prints_dir = temp_dir.path().join("kuka-prints");

        fs::create_dir_all(&movies_dir).unwrap();
        fs::create_dir_all(&prints_dir).unwrap();

        let movies_json = r#"[
            {
                "id": "video-1",
                "title": "Localization Demo",
                "type": "video",
                "topic": "localization",
                "keywords": ["localization", "log"],
                "mimeType": "video/mp4",
                "folder": "kuka-movies",
                "filename": "loc.mp4",
                "uri": "kuka://media/kuka-movies/loc.mp4"
            }
        ]"#;

        let prints_json = r#"[
            {
                "id": "print-1",
                "title": "Charger Schematic",
                "type": "print",
                "topic": "electrical",
                "keywords": ["electrical", "charger", "schematic"],
                "mimeType": "application/pdf",
                "folder": "kuka-prints",
                "filename": "charger.pdf",
                "uri": "kuka://media/kuka-prints/charger.pdf",
                "description": "24V charger schematic"
            }
        ]"#;

        fs::write(movies_dir.join("index.json"), movies_json).unwrap();
        fs::write(prints_dir.join("index.json"), prints_json).unwrap();

        let registry = MediaRegistry::load(temp_dir.path()).unwrap();
        (temp_dir, registry)
    }

    #[test]
    fn test_load_media_registry() {
        let (_dir, registry) = setup_test_media_registry();
        assert_eq!(registry.items().len(), 2);
    }

    #[test]
    fn test_list_media_filters() {
        let (_dir, registry) = setup_test_media_registry();

        let ele = registry.list_media(Some("electrical"), None, None);
        assert_eq!(ele.len(), 1);
        assert_eq!(ele[0].id, "print-1");

        let videos = registry.list_media(None, None, Some("video"));
        assert_eq!(videos.len(), 1);
        assert_eq!(videos[0].id, "video-1");

        let kw = registry.list_media(None, Some("schematic"), None);
        assert_eq!(kw.len(), 1);
        assert_eq!(kw[0].id, "print-1");

        // Topic filter falls through to keywords if primary topic is different
        let kw_topic = registry.list_media(Some("charger"), None, None);
        assert_eq!(kw_topic.len(), 1);
        assert_eq!(kw_topic[0].id, "print-1");
    }

    #[test]
    fn test_get_media() {
        let (_dir, registry) = setup_test_media_registry();

        let item1 = registry.get_media(Some("print-1"), None);
        assert!(item1.is_some());
        assert_eq!(item1.unwrap().title, "Charger Schematic");

        let item2 = registry.get_media(None, Some("log"));
        assert!(item2.is_some());
        assert_eq!(item2.unwrap().id, "video-1");
    }
}
