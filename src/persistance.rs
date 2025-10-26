use directories_next::ProjectDirs;
use ron::de::from_str;
use ron::ser::{to_string_pretty, PrettyConfig};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self};
use std::path::PathBuf;
use log::info;

pub const APP_KEY: &str = "app";
pub const WINDOW_KEY: &str = "window";

#[derive(Serialize, Deserialize, Default)]
struct StorageData {
    map: HashMap<String, String>,
}

pub struct Persistence {
    path: PathBuf,
    data: StorageData,
}

impl Persistence {
    pub fn new(app_name: &str, organization: &str) -> Self {
        let path = ProjectDirs::from("com", organization, app_name)
            .expect("Could not determine project dirs")
            .data_dir()
            .join("storage.ron");
        
        info!("Initializing persistence @ {}", path.display());

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).ok();
        }

        let data = if path.exists() {
            if let Ok(contents) = fs::read_to_string(&path) {
                from_str(&contents).unwrap_or_default()
            } else {
                StorageData::default()
            }
        } else {
            StorageData::default()
        };

        Self { path, data }
    }

    pub fn set<T: Serialize>(&mut self, key: &str, value: &T) {
        if let Ok(serialized) = to_string_pretty(value, PrettyConfig::default()) {
            self.data.map.insert(key.to_string(), serialized);
            self.save();
        }
    }

    pub fn get<T: for<'de> Deserialize<'de>>(&self, key: &str) -> Option<T> {
        self.data.map.get(key).and_then(|ron_str| from_str(ron_str).ok())
    }

    pub fn save(&self) {
        if let Ok(ron_string) = to_string_pretty(&self.data, PrettyConfig::default()) {
            let _ = fs::write(&self.path, ron_string);
        }
    }
}

impl Drop for Persistence {
    fn drop(&mut self) {
        self.save();
    }   
}
