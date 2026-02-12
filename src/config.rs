use log::{error, info};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ─── Persisted config ────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct AppConfig {
    pub process_name: String,
    pub wow_folder_path: String,
    pub selected_character: String,
    pub font_name: String,
    pub font_size: f32,
    pub theme: String,
    pub app_language: String,
    pub deepl_api_key: String,
    pub target_language: String,
    pub auto_translate: bool,
    pub translator_source_lang: String,
    pub translator_target_lang: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            process_name: "Wow.exe".into(),
            wow_folder_path: String::new(),
            selected_character: String::new(),
            font_name: "segoeui".into(),
            font_size: 20.0,
            theme: "Dark".into(),
            app_language: "RU".into(),
            deepl_api_key: String::new(),
            target_language: "RU".into(),
            auto_translate: false,
            translator_source_lang: String::new(),
            translator_target_lang: "EN-US".into(),
        }
    }
}

pub fn config_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

impl AppConfig {
    pub fn load() -> Self {
        let path = config_dir().join("config.toml");
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                info!("Loaded config from {}", path.display());
                toml::from_str(&content).unwrap_or_default()
            }
            Err(_) => {
                info!("No config file found, creating default config");
                let config = Self::default();
                config.save();
                config
            }
        }
    }

    pub fn save(&self) {
        let path = config_dir().join("config.toml");
        match toml::to_string_pretty(self) {
            Ok(content) => {
                if let Err(e) = std::fs::write(&path, content) {
                    error!("Failed to save config: {}", e);
                }
            }
            Err(e) => error!("Failed to serialize config: {}", e),
        }
    }
}

// ─── Font discovery ──────────────────────────────────────────────────

pub struct FontEntry {
    pub name: String,
    pub path: String,
}

pub fn discover_system_fonts() -> Vec<FontEntry> {
    let mut fonts = Vec::new();

    let dirs: &[&str] = if cfg!(windows) {
        &["C:\\Windows\\Fonts"]
    } else {
        &[
            "/usr/share/fonts/truetype",
            "/usr/share/fonts/TTF",
            "/usr/share/fonts/truetype/dejavu",
        ]
    };

    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_ttf = path
                .extension()
                .map_or(false, |e| e.to_string_lossy().eq_ignore_ascii_case("ttf"));
            if is_ttf {
                let name = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                fonts.push(FontEntry {
                    name,
                    path: path.to_string_lossy().into_owned(),
                });
            }
        }
    }

    fonts.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    info!("Discovered {} system fonts", fonts.len());
    fonts
}
